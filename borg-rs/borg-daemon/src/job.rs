//! Backup job execution

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{debug, error, info, warn, instrument};

use borg_core::{
    archive::{ArchiveCreator, Archive},
    compression::{CompressionAlgorithm, CompressionConfig, CompressionLevel, Compressor},
    exclusion::{ExclusionList, ExclusionPattern},
    repository::Repository,
};

use crate::config::BackupJob;

/// Statistics from a completed backup job
#[derive(Debug, Clone)]
pub struct JobStats {
    pub files_processed: u64,
    pub bytes_processed: u64,
    pub bytes_deduplicated: u64,
    pub bytes_compressed: u64,
    pub duration_secs: f64,
    pub archive_name: String,
}

/// Run a backup job
#[instrument(skip(job), fields(job_name = %job.name))]
pub async fn run_backup_job(job: &BackupJob) -> Result<JobStats> {
    let start_time = Instant::now();

    info!("Starting backup job");

    // Run pre-backup command if configured
    if let Some(ref cmd) = job.pre_command {
        run_hook_command("pre-backup", cmd).await?;
    }

    // Build exclusion matcher
    let exclusion_matcher = build_exclusion_matcher(job)?;

    // Resolve compression settings
    let compression = resolve_compression(job)?;

    // Generate archive name
    let archive_name = generate_archive_name(&job.archive_name)?;

    // Open repository
    let mut repo = open_repository(&job.repository).await?;

    // Create archive
    let archive = create_archive(
        &mut repo,
        &archive_name,
        &job.paths,
        exclusion_matcher,
        &compression,
        job,
    ).await?;

    // Run prune if configured (disabled for now - Pruner missing in core)
    /*
    if let Some(ref prune_config) = job.prune {
        run_prune(&repo, prune_config).await?;
    }
    */

    // Run post-backup command if configured
    if let Some(ref cmd) = job.post_command {
        if let Err(e) = run_hook_command("post-backup", cmd).await {
            warn!("Post-backup command failed: {}", e);
            // Don't fail the whole job for post-command failure
        }
    }

    let duration = start_time.elapsed();

    let stats = JobStats {
        files_processed: archive.stats.nfiles,
        bytes_processed: archive.stats.original_size,
        bytes_deduplicated: archive.stats.deduplicated_size,
        bytes_compressed: archive.stats.compressed_size,
        duration_secs: duration.as_secs_f64(),
        archive_name,
    };

    info!(
        files = stats.files_processed,
        bytes = stats.bytes_processed,
        duration_secs = stats.duration_secs,
        "Backup job completed"
    );

    Ok(stats)
}

/// Build exclusion list from job configuration
fn build_exclusion_matcher(job: &BackupJob) -> Result<ExclusionList> {
    let mut patterns = Vec::new();

    // Add shell-style patterns
    for pattern in &job.exclude_patterns {
        patterns.push(ExclusionPattern::glob(pattern));
    }

    let mut list = ExclusionList::from_patterns(patterns)?;

    // Load patterns from exclusion files
    for file in &job.exclude_files {
        if file.exists() {
            let file_list = ExclusionList::from_file(file)?;
            for pattern in file_list.patterns() {
                list.add_pattern(pattern.clone())?;
            }
        } else {
            warn!("Exclusion file not found: {:?}", file);
        }
    }

    // Configure special exclusions
    // Note: ExclusionList in core doesn't seem to have direct methods for caches, but we can add patterns
    if job.exclude_caches {
        // Common caches could be added here
    }

    Ok(list)
}

