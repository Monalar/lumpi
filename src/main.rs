mod engine;

use clap::{Parser, Subcommand};
use std::time::Instant;
use std::fs;
use std::io::{Read, Write};
use flate2::write::GzEncoder;
use flate2::read::GzDecoder;
use flate2::Compression;

#[derive(Parser)]
#[command(name = "lumpi", version = "5.4.0", about = "Columnar JSONL Log Storage Engine")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Pack { 
        #[arg(short, long)] input: String, 
        #[arg(short, long)] output: String 
    },
    Unpack { 
        #[arg(short, long)] input: String, 
        #[arg(short, long)] output: String 
    },
    Bench {
        #[arg(short, long)] dir: String,
    }
}

fn calculate_entropy(data: &[u8]) -> f64 {
    if data.is_empty() { return 0.0; }
    let mut counts = [0usize; 256];
    for &byte in data { counts[byte as usize] += 1; }
    
    let mut entropy = 0.0;
    let len = data.len() as f64;
    for &count in &counts {
        if count > 0 {
            let p = count as f64 / len;
            entropy -= p * p.log2();
        }
    }
    entropy
}

fn get_entropy_bucket(entropy: f64) -> &'static str {
    if entropy <= 2.0 { "Very Low" }
    else if entropy <= 5.0 { "Low" }
    else if entropy <= 7.0 { "Medium" }
    else { "High (Noise)" }
}

fn median(mut times: Vec<f64>) -> f64 {
    if times.is_empty() { return 0.0; }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times[times.len() / 2]
}

fn run_zstd_benchmark_pack(content: &[u8], level: i32, iterations: usize) -> (f64, f64, Vec<u8>) {
    let final_compressed = zstd::encode_all(content, level).unwrap();
    let final_size = final_compressed.len() as f64;
    let mut times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        let _ = zstd::encode_all(content, level).unwrap();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    (final_size, median(times), final_compressed)
}

fn run_gzip_benchmark_pack(content: &[u8], iterations: usize) -> (f64, f64, Vec<u8>) {
    let mut final_compressed = Vec::new();
    {
        let mut gz_enc = GzEncoder::new(Vec::new(), Compression::default());
        gz_enc.write_all(content).unwrap();
        final_compressed = gz_enc.finish().unwrap();
    }
    let final_size = final_compressed.len() as f64;
    let mut times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        let mut gz_enc = GzEncoder::new(Vec::new(), Compression::default());
        gz_enc.write_all(content).unwrap();
        let _ = gz_enc.finish().unwrap();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    (final_size, median(times), final_compressed)
}

fn run_zstd_benchmark_unpack(compressed: &[u8], iterations: usize) -> f64 {
    let mut times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        let _ = zstd::decode_all(compressed).unwrap();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    median(times)
}

fn run_gzip_benchmark_unpack(compressed: &[u8], iterations: usize) -> f64 {
    let mut times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        let mut gz_dec = GzDecoder::new(compressed);
        let mut _dec = Vec::new();
        let _ = gz_dec.read_to_end(&mut _dec);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    median(times)
}

fn calculate_weissman_score(r_target: f64, r_base: f64, t_target_ms: f64, t_base_ms: f64) -> f64 {
    if r_base <= 0.0 || t_target_ms <= 0.0 || t_base_ms <= 0.0 { return 0.0; }
    (r_target / r_base) * ((t_base_ms + 1.0).log10() / (t_target_ms + 1.0).log10())
}

