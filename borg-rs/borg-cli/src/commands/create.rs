//! Archive creation command

use std::time::Instant;

use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use tracing::{info, debug};

use borg_core::{
    archive::{default_archive_unique_name, ArchiveCreator, BackupProgress},
    compression::{CompressionAlgorithm, CompressionConfig, CompressionLevel, Compressor},
    exclusion::{ExclusionList, ExclusionPattern},
};
use std::path::Path;

use super::{format_duration, format_size, get_repo_path, open_repository};
use crate::{Cli, CreateArgs};

pub async fn run(cli: &Cli, args: &CreateArgs) -> Result<()> {
    let repo_path_raw = get_repo_path(cli)?;

    // Normalize WebDAV URL if credentials are provided (either from CLI or if it's a remote repo)
    let repo_path = if args.webdav_user.is_some() || args.webdav_pass.is_some() {
        let scheme = if cli.remote_repo.is_some() {
            if repo_path_raw.starts_with("https") || repo_path_raw.starts_with("webdavs") || repo_path_raw.starts_with("davs") {
                "webdavs"
            } else {
                "webdav"
            }
        } else {
            "webdav"
        };
        crate::commands::init::normalize_webdav_url(
            &repo_path_raw,
            scheme,
            args.webdav_user.as_deref(),
            args.webdav_pass.as_deref(),
        )?
    } else {
        repo_path_raw
    };

    let start_time = Instant::now();

    let archive_name = args.archive.clone().unwrap_or_else(default_archive_unique_name);

    info!("Creating archive '{}' in repository {}", archive_name, repo_path);

    // Validate paths exist
    for path in &args.paths {
        if !path.exists() {
            anyhow::bail!("Path does not exist: {:?}", path);
        }
    }

    // Build exclusion matcher
    let exclusion_matcher = build_exclusion_matcher(&args)?;

    // Configure compression
    let compressor = build_compressor(&args)?;
    debug!("Using compression: {} (level {})", compressor.config().algorithm, compressor.config().level.0);

    // Open repository
    let mut repo = open_repository(&repo_path).await
        .context("Failed to open repository")?;

    // Create progress bar if requested
    let progress = if cli.progress {
        println!("Scanning files to calculate total...");
        let total_files = count_files_recursive(&args.paths);
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")
                .unwrap()
        );
        pb.set_length(total_files);
        pb.set_message("Starting backup...");
        Some((pb, total_files))
    } else {
        None
    };

    // Build archive creator
    let mut creator = ArchiveCreator::new(&mut repo)
        .with_exclusions(exclusion_matcher)
        .with_compression(compressor.config().clone());

    // Apply forced chunker profile if provided
    if let Some(profile) = args.force_chunk_profile {
        creator = creator.with_forced_chunker_profile(profile);
    }

    // Set progress handler if enabled
    if let Some((pb, total)) = &progress {
        let cli_progress = CliProgress {
            pb: pb.clone(),
            list_files: args.list,
            total_files: *total,
        };
        creator = creator.with_progress(Box::new(cli_progress));
    }

    // Create the archive
    if args.dry_run {
        println!("Dry run - no archive created");
        return Ok(());
    }

    let archive = creator.create(&archive_name, &args.paths, args.comment.clone(), args.tags.clone()).await
        .map_err(|e| anyhow::anyhow!("Failed to create archive: {}", e))?;

    // Finish progress bar
    if let Some((pb, _)) = progress {
        pb.finish_with_message("Done");
        println!("[ 100%] transfer completed");
    }

    let duration = start_time.elapsed();

    println!("Archive: {}", archive_name);
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
        println!("Archive: {}", archive_name);
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
    total_files: u64,
}

impl BackupProgress for CliProgress {
    fn on_file_start(&self, path: &Path) {
        let pos = self.pb.position();
        let percent = if self.total_files > 0 {
            (pos * 100) / self.total_files
        } else {
            0
        };
        
        // Print the log line above the progress bar
        self.pb.println(format!("[{:3}%] {}", percent, path.display()));
        
        if !self.list_files {
            self.pb.set_message(format!("Processing: {}", path.display()));
        }
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

fn count_files_recursive(paths: &[std::path::PathBuf]) -> u64 {
    let mut count = 0;
    for path in paths {
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    count += count_files_recursive(&[path]);
                }
            }
        } else if path.is_file() {
            count += 1;
        }
    }
    count
}
