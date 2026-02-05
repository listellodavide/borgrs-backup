//! Repository and archive verification command
//!
//! Provides comprehensive integrity verification for repositories and archives,
//! including chunk verification, archive validation, and restore testing.

use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

use borg_core::verification::{ProgressReporter, VerifyReport, ArchiveVerifyReport};
use super::{format_duration, format_size, get_repo_path, open_repository};
use crate::{Cli, VerifyArgs};

/// CLI-integrated progress reporter using indicatif
struct CliProgressReporter {
    progress_bar: ProgressBar,
}

impl CliProgressReporter {
    fn new(total: usize) -> Self {
        let pb = ProgressBar::new(total as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("#>-")
        );
        Self { progress_bar: pb }
    }
}

impl ProgressReporter for CliProgressReporter {
    fn report(&self, current: usize, total: usize, message: &str) {
        self.progress_bar.set_length(total as u64);
        self.progress_bar.set_position(current as u64);
        self.progress_bar.set_message(message.to_string());
    }
}

impl Drop for CliProgressReporter {
    fn drop(&mut self) {
        self.progress_bar.finish_and_clear();
    }
}

pub async fn run(cli: &Cli, args: &VerifyArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let repo = open_repository(&repo_path).await?;

    println!("Verifying repository: {}", repo_path);
    println!();

    let mut all_passed = true;
    let mut total_errors = 0;

    // Repository verification
    if !args.archives_only {
        println!("=== Repository Verification ===");
        
        let progress: Option<Box<dyn ProgressReporter + Send + Sync>> = if cli.progress {
            let chunk_count = repo.chunk_index_len();
            Some(Box::new(CliProgressReporter::new(chunk_count)))
        } else {
            None
        };

        let report = repo.verify_all(progress.as_deref()).await
            .context("Failed to verify repository")?;

        print_verify_report(&report);

        if !report.is_ok() {
            all_passed = false;
            total_errors += report.error_count();
        }

        println!();
    }

    // Archive verification
    if !args.repository_only {
        println!("=== Archive Verification ===");
        
        let manifest = repo.load_manifest().await
            .context("Failed to load manifest")?;
        
        let archives_to_verify = if let Some(ref archive_name) = args.archive {
            // Verify specific archive
            vec![archive_name.clone()]
        } else {
            // Verify all archives
            manifest.archives.iter().map(|a| a.name.clone()).collect()
        };

        if archives_to_verify.is_empty() {
            println!("  No archives to verify.");
        } else {
            for archive_name in &archives_to_verify {
                let archive_report = repo.verify_archive(archive_name).await;
                match archive_report {
                    Ok(report) => {
                        print_archive_verify_report(&report);
                        if !report.is_ok() {
                            all_passed = false;
                            total_errors += report.errors.len() + report.missing_chunks.len();
                        }
                    }
                    Err(e) => {
                        println!("  Archive '{}': FAILED - {}", archive_name, e);
                        all_passed = false;
                        total_errors += 1;
                    }
                }
            }
        }
        println!();
    }

    // Restore testing (if requested)
    if args.test_restore {
        println!("=== Restore Testing ===");
        
        let manifest = repo.load_manifest().await
            .context("Failed to load manifest")?;

        if manifest.archives.is_empty() {
            println!("  No archives available for restore testing.");
        } else {
            // Test restore on the most recent archive
            let latest_archive = manifest.archives
                .iter()
                .max_by_key(|a| a.time)
                .unwrap();

            println!("  Testing restore of archive '{}'...", latest_archive.name);
            
            match run_restore_test(&repo, &latest_archive.name, args.sample_files).await {
                Ok(restore_report) => {
                    print_restore_test_report(&restore_report);
                    if !restore_report.success {
                        all_passed = false;
                        total_errors += restore_report.errors.len();
                    }
                }
                Err(e) => {
                    println!("  Restore test FAILED: {}", e);
                    all_passed = false;
                    total_errors += 1;
                }
            }
        }
        println!();
    }

    // Summary
    println!("=== Verification Summary ===");
    if all_passed {
        println!("  Status: PASSED");
        println!("  All integrity checks completed successfully.");
    } else {
        println!("  Status: FAILED");
        println!("  Total errors: {}", total_errors);
        
        if args.repair {
            println!();
            println!("  Repair mode not yet implemented.");
            println!("  Consider removing and re-creating corrupted archives.");
        }
    }

    if !all_passed {
        std::process::exit(1);
    }

    Ok(())
}

/// Print repository verification report
fn print_verify_report(report: &VerifyReport) {
    println!("  Chunks verified: {}/{}", report.verified_chunks, report.total_chunks);
    println!("  Data verified: {}", format_size(report.bytes_verified));
    println!("  Duration: {}", format_duration(report.duration.as_secs_f64()));

    if !report.corrupted_chunks.is_empty() {
        println!("  Corrupted chunks: {}", report.corrupted_chunks.len());
        for chunk_id in &report.corrupted_chunks[..report.corrupted_chunks.len().min(5)] {
            println!("    - {}", chunk_id);
        }
        if report.corrupted_chunks.len() > 5 {
            println!("    ... and {} more", report.corrupted_chunks.len() - 5);
        }
    }

    if !report.missing_chunks.is_empty() {
        println!("  Missing chunks: {}", report.missing_chunks.len());
        for chunk_id in &report.missing_chunks[..report.missing_chunks.len().min(5)] {
            println!("    - {}", chunk_id);
        }
        if report.missing_chunks.len() > 5 {
            println!("    ... and {} more", report.missing_chunks.len() - 5);
        }
    }

    if !report.orphaned_chunks.is_empty() {
        println!("  Orphaned chunks: {}", report.orphaned_chunks.len());
        println!("    (Use 'compact' command to clean up)");
    }

    let status = if report.is_ok() { "OK" } else { "FAILED" };
    println!("  Repository status: {}", status);
}