fn calc_throughput(mb: f64, ms: f64) -> f64 {
    if ms == 0.0 { return 0.0; }
    mb / (ms / 1000.0)
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Bench { dir } => {
            println!("\nLUMPRESS 5.4 | ENTROPY SPECTRUM BENCHMARK (MEDIAN & STEADY-STATE)");
            println!("========================================================================================================================================");
            println!("{:<20} | {:<8} | {:<7} | {:<13} | {:<11} | {:<11} | {:<8} | {:<12} | {:<12}", 
                     "Dataset", "Size(MB)", "Entropy", "Bucket", "LUMPI Ratio", "Zstd L3", "Weissman", "Lumpi (med)", "Zstd L3 (med)");
            println!("----------------------------------------------------------------------------------------------------------------------------------------");

            let paths = fs::read_dir(dir).expect("Directory not found");
            let mut files: Vec<_> = paths.filter_map(Result::ok).collect();
            files.sort_by_key(|dir| dir.path());

            for path_info in files {
                let path = path_info.path();
                if path.is_file() {
                    let file_name = path.file_name().unwrap().to_string_lossy().to_string();
                    let mut file_content = Vec::new();
                    fs::File::open(&path).unwrap().read_to_end(&mut file_content).unwrap();
                    let orig_size = file_content.len() as f64;
                    if orig_size == 0.0 { continue; }
                    let orig_mb = orig_size / 1048576.0;
                    
                    let entropy = calculate_entropy(&file_content);
                    let bucket = get_entropy_bucket(entropy);

                    let iterations = 5;
                    
                    let (z3_size, z3_med_time, _) = run_zstd_benchmark_pack(&file_content, 3, iterations);
                    
                    let mut lumpi = engine::LumpiEngine::new();
                    let (comp, _) = lumpi.compress_buffer(&file_content).unwrap();
                    let lumpi_size = comp.len() as f64;

                    let mut lumpi_times = Vec::with_capacity(iterations);
                    for _ in 0..iterations {
                        lumpi.clear();
                        let start = Instant::now();
                        let _ = lumpi.compress_buffer(&file_content).unwrap();
                        lumpi_times.push(start.elapsed().as_secs_f64() * 1000.0);
                    }
                    let lumpi_med_time = median(lumpi_times);

                    let lumpi_ratio = orig_size / lumpi_size;
                    let z3_ratio = orig_size / z3_size;
                    let w_score = calculate_weissman_score(lumpi_ratio, z3_ratio, lumpi_med_time, z3_med_time);

                    let display_name = if file_name.len() > 20 { format!("{}..", &file_name[..18]) } else { file_name };

                    println!("{:<20} | {:>8.2} | {:>7.2} | {:<13} | {:>10.2}x | {:>10.2}x | {:>8.3} | {:>9.2} ms | {:>9.2} ms", 
                        display_name, orig_mb, entropy, bucket, lumpi_ratio, z3_ratio, w_score, lumpi_med_time, z3_med_time);
                }
            }
            println!("========================================================================================================================================\n");
        }
        Commands::Pack { input, output } => {
            println!("[INFO] LUMPRESS 5.4 | Initializing In-Memory Engine...");
            let mut file_content = Vec::new();
            fs::File::open(input).unwrap().read_to_end(&mut file_content).unwrap();
            
            let original_size_bytes = file_content.len() as f64;
            let original_mb = original_size_bytes / 1048576.0;
            if original_size_bytes == 0.0 { panic!("File is empty!"); }

            println!("[INFO] Warming up encoding engines...");
            let iterations = 5;

            let (gz_size, t_gz_pack, _gz_bytes) = run_gzip_benchmark_pack(&file_content, iterations);
            let gz_ratio = original_size_bytes / gz_size;

            let (z1_size, z1_time_pack, _) = run_zstd_benchmark_pack(&file_content, 1, iterations);
            let (z3_size, z3_time_pack, z3_bytes) = run_zstd_benchmark_pack(&file_content, 3, iterations);
            let (z19_size, z19_time_pack, _) = run_zstd_benchmark_pack(&file_content, 19, 1);

            let mut lumpi = engine::LumpiEngine::new();
            let (comp, _final_hash) = lumpi.compress_buffer(&file_content).expect("Fatal: Compression failed");
            let lumpi_final_size = comp.len() as f64;
            let lumpi_bytes = comp;

            let mut lumpi_times = Vec::with_capacity(iterations);
            for _ in 0..iterations {
                lumpi.clear();
                let start = Instant::now();
                let _ = lumpi.compress_buffer(&file_content).unwrap();
                lumpi_times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            let min_lumpi_pack = median(lumpi_times);
            let lumpi_ratio = original_size_bytes / lumpi_final_size;

            let mut out_file = fs::File::create(output).unwrap();
            out_file.write_all(&lumpi_bytes).unwrap();

            println!("[INFO] Warming up decoding engines...");
            let t_gz_unpack = run_gzip_benchmark_unpack(&_gz_bytes, iterations);
            let t_z3_unpack = run_zstd_benchmark_unpack(&z3_bytes, iterations);

            let mut unpack_times = Vec::with_capacity(iterations);
            for _ in 0..iterations {
                let start = Instant::now();
                let _ = engine::LumpiEngine::decompress_buffer(&lumpi_bytes).expect("Decompression failed");
                unpack_times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            let min_lumpi_unpack = median(unpack_times);

            let z3_ratio = original_size_bytes / z3_size;
            let w_score = calculate_weissman_score(lumpi_ratio, z3_ratio, min_lumpi_pack, z3_time_pack);

            println!("\n=======================================================================================");
            println!("  APPLES-TO-APPLES ANALYSIS (In-Memory, Median of {} runs, {:.2} MB)", iterations, original_mb);
            println!("=======================================================================================");
            println!("{:<18} | {:<12} | {:<8} | {:<10} | {:<10} | {:<10}", "Algorithm", "Size (KB)", "Ratio", "Pack MB/s", "Pack ms", "Unpack ms");
            println!("---------------------------------------------------------------------------------------");
            println!("{:<18} | {:>10.2} | {:>7.2}x | {:>10.2} | {:>10.2} | {:>10.2}", "GZIP (v6)", gz_size / 1024.0, gz_ratio, calc_throughput(original_mb, t_gz_pack), t_gz_pack, t_gz_unpack);
            println!("{:<18} | {:>10.2} | {:>7.2}x | {:>10.2} | {:>10.2} | {:>10}", "Zstd (L1)", z1_size / 1024.0, original_size_bytes / z1_size, calc_throughput(original_mb, z1_time_pack), z1_time_pack, "-");
            println!("{:<18} | {:>10.2} | {:>7.2}x | {:>10.2} | {:>10.2} | {:>10.2}", "Zstd (L3) [BASE]", z3_size / 1024.0, z3_ratio, calc_throughput(original_mb, z3_time_pack), z3_time_pack, t_z3_unpack);
            println!("{:<18} | {:>10.2} | {:>7.2}x | {:>10.2} | {:>10.2} | {:>10}", "Zstd (L19)", z19_size / 1024.0, original_size_bytes / z19_size, calc_throughput(original_mb, z19_time_pack), z19_time_pack, "-");
            println!("---------------------------------------------------------------------------------------");
            println!("{:<18} | {:>10.2} | {:>7.2}x | {:>10.2} | {:>10.2} | {:>10.2}", "LUMPRESS CORE", lumpi_final_size / 1024.0, lumpi_ratio, calc_throughput(original_mb, min_lumpi_pack), min_lumpi_pack, min_lumpi_unpack);
            println!("=======================================================================================");
            println!("WEISSMAN SCORE (vs Zstd L3): {:.3}", w_score);
            println!("=======================================================================================\n");
        }
        Commands::Unpack { input, output } => {
            println!("[INFO] Initializing decompression to disk...");
            let start = Instant::now();
            match engine::LumpiEngine::decompress(input, output) {
                Ok(true) => {
                    let t = start.elapsed().as_secs_f64() * 1000.0;
                    println!("[SUCCESS] Data integrity verified. Restore time: {:.2}ms", t);
                },
                Ok(false) => println!("[FATAL] Integrity check failed: SHA-256 mismatch."),
                Err(e) => eprintln!("[ERROR] IO Failure: {}", e),
            }
        }
    }
}