//! Benchmark commands

use std::time::Instant;
use anyhow::{Context, Result};
use crate::{Cli, BenchmarkArgs, BenchmarkCommands};

pub async fn run(_cli: &Cli, args: &BenchmarkArgs) -> Result<()> {
    match &args.command {
        BenchmarkCommands::Cpu => benchmark_cpu().await,
        BenchmarkCommands::Compression { file } => benchmark_compression(file).await,
        BenchmarkCommands::Chunking { file } => benchmark_chunking(file).await,
    }
}

async fn benchmark_cpu() -> Result<()> {
    println!("Running CPU benchmark...");
    
    // Benchmark hashing
    let data = vec![0u8; 64 * 1024 * 1024]; // 64MB
    
    let start = Instant::now();
    for _ in 0..10 {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let _ = hasher.finalize();
    }
    let sha256_time = start.elapsed();
    
    let start = Instant::now();
    for _ in 0..10 {
        use blake2::{Blake2b512, Digest};
        let mut hasher = Blake2b512::new();
        hasher.update(&data);
        let _ = hasher.finalize();
    }
    let blake2_time = start.elapsed();
    
    println!("SHA-256: {:.2} MB/s", 640.0 / sha256_time.as_secs_f64());
    println!("BLAKE2b: {:.2} MB/s", 640.0 / blake2_time.as_secs_f64());
    
    Ok(())
}

async fn benchmark_compression(file: &std::path::Path) -> Result<()> {
    println!("Benchmarking compression on {:?}...", file);
    
    let data = tokio::fs::read(file).await
        .context("Failed to read file")?;
    
    println!("File size: {} bytes", data.len());
    
    // Benchmark zstd at different levels
    for level in [1, 3, 6, 9] {
        let start = Instant::now();
        let compressed = zstd::bulk::compress(&data[..], level as i32)?;
        let elapsed = start.elapsed();
        
        let ratio = compressed.len() as f64 / data.len() as f64;
        let speed = data.len() as f64 / elapsed.as_secs_f64() / 1024.0 / 1024.0;
        
        println!("zstd level {}: ratio={:.2}%, speed={:.1} MB/s", 
            level, ratio * 100.0, speed);
    }
    
    Ok(())
}

async fn benchmark_chunking(file: &std::path::Path) -> Result<()> {
    println!("Benchmarking chunking on {:?}...", file);
    
    let data = tokio::fs::read(file).await
        .context("Failed to read file")?;
    
    println!("File size: {} bytes", data.len());
    
    // Benchmark different chunking algorithms
    use borg_core::chunker::{Chunker, ChunkerConfig};
    
    for (name, config) in [
        ("Default (1MB avg)", ChunkerConfig::default()),
        ("Small files (64KB avg)", ChunkerConfig::small_files()),
        ("Large files (4MB avg)", ChunkerConfig::large_files()),
    ] {
        let chunker = Chunker::new(config).expect("failed to create chunker");
        let start = Instant::now();
        let chunks = chunker.chunk_data(&data);
        let elapsed = start.elapsed();
        
        let speed = data.len() as f64 / elapsed.as_secs_f64() / 1024.0 / 1024.0;
        let avg_size = if chunks.is_empty() { 0 } else { data.len() / chunks.len() };
        
        println!("{}: {} chunks, avg size={} bytes, speed={:.1} MB/s", 
            name, chunks.len(), avg_size, speed);
    }
    
    Ok(())
}