/// Print archive verification report
fn print_archive_verify_report(report: &ArchiveVerifyReport) {
    let status = if report.is_ok() { "OK" } else { "FAILED" };
    println!("  Archive '{}': {}", report.archive_name, status);
    println!("    Items verified: {}/{}", report.verified_items, report.total_items);

    if !report.missing_chunks.is_empty() {
        println!("    Missing chunks: {}", report.missing_chunks.len());
        for (path, chunk_id) in &report.missing_chunks[..report.missing_chunks.len().min(3)] {
            println!("      - {} ({})", path.display(), chunk_id);
        }
    }

    if !report.errors.is_empty() {
        println!("    Errors: {}", report.errors.len());
        for error in &report.errors[..report.errors.len().min(3)] {
            println!("      - {}", error);
        }
    }
}

/// Report from restore testing
#[derive(Debug)]
struct RestoreTestReport {
    /// Archive name tested
    archive_name: String,
    /// Number of files sampled
    files_sampled: usize,
    /// Number of files successfully restored
    files_restored: usize,
    /// Bytes restored
    bytes_restored: u64,
    /// Duration of test
    duration: Duration,
    /// Whether all tests passed
    success: bool,
    /// Errors encountered
    errors: Vec<String>,
}

/// Run restore test on an archive
async fn run_restore_test(
    repo: &borg_core::repository::Repository,
    archive_name: &str,
    sample_count: Option<usize>,
) -> Result<RestoreTestReport> {
    use borg_core::archive::ArchiveExtractor;
    use tempfile::TempDir;

    let start = std::time::Instant::now();
    let extractor = ArchiveExtractor::new(repo);
    
    // Load archive
    let archive = extractor.load_archive(archive_name).await
        .context("Failed to load archive")?;

    let sample_count = sample_count.unwrap_or(10);
    let mut files_sampled = 0;
    let mut files_restored = 0;
    let mut bytes_restored = 0u64;
    let mut errors = Vec::new();

    // Create temporary directory for restore test
    let temp_dir = TempDir::new()
        .context("Failed to create temp directory")?;

    // Sample files from archive
    let file_items: Vec<_> = archive.items.iter()
        .filter(|item| matches!(item.item_type, borg_core::archive::ItemType::File))
        .take(sample_count)
        .collect();

    for item in &file_items {
        files_sampled += 1;
        
        // Try to restore each file
        match restore_single_file(repo, item, temp_dir.path()).await {
            Ok(size) => {
                files_restored += 1;
                bytes_restored += size;
            }
            Err(e) => {
                errors.push(format!("{}: {}", item.path.display(), e));
            }
        }
    }

    let duration = start.elapsed();
    let success = errors.is_empty() && files_restored == files_sampled;

    Ok(RestoreTestReport {
        archive_name: archive_name.to_string(),
        files_sampled,
        files_restored,
        bytes_restored,
        duration,
        success,
        errors,
    })
}

/// Restore a single file and verify its contents
async fn restore_single_file(
    repo: &borg_core::repository::Repository,
    item: &borg_core::archive::ArchiveItem,
    dest_dir: &std::path::Path,
) -> Result<u64> {
    let mut file_data = Vec::new();
    
    for chunk_id in &item.chunks {
        let chunk = repo.get_chunk(chunk_id).await
            .context("Failed to retrieve chunk")?;
        file_data.extend_from_slice(&chunk.data);
    }

    let dest_path = dest_dir.join(&item.path);
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    
    std::fs::write(&dest_path, &file_data)?;
    
    // Verify size matches
    let written_size = std::fs::metadata(&dest_path)?.len();
    if written_size != item.size {
        anyhow::bail!(
            "Size mismatch: expected {} bytes, got {} bytes",
            item.size, written_size
        );
    }

    Ok(written_size)
}

/// Print restore test report
fn print_restore_test_report(report: &RestoreTestReport) {
    let status = if report.success { "PASSED" } else { "FAILED" };
    println!("  Restore test: {}", status);
    println!("    Files tested: {}/{}", report.files_restored, report.files_sampled);
    println!("    Data restored: {}", format_size(report.bytes_restored));
    println!("    Duration: {}", format_duration(report.duration.as_secs_f64()));

    if !report.errors.is_empty() {
        println!("    Errors:");
        for error in &report.errors[..report.errors.len().min(5)] {
            println!("      - {}", error);
        }
        if report.errors.len() > 5 {
            println!("      ... and {} more", report.errors.len() - 5);
        }
    }
}
