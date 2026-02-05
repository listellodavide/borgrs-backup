//! Archive creation command

use std::time::Instant;

use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use tracing::info;

use borg_core::{
    archive::{ArchiveCreator, BackupProgress},
    compression::{CompressionAlgorithm, CompressionConfig, CompressionLevel, Compressor},
    exclusion::{ExclusionList, ExclusionPattern},
};
use std::path::Path;

use super::{format_duration, format_size, get_repo_path, open_repository};
use crate::{Cli, CreateArgs};

pub async fn run(cli: &Cli, args: &CreateArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let start_time = Instant::now();

    info!("Creating archive '{}' in repository {}", args.archive, repo_path);

    // Validate paths exist
    for path in &args.paths {
        if !path.exists() {
            anyhow::bail!("Path does not exist: {:?}", path);
        }
    }

    // Build exclusion matcher
    let exclusion_matcher = build_exclusion_matcher(&args)?;

    // Configure compression
    let _compressor = build_compressor(&args)?;

    // Open repository
    let mut repo = open_repository(&repo_path).await
        .context("Failed to open repository")?;

    // Create progress bar if requested
    let progress = if cli.progress {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} [{elapsed_precise}] {msg}")
                .unwrap()
        );
        pb.set_message("Scanning files...");
        Some(pb)
    } else {
        None
    };

    // Build archive creator
    let mut creator = ArchiveCreator::new(&mut repo)
        .with_exclusions(exclusion_matcher);

    // Set progress handler if enabled
    if let Some(ref pb) = progress {
        let cli_progress = CliProgress {
            pb: pb.clone(),
            list_files: args.list,
        };
        creator = creator.with_progress(Box::new(cli_progress));
    }

    // Create the archive
    if args.dry_run {
        println!("Dry run - no archive created");
        return Ok(());
    }

    let archive = creator.create(&args.archive, &args.paths, args.comment.clone()).await
        .map_err(|e| anyhow::anyhow!("Failed to create archive: {}", e))?;

    // Finish progress bar
    if let Some(pb) = progress {
        pb.finish_with_message("Done");
    }

    let duration = start_time.elapsed();

    println!("Archive: {}", args.archive);
    println!("Status: Success");
    println!("Files: {}", archive.stats.nfiles);
    println!("Directories: {}", archive.stats.ndirs);
    println!("Original size: {}", format_size(archive.stats.original_size));
    println!("Compressed size: {}", format_size(archive.stats.compressed_size));
    println!("Deduplicated size: {}", format_size(archive.stats.deduplicated_size));
    println!("Duration: {}", format_duration(duration.as_secs_f64()));
    println!("Average speed: {}/s", format_size((archive.stats.original_size as f64 / duration.as_secs_f64()) as u64));

    // Print statistics if requested
    if args.stats {
        println!();
        println!("Archive: {}", args.archive);
        println!("Duration: {}", format_duration(duration.as_secs_f64()));
        println!();
        println!("                       Original size      Compressed size    Deduplicated size");
        println!("This archive:          {:>15}    {:>15}    {:>15}",
            format_size(archive.stats.original_size),
            format_size(archive.stats.compressed_size),
            format_size(archive.stats.deduplicated_size)
        );
        println!();
        println!("Number of files: {}", archive.stats.nfiles);
    }

    Ok(())
}

fn build_exclusion_matcher(args: &CreateArgs) -> Result<ExclusionList> {
    let mut patterns = Vec::new();

    // Add exclude patterns
    for pattern in &args.exclude {
        patterns.push(ExclusionPattern::glob(pattern));
    }

    let mut list = ExclusionList::from_patterns(patterns)?;

    // Load patterns from files
    for file in &args.exclude_from {
        if file.exists() {
            let file_list = ExclusionList::from_file(file)?;
            for pattern in file_list.patterns() {
                list.add_pattern(pattern.clone())?;
            }
        } else {
            anyhow::bail!("Exclude file not found: {:?}", file);
        }
    }

    Ok(list)
}

struct CliProgress {
    pb: ProgressBar,
    list_files: bool,
}

impl BackupProgress for CliProgress {
    fn on_file_start(&self, path: &Path) {
        if self.list_files {
            println!("{}", path.display());
        }
        self.pb.set_message(format!("Processing: {}", path.display()));
    }

    fn on_file_complete(&self, _path: &Path, _size: u64, _chunks: usize) {
        self.pb.inc(1);
    }

    fn on_file_skipped(&self, _path: &Path, _reason: &str) {}

    fn on_progress(&self, _processed: u64, _total: u64) {}

    fn on_error(&self, _path: &Path, _error: &str) {}
}

fn build_compressor(args: &CreateArgs) -> Result<Compressor> {
    let algo_str = &args.compression;
    let level_num = args.compression_level.unwrap_or(3) as u8;

    let algorithm = CompressionAlgorithm::from_str(algo_str)
        .map_err(|e| anyhow::anyhow!("Invalid compression algorithm: {}", e))?;
    let level = CompressionLevel::new(level_num)
        .map_err(|e| anyhow::anyhow!("Invalid compression level: {}", e))?;

    let config = CompressionConfig {
        algorithm,
        level,
        ..Default::default()
    };

    Ok(Compressor::new(config))
}
