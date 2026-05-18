mod engine;

use clap::{Parser, Subcommand};
use std::time::Instant;
use std::fs;
use std::io::{Read, Write};
use flate2::write::GzEncoder;
use flate2::Compression;

#[derive(Parser)]
#[command(name = "lumpi", version, about = "Columnar compression for flat JSONL and CSV logs")]
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
    Grep {
        input: String,
        pattern: String,
    },
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {}", msg);
    std::process::exit(1);
}

fn read_file(path: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    fs::File::open(path)
        .unwrap_or_else(|e| die(format!("{}: {}", path, e)))
        .read_to_end(&mut buf)
        .unwrap_or_else(|e| die(format!("read {}: {}", path, e)));
    buf
}

fn write_file(path: &str, data: &[u8]) {
    fs::File::create(path)
        .unwrap_or_else(|e| die(format!("{}: {}", path, e)))
        .write_all(data)
        .unwrap_or_else(|e| die(format!("write {}: {}", path, e)));
}

fn zstd_thread_count() -> u32 {
    std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4)
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

fn median(mut times: Vec<f64>) -> f64 {
    if times.is_empty() { return 0.0; }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times[times.len() / 2]
}

fn run_zstd_pack(content: &[u8], level: i32, iterations: usize) -> (f64, f64, Vec<u8>) {
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

fn run_zstd_mt_pack(content: &[u8], level: i32, threads: u32, iterations: usize) -> (f64, f64) {
    let compress_once = |buf: &mut Vec<u8>| {
        buf.clear();
        let mut enc = zstd::stream::Encoder::new(buf, level).unwrap();
        enc.multithread(threads).unwrap();
        enc.write_all(content).unwrap();
        enc.finish().unwrap();
    };
    let mut buf = Vec::new();
    compress_once(&mut buf);
    let final_size = buf.len() as f64;
    let mut times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        compress_once(&mut buf);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    (final_size, median(times))
}

fn run_zstd_long_pack(content: &[u8], level: i32, iterations: usize) -> (f64, f64) {
    let compress_once = |buf: &mut Vec<u8>| {
        buf.clear();
        let mut enc = zstd::stream::Encoder::new(buf, level).unwrap();
        enc.long_distance_matching(true).unwrap();
        enc.window_log(27).unwrap();
        enc.write_all(content).unwrap();
        enc.finish().unwrap();
    };
    let mut buf = Vec::new();
    compress_once(&mut buf);
    let final_size = buf.len() as f64;
    let mut times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        compress_once(&mut buf);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    (final_size, median(times))
}

fn run_brotli_pack(content: &[u8], level: u32, iterations: usize) -> (f64, f64) {
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

fn run_gzip_pack(content: &[u8], iterations: usize) -> (f64, f64) {
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

fn calc_throughput(mb: f64, ms: f64) -> f64 {
    if ms == 0.0 { return 0.0; }
    mb / (ms / 1000.0)
}

fn main() {
    use std::io::IsTerminal;
    if std::io::stderr().is_terminal() {
        eprintln!("mnlr \u{22c5} lumpi");
    }
    let cli = Cli::parse();
    match &cli.command {
        Commands::Pack { input, output } => {
            let output = output.clone().unwrap_or_else(|| format!("{}.lmp", input));
            let file_content = read_file(input);
            let original_size = file_content.len() as f64;
            let mb = original_size / 1_048_576.0;

            let start = Instant::now();
            let mut lumpi = engine::LumpiEngine::new();
            let (l_bytes, _) = lumpi.compress_buffer(&file_content)
                .unwrap_or_else(|e| die(format!("compress failed: {}", e)));
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            let compressed_size = l_bytes.len() as f64;

            write_file(&output, &l_bytes);
            println!("Ratio: {:.2}x | {:.0} MB/s | {}",
                original_size / compressed_size,
                calc_throughput(mb, elapsed),
                output);
        }
        Commands::Unpack { input, output } => {
            let output = output.clone().unwrap_or_else(|| {
                if input.ends_with(".lmp") {
                    input[..input.len() - 4].to_string()
                } else {
                    format!("{}.out", input)
                }
            });
            let start = Instant::now();
            let data = read_file(input);
            let decompressed = engine::LumpiEngine::decompress_buffer(&data)
                .unwrap_or_else(|e| die(format!("decompress failed: {}", e)));
            write_file(&output, &decompressed);
            println!("Time: {:.2}ms | {}", start.elapsed().as_secs_f64() * 1000.0, output);
        }
        Commands::Research { input } => {
            let file_content = read_file(input);
            let original_size = file_content.len() as f64;
            let mb = original_size / 1_048_576.0;
            let n = 3;
            let threads = zstd_thread_count();

            let detected = engine::LumpiEngine::detect_format(&file_content);

            println!("\n[RUNNING COMPRESSION SPECTRUM ANALYSIS — honest baselines]");
            let (g_s, g_t)           = run_gzip_pack(&file_content, n);
            let (z3_s, z3_t, _)      = run_zstd_pack(&file_content, 3, n);
            let (z9_s, z9_t, _)      = run_zstd_pack(&file_content, 9, n);
            let (z9mt_s, z9mt_t)     = run_zstd_mt_pack(&file_content, 9, threads, n);
            let (z19_s, z19_t, _)    = run_zstd_pack(&file_content, 19, 1);
            println!("[Zstd L19+LDM] computing (slow)...");
            let (zldm_s, zldm_t)     = run_zstd_long_pack(&file_content, 19, 1);
            println!("[Brotli] computing (slow)...");
            let (b3_s, b3_t)         = run_brotli_pack(&file_content, 3, n);
            let (b11_s, b11_t)       = run_brotli_pack(&file_content, 11, 1);

            let mut lumpi = engine::LumpiEngine::new();
            let (l_bytes, _) = lumpi.compress_buffer(&file_content)
                .unwrap_or_else(|e| die(format!("compress failed: {}", e)));
            let format_label = if lumpi.was_structured() { detected.label() } else { "Raw" };
            let mut l_times = Vec::new();
            for _ in 0..n {
                lumpi.clear();
                let start = Instant::now();
                let _ = lumpi.compress_buffer(&file_content)
                    .unwrap_or_else(|e| die(format!("compress failed: {}", e)));
                l_times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            let l_t = median(l_times);
            let l_s = l_bytes.len() as f64;

            let w = 95;
            println!("\n{}", "=".repeat(w));
            println!("  SPECTRUM ANALYSIS  {:.2} MB  format={}", mb, format_label);
            println!("{}", "=".repeat(w));
            println!("{:<30} | {:>10} | {:>7} | {:>10} | {:>10}",
                "Algorithm", "Size (KB)", "Ratio", "Pack ms", "MB/s");
            println!("{}", "-".repeat(w));
            let row = |name: &str, size: f64, time: f64| {
                println!("{:<30} | {:>10.2} | {:>6.2}x | {:>10.2} | {:>10.1}",
                    name, size / 1024.0, original_size / size, time, calc_throughput(mb, time));
            };
            row("GZIP",                                  g_s,    g_t);
            row("Zstd L3 (1 thread)",                   z3_s,   z3_t);
            row("Zstd L9 (1 thread)",                   z9_s,   z9_t);
            row(&format!("Zstd L9 MT ({} threads)", threads),  z9mt_s, z9mt_t);
            row("Zstd L19 (1 thread)",                  z19_s,  z19_t);
            row("Zstd L19+LDM (1 thread)",              zldm_s, zldm_t);
            row("Brotli L3",                            b3_s,   b3_t);
            row("Brotli L11",                           b11_s,  b11_t);
            println!("{}", "-".repeat(w));
            row(&format!("LUMPI columnar+Zstd L9 MT ({} threads)", threads), l_s, l_t);
            println!("{}", "=".repeat(w));
            println!("  LUMPI vs Zstd L9 MT:    ratio {:.2}x better, throughput {:.1}x delta",
                (original_size / l_s) / (original_size / z9mt_s),
                calc_throughput(mb, l_t) / calc_throughput(mb, z9mt_t));
            println!("  LUMPI vs Zstd L19+LDM:  ratio {:.2}x delta",
                (original_size / l_s) / (original_size / zldm_s));
            println!("{}", "=".repeat(w));
        }
        Commands::Bench { dir } => {
            let threads = zstd_thread_count();
            let w = 120;
            let sep = "=".repeat(w);
            let dash = "-".repeat(w);
            println!("\nLUMPI BENCHMARK  (skips unstructured files — use plain zstd for those)");
            println!("Zstd baselines: L9 MT uses {} threads (same as lumpi internally); L19+LDM is single-threaded max compression.", threads);
            println!("{}", sep);
            println!("{:<20} | {:<6} | {:>7} | {:>12} | {:>14} | {:>14} | {:>11} | {:>11}",
                "Dataset", "Format", "Size MB",
                "Lumpi ratio",
                &format!("Zstd L9 MT ({}t)", threads),
                "Zstd L19+LDM",
                "Lumpi MB/s",
                &format!("L9 MT MB/s"));
            println!("{}", dash);

            let paths = fs::read_dir(dir).unwrap_or_else(|e| die(format!("{}: {}", dir, e)));
            let mut files: Vec<_> = paths.filter_map(Result::ok).collect();
            files.sort_by_key(|d| d.path());

            for path_info in files {
                let path = path_info.path();
                if !path.is_file() { continue; }
                let file_name = path.file_name().unwrap().to_string_lossy().to_string();
                let mut content = Vec::new();
                if fs::File::open(&path)
                    .and_then(|mut f| f.read_to_end(&mut content))
                    .is_err() { continue; }
                let orig = content.len() as f64;
                if orig == 0.0 { continue; }
                let mb = orig / 1_048_576.0;

                let mut lumpi = engine::LumpiEngine::new();
                let (comp, _) = match lumpi.compress_buffer(&content) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if !lumpi.was_structured() { continue; }

                let format_label = engine::LumpiEngine::detect_format(&content).label();
                let lumpi_size = comp.len() as f64;

                let mut lumpi_times = Vec::with_capacity(3);
                for _ in 0..3 {
                    lumpi.clear();
                    let t = Instant::now();
                    let _ = lumpi.compress_buffer(&content).unwrap_or_else(|e| die(format!("compress: {}", e)));
                    lumpi_times.push(t.elapsed().as_secs_f64() * 1000.0);
                }
                let lumpi_ms = median(lumpi_times);

                let (z9mt_size, z9mt_ms) = run_zstd_mt_pack(&content, 9, threads, 1);
                let (zldm_size, _)       = run_zstd_long_pack(&content, 19, 1);

                let name = if file_name.len() > 20 { format!("{}..", &file_name[..18]) } else { file_name };

                println!("{:<20} | {:<6} | {:>7.2} | {:>11.2}x | {:>13.2}x | {:>13.2}x | {:>10.1} | {:>10.1}",
                    name, format_label, mb,
                    orig / lumpi_size,
                    orig / z9mt_size,
                    orig / zldm_size,
                    calc_throughput(mb, lumpi_ms),
                    calc_throughput(mb, z9mt_ms));

                let _ = calculate_entropy(&content);
            }
            println!("{}", sep);
        }
        Commands::Grep { input, pattern } => {
            let sep = match pattern.find('=') {
                Some(p) => p,
                None => {
                    eprintln!("error: pattern must be KEY=VALUE");
                    std::process::exit(1);
                }
            };
            let key = pattern[..sep].as_bytes();
            let value = pattern[sep + 1..].as_bytes();
            let data = read_file(input);
            let start = Instant::now();
            let matches = engine::LumpiEngine::grep_buffer(&data, key, value)
                .unwrap_or_else(|e| die(format!("grep failed: {}", e)));
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            for m in &matches {
                out.write_all(m).unwrap_or_else(|e| die(format!("write stdout: {}", e)));
            }
            eprintln!("{} matches in {:.2}ms", matches.len(), elapsed);
        }
    }
}