/// Resolve compression settings
fn resolve_compression(job: &BackupJob) -> Result<Compressor> {
    let algo_str = job.compression.as_deref().unwrap_or("zstd");
    let level_num = job.compression_level.unwrap_or(3) as u8;

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

/// Generate archive name from template
fn generate_archive_name(template: &str) -> Result<String> {
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown".to_string());
    let now = chrono::Local::now();

    let name = template
        .replace("{hostname}", &hostname)
        .replace("{now}", &now.format("%Y-%m-%d_%H:%M:%S").to_string())
        .replace("{now:%Y-%m-%d_%H:%M:%S}", &now.format("%Y-%m-%d_%H:%M:%S").to_string())
        .replace("{now:%Y-%m-%d}", &now.format("%Y-%m-%d").to_string());

    Ok(name)
}

/// Open or connect to repository
async fn open_repository(repo_str: &str) -> Result<Repository> {
    use borg_core::storage::{parse_storage_config, build_operator};
    let config = parse_storage_config(repo_str)?;
    let op = build_operator(config)?;
    
    // For now, assume no passphrase or handle it if available in config
    Repository::open(op, None).await.context("Failed to open repository")
}

/// Create a backup archive
async fn create_archive(
    repo: &mut Repository,
    archive_name: &str,
    paths: &[PathBuf],
    exclusion_list: ExclusionList,
    _compressor: &Compressor,
    job: &BackupJob,
) -> Result<Archive> {
    let mut creator = ArchiveCreator::new(repo);

    // Configure archive creation
    creator = creator.with_exclusions(exclusion_list);

    // Note: one_file_system, read_special, etc. are not yet in core ArchiveCreator
    
    // Create the archive (async)
    creator.create(archive_name, paths, job.archive_name.clone().into()).await
        .map_err(|e| anyhow::anyhow!("Failed to create archive: {}", e))
}

/*
/// Run automatic pruning
async fn run_prune(repo: &Repository, config: &PruneConfig) -> Result<()> {
    info!("Running automatic prune");

    let mut pruner = repo.pruner();
    // ...
    Ok(())
}
*/

/// Run a hook command (pre/post backup)
async fn run_hook_command(hook_type: &str, command: &str) -> Result<()> {
    debug!("Running {} command: {}", hook_type, command);

    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .await
        .context(format!("Failed to execute {} command", hook_type))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("{} command failed: {}", hook_type, stderr);
        anyhow::bail!("{} command failed with status: {}", hook_type, output.status);
    }

    debug!("{} command completed successfully", hook_type);
    Ok(())
}

/// Retry a backup job with exponential backoff
pub async fn run_backup_job_with_retry(
    job: &BackupJob,
) -> Result<JobStats> {
    let retry_config = &job.retry;
    let mut attempt = 0;
    let mut delay = std::time::Duration::from_secs(retry_config.initial_delay);

    loop {
        attempt += 1;
        
        match run_backup_job(job).await {
            Ok(stats) => return Ok(stats),
            Err(e) => {
                if attempt > retry_config.max_retries {
                    error!(
                        "Job '{}' failed after {} attempts: {}", 
                        job.name, attempt, e
                    );
                    return Err(e);
                }

                warn!(
                    "Job '{}' attempt {} failed: {}. Retrying in {:?}",
                    job.name, attempt, e, delay
                );

                tokio::time::sleep(delay).await;

                // Calculate next delay with exponential backoff
                delay = std::cmp::min(
                    std::time::Duration::from_secs_f64(
                        delay.as_secs_f64() * retry_config.backoff_multiplier
                    ),
                    std::time::Duration::from_secs(retry_config.max_delay),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_archive_name_generation() {
        let template = "{hostname}-backup-{now:%Y-%m-%d}";
        let name = generate_archive_name(template).unwrap();
        
        // Should contain hostname and date
        assert!(name.contains('-'));
        assert!(name.contains("backup"));
    }

    #[test]
    fn test_compression_resolution() {
        let job = BackupJob {
            name: "test".to_string(),
            schedule: "0 0 * * *".to_string(),
            repository: "/tmp/repo".to_string(),
            paths: vec![],
            exclude_patterns: vec![],
            exclude_files: vec![],
            exclude_caches: true,
            exclude_if_present: vec![],
            archive_name: "test".to_string(),
            compression: Some("zstd".to_string()),
            compression_level: Some(5),
            one_file_system: false,
            read_special: false,
            numeric_ids: false,
            noatime: true,
            exclude_nodump: false,
            prune: None,
            pre_command: None,
            post_command: None,
            notifications: Default::default(),
            priority: 100,
            enabled: true,
            retry: Default::default(),
        };

        let compressor = resolve_compression(&job).unwrap();
        // Compression should be created successfully
    }
}
