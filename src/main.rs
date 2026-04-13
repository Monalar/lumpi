mod engine;

use clap::{Parser, Subcommand};
use std::time::Instant;
use std::fs;
use std::io::{Read, Write};
use flate2::write::GzEncoder;
use flate2::read::GzDecoder;
use flate2::Compression;

#[derive(Parser)]
#[command(name = "lumpi", version = "6.0.0", about = "Columnar Log/CSV/JSON Storage Engine")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Pack {
        input: String,
        output: Option<String>,
    },
    Unpack {
        input: String,
        output: Option<String>,
    },
    Research {
        input: String,
    },
    Bench {
        dir: String,
    },
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

fn run_brotli_benchmark_pack(content: &[u8], level: u32, iterations: usize) -> (f64, f64) {
    let mut final_size = 0.0;
    let mut times = Vec::with_capacity(iterations);
    for i in 0..iterations {
        let start = Instant::now();
        let mut compressed = Vec::new();
        let mut writer = brotli::CompressorWriter::new(&mut compressed, 4096, level, 22);
        writer.write_all(content).unwrap();
        writer.flush().unwrap();
        drop(writer);
        if i == 0 { final_size = compressed.len() as f64; }
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    (final_size, median(times))
}

fn run_gzip_benchmark_pack(content: &[u8], iterations: usize) -> (f64, f64) {
    let mut final_size = 0.0;
    let mut times = Vec::with_capacity(iterations);
    for i in 0..iterations {
        let start = Instant::now();
        let mut gz_enc = GzEncoder::new(Vec::new(), Compression::default());
        gz_enc.write_all(content).unwrap();
        let compressed = gz_enc.finish().unwrap();
        if i == 0 { final_size = compressed.len() as f64; }
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    (final_size, median(times))
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
        Commands::Pack { input, output } => {
            let output = output.clone().unwrap_or_else(|| format!("{}.lumpi", input));
            let mut file_content = Vec::new();
            fs::File::open(input).unwrap().read_to_end(&mut file_content).unwrap();
            let original_size = file_content.len() as f64;

            let start = Instant::now();
            let mut lumpi = engine::LumpiEngine::new();
            let (l_bytes, _) = lumpi.compress_buffer(&file_content).unwrap();
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            let compressed_size = l_bytes.len() as f64;

            fs::File::create(&output).unwrap().write_all(&l_bytes).unwrap();
            println!("Ratio: {:.2}x | Time: {:.2}ms | {}", original_size / compressed_size, elapsed, output);
        }
        Commands::Unpack { input, output } => {
            let output = output.clone().unwrap_or_else(|| {
                if input.ends_with(".lumpi") {
                    input[..input.len() - 6].to_string()
                } else {
                    format!("{}.out", input)
                }
            });
            let start = Instant::now();
            let mut data = Vec::new();
            fs::File::open(input).unwrap().read_to_end(&mut data).unwrap();
            let decompressed = engine::LumpiEngine::decompress_buffer(&data).unwrap();
            fs::File::create(&output).unwrap().write_all(&decompressed).unwrap();
            println!("Time: {:.2}ms | {}", start.elapsed().as_secs_f64() * 1000.0, output);
        }
        Commands::Research { input } => {
            let mut file_content = Vec::new();
            fs::File::open(input).unwrap().read_to_end(&mut file_content).unwrap();
            let original_size = file_content.len() as f64;
            let mb = original_size / 1048576.0;
            let iterations = 3;

            let detected = engine::LumpiEngine::detect_format(&file_content);

            println!("\n[RUNNING COMPRESSION SPECTRUM ANALYSIS]");
            let (g_s, g_t) = run_gzip_benchmark_pack(&file_content, iterations);
            let (z3_s, z3_t, _) = run_zstd_benchmark_pack(&file_content, 3, iterations);
            let (z6_s, z6_t, _) = run_zstd_benchmark_pack(&file_content, 6, iterations);
            let (z9_s, z9_t, _) = run_zstd_benchmark_pack(&file_content, 9, iterations);
            let (z15_s, z15_t, _) = run_zstd_benchmark_pack(&file_content, 15, 1);
            let (z19_s, z19_t, _) = run_zstd_benchmark_pack(&file_content, 19, 1);

            println!("[BROTLI] This will be slow...");
            let (b3_s, b3_t) = run_brotli_benchmark_pack(&file_content, 3, iterations);
            let (b11_s, b11_t) = run_brotli_benchmark_pack(&file_content, 11, 1);

            let mut lumpi = engine::LumpiEngine::new();
            let (l_bytes, _) = lumpi.compress_buffer(&file_content).unwrap();
            let format_label = if lumpi.was_structured() { detected.label() } else { "Raw" };
            let mut l_times = Vec::new();
            for _ in 0..iterations {
                lumpi.clear();
                let start = Instant::now();
                let _ = lumpi.compress_buffer(&file_content).unwrap();
                l_times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            let l_t = median(l_times);
            let l_s = l_bytes.len() as f64;

            println!("\n=======================================================================================");
            println!("  ULTIMATE ANALYSIS ({:.2} MB) - Format: {}", mb, format_label);
            println!("=======================================================================================");
            println!("{:<18} | {:<12} | {:<8} | {:<10} | {:<10}", "Algorithm", "Size (KB)", "Ratio", "Pack ms", "MB/s");
            println!("---------------------------------------------------------------------------------------");
            let print_row = |name: &str, size: f64, time: f64| {
                println!("{:<18} | {:>10.2} | {:>7.2}x | {:>10.2} | {:>10.2}", name, size / 1024.0, original_size / size, time, calc_throughput(mb, time));
            };
            print_row("GZIP (v6)", g_s, g_t);
            print_row("Zstd (L3)", z3_s, z3_t);
            print_row("Zstd (L6)", z6_s, z6_t);
            print_row("Zstd (L9)", z9_s, z9_t);
            print_row("Zstd (L15)", z15_s, z15_t);
            print_row("Zstd (L19)", z19_s, z19_t);
            print_row("Brotli (L3)", b3_s, b3_t);
            print_row("Brotli (L11)", b11_s, b11_t);
            println!("---------------------------------------------------------------------------------------");
            print_row("LUMPRESS (L9)", l_s, l_t);
            println!("=======================================================================================");
            println!("WEISSMAN SCORE (vs Zstd L3): {:.3}", calculate_weissman_score(original_size/l_s, original_size/z3_s, l_t, z3_t));
        }
        Commands::Bench { dir } => {
            println!("\nLUMPRESS 6.0 | SPECTRUM BENCHMARK");
            println!("================================================================================================================================================");
            println!("{:<20} | {:<6} | {:<8} | {:<7} | {:<13} | {:<11} | {:<11} | {:<8} | {:<12} | {:<12}",
                     "Dataset", "Format", "Size(MB)", "Entropy", "Bucket", "LUMPI Ratio", "Zstd L3", "Weissman", "Lumpi (med)", "Zstd L3 (med)");
            println!("------------------------------------------------------------------------------------------------------------------------------------------------");
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
                    let detected = engine::LumpiEngine::detect_format(&file_content);
                    let (z3_size, z3_med_time, _) = run_zstd_benchmark_pack(&file_content, 3, 3);
                    let mut lumpi = engine::LumpiEngine::new();
                    let (comp, _) = lumpi.compress_buffer(&file_content).unwrap();
                    let format_label = if lumpi.was_structured() { detected.label() } else { "Raw" };
                    let lumpi_size = comp.len() as f64;
                    let mut lumpi_times = Vec::with_capacity(3);
                    for _ in 0..3 {
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
                    println!("{:<20} | {:<6} | {:>8.2} | {:>7.2} | {:<13} | {:>10.2}x | {:>10.2}x | {:>8.3} | {:>9.2} ms | {:>9.2} ms",
                        display_name, format_label, orig_mb, calculate_entropy(&file_content), get_entropy_bucket(calculate_entropy(&file_content)), lumpi_ratio, z3_ratio, w_score, lumpi_med_time, z3_med_time);
                }
            }
        }
    }
}
