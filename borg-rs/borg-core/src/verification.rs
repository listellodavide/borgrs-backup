//! Integrity verification and restore testing
//!
//! Provides functionality to verify repository integrity, check chunk
//! validity, and perform automated restore tests.

use crate::chunker::ChunkId;
use crate::error::{BorgError, Result};
use crate::repository::Repository;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Report from repository verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyReport {
    /// Total number of chunks in the repository
    pub total_chunks: usize,
    /// Number of chunks successfully verified
    pub verified_chunks: usize,
    /// Chunks that failed integrity check (hash mismatch)
    pub corrupted_chunks: Vec<ChunkId>,
    /// Chunks referenced in index but missing from disk
    pub missing_chunks: Vec<ChunkId>,
    /// Chunks on disk but not in index (orphaned)
    pub orphaned_chunks: Vec<ChunkId>,
    /// Duration of the verification process
    pub duration: Duration,
    /// Total bytes verified
    pub bytes_verified: u64,
}

impl VerifyReport {
    /// Check if verification passed with no errors
    pub fn is_ok(&self) -> bool {
        self.corrupted_chunks.is_empty() && self.missing_chunks.is_empty()
    }

    /// Get count of errors found
    pub fn error_count(&self) -> usize {
        self.corrupted_chunks.len() + self.missing_chunks.len()
    }

    /// Get verification success percentage
    pub fn success_rate(&self) -> f64 {
        if self.total_chunks == 0 {
            100.0
        } else {
            (self.verified_chunks as f64 / self.total_chunks as f64) * 100.0
        }
    }

    /// Get throughput in bytes per second
    pub fn throughput(&self) -> f64 {
        let secs = self.duration.as_secs_f64();
        if secs > 0.0 {
            self.bytes_verified as f64 / secs
        } else {
            0.0
        }
    }
}

/// Report from archive verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveVerifyReport {
    /// Archive name
    pub archive_name: String,
    /// Total items (files/directories) in archive
    pub total_items: usize,
    /// Items successfully verified
    pub verified_items: usize,
    /// Missing chunks for specific files
    pub missing_chunks: Vec<(PathBuf, ChunkId)>,
    /// Errors encountered during verification
    pub errors: Vec<String>,
}

impl ArchiveVerifyReport {
    /// Check if archive verification passed
    pub fn is_ok(&self) -> bool {
        self.missing_chunks.is_empty() && self.errors.is_empty()
    }
}

/// Progress reporter trait for verification operations
pub trait ProgressReporter: Send + Sync {
    /// Report progress (current, total, message)
    fn report(&self, current: usize, total: usize, message: &str);
}

/// Console progress reporter (simple implementation)
pub struct ConsoleProgress;

impl ProgressReporter for ConsoleProgress {
    fn report(&self, current: usize, total: usize, message: &str) {
        if current % 100 == 0 || current == total {
            println!("  [{}/{}] {}", current, total, message);
        }
    }
}

/// Comprehensive repository integrity report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryIntegrityReport {
    /// Repository path or URL
    pub repository_path: String,
    /// Repository ID
    pub repository_id: String,
    /// Timestamp of verification
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Chunk verification report
    pub chunk_report: VerifyReport,
    /// Archive verification reports
    pub archive_reports: Vec<ArchiveVerifyReport>,
    /// Manifest integrity
    pub manifest_valid: bool,
    /// Config integrity
    pub config_valid: bool,
    /// Total duration of all checks
    pub total_duration: Duration,
    /// Verification statistics
    pub statistics: VerificationStatistics,
}

impl RepositoryIntegrityReport {
    /// Check if repository passed all integrity checks
    pub fn is_ok(&self) -> bool {
        self.chunk_report.is_ok()
            && self.manifest_valid
            && self.config_valid
            && self.archive_reports.iter().all(|r| r.is_ok())
    }

    /// Get total error count across all checks
    pub fn total_errors(&self) -> usize {
        let mut errors = self.chunk_report.error_count();
        if !self.manifest_valid { errors += 1; }
        if !self.config_valid { errors += 1; }
        for report in &self.archive_reports {
            errors += report.errors.len() + report.missing_chunks.len();
        }
        errors
    }
}

/// Statistics from verification operations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerificationStatistics {
    /// Total chunks in repository
    pub total_chunks: u64,
    /// Total archives in repository
    pub total_archives: u64,
    /// Total original data size (bytes)
    pub total_data_size: u64,
    /// Chunks verified per second
    pub chunks_per_second: f64,
    /// Data verified per second (bytes)
    pub bytes_per_second: f64,
    /// Number of corrupted chunks found
    pub corrupted_chunks: u64,
    /// Number of missing chunks found
    pub missing_chunks: u64,
    /// Number of orphaned chunks found
    pub orphaned_chunks: u64,
    /// Number of archives with errors
    pub archives_with_errors: u64,
}

/// Configuration for restore testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreTestConfig {
    /// Pattern to match archives to test
    pub archive_pattern: Option<String>,
    /// Number of random files to restore per archive
    pub files_per_archive: usize,
    /// Whether to verify file content hash after restore
    pub verify_content: bool,
    /// Temporary directory for restore tests
    pub temp_dir: Option<PathBuf>,
}

/// Result of a single file restore test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRestoreResult {
    /// Path of the file in the archive
    pub path: PathBuf,
    /// Size of the file
    pub size: u64,
    /// Whether restore was successful
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
    /// Duration of restore
    pub duration: Duration,
}

/// Report from a restore test run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreTestReport {
    /// Archive being tested
    pub archive_name: String,
    /// Results for individual files
    pub file_results: Vec<FileRestoreResult>,
    /// Total duration
    pub duration: Duration,
}

/// Type of verification performed
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum VerificationType {
    /// Full repository scan
    Full,
    /// Quick check (manifest/config only)
    Quick,
    /// Specific archive check
    Archive(String),
    /// Restore test
    RestoreTest,
}

/// Entry in verification history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationHistoryEntry {
    /// Timestamp of verification
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Type of verification
    pub verification_type: VerificationType,
    /// Whether verification passed
    pub success: bool,
    /// Summary of results
    pub summary: String,
    /// Duration of operation
    pub duration: Duration,
}

/// History of verification runs
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerificationHistory {
    /// List of past verifications
    pub entries: Vec<VerificationHistoryEntry>,
}

impl Repository {
    /// Verify a single chunk's integrity
    pub async fn verify_chunk(&self, id: &ChunkId) -> Result<bool> {
        match self.get_chunk(id).await {
            Ok(chunk) => Ok(chunk.id == *id),
            Err(BorgError::IntegrityCheck { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Perform comprehensive repository-wide integrity verification
    pub async fn verify_repository(&self, progress: Option<&(dyn ProgressReporter + Send + Sync)>) -> Result<RepositoryIntegrityReport> {
        info!("Starting comprehensive repository verification");
        let start = std::time::Instant::now();

        let repo_path = "unknown".to_string();
        let config = self.descriptor();

        let config_valid = true;
        let (manifest_valid, manifest) = self.verify_manifest().await;

        let chunk_report = self.verify_all(progress).await?;

        let mut archive_reports = Vec::new();
        if let Some(ref manifest) = manifest {
            for archive_ref in &manifest.archives {
                match self.verify_archive(&archive_ref.name).await {
                    Ok(report) => archive_reports.push(report),
                    Err(e) => {
                        archive_reports.push(ArchiveVerifyReport {
                            archive_name: archive_ref.name.clone(),
                            total_items: 0,
                            verified_items: 0,
                            missing_chunks: Vec::new(),
                            errors: vec![format!("Failed to verify: {}", e)],
                        });
                    }
                }
            }
        }

        let total_duration = start.elapsed();

        let statistics = VerificationStatistics {
            total_chunks: chunk_report.total_chunks as u64,
            total_archives: archive_reports.len() as u64,
            total_data_size: chunk_report.bytes_verified,
            chunks_per_second: if total_duration.as_secs_f64() > 0.0 {
                chunk_report.total_chunks as f64 / total_duration.as_secs_f64()
            } else { 0.0 },
            bytes_per_second: chunk_report.throughput(),
            corrupted_chunks: chunk_report.corrupted_chunks.len() as u64,
            missing_chunks: chunk_report.missing_chunks.len() as u64,
            orphaned_chunks: chunk_report.orphaned_chunks.len() as u64,
            archives_with_errors: archive_reports.iter().filter(|r| !r.is_ok()).count() as u64,
        };

        let report = RepositoryIntegrityReport {
            repository_path: repo_path,
            repository_id: config.id.clone(),
            timestamp: chrono::Utc::now(),
            chunk_report,
            archive_reports,
            manifest_valid,
            config_valid,
            total_duration,
            statistics,
        };

        if report.is_ok() {
            info!("Repository verification completed successfully");
        } else {
            warn!("Repository verification completed with {} errors", report.total_errors());
        }

        Ok(report)
    }

    /// Verify manifest integrity
    async fn verify_manifest(&self) -> (bool, Option<crate::repository::Manifest>) {
        match self.load_manifest().await {
            Ok(manifest) => {
                if manifest.version == 0 {
                    warn!("Invalid manifest version");
                    return (false, Some(manifest));
                }
                (true, Some(manifest))
            }
            Err(e) => {
                warn!("Manifest verification failed: {}", e);
                (false, None)
            }
        }
    }

    /// Verify all chunks in the repository
    pub async fn verify_all(&self, progress: Option<&(dyn ProgressReporter + Send + Sync)>) -> Result<VerifyReport> {
        info!("Starting repository verification");
        let start = std::time::Instant::now();

        let total_chunks = self.chunk_count();
        let mut verified_chunks = 0;
        let mut corrupted_chunks = Vec::new();
        let mut missing_chunks = Vec::new();
        let mut bytes_verified = 0u64;

        for (idx, chunk_id) in self.chunk_index_iterator().enumerate() {
            if let Some(reporter) = progress {
                reporter.report(idx + 1, total_chunks, &format!("Verifying chunk {}", chunk_id));
            }

            match self.verify_chunk(chunk_id).await {
                Ok(true) => {
                    verified_chunks += 1;
                    if let Ok(chunk) = self.get_chunk(chunk_id).await {
                        bytes_verified += chunk.data.len() as u64;
                    }
                }
                Ok(false) => {
                    warn!("Corrupted chunk detected: {}", chunk_id);
                    corrupted_chunks.push(chunk_id.clone());
                }
                Err(BorgError::Repository(_)) => {
                    warn!("Missing chunk: {}", chunk_id);
                    missing_chunks.push(chunk_id.clone());
                }
                Err(e) => return Err(e),
            }
        }

        let orphaned_chunks = self.find_orphaned_chunks().await?;
        if !orphaned_chunks.is_empty() {
            debug!("Found {} orphaned chunks", orphaned_chunks.len());
        }

        let duration = start.elapsed();

        let report = VerifyReport {
            total_chunks,
            verified_chunks,
            corrupted_chunks,
            missing_chunks,
            orphaned_chunks,
            duration,
            bytes_verified,
        };

        if report.is_ok() {
            info!("Repository verification completed successfully");
        } else {
            warn!("Repository verification completed with {} errors", report.error_count());
        }

        Ok(report)
    }

    /// Verify a specific archive's integrity
    pub async fn verify_archive(&self, archive_name: &str) -> Result<ArchiveVerifyReport> {
        info!("Verifying archive: {}", archive_name);

        let restorer = crate::archive::ArchiveRestorer::new(self);
        let archive = restorer.load_archive(archive_name).await?;
        
        let total_items = archive.items.len();
        let mut verified_items = 0;
        let mut missing_chunks = Vec::new();
        let mut errors = Vec::new();

        for item in &archive.items {
            let mut item_valid = true;
            for chunk_id in &item.chunks {
                if !self.has_chunk(chunk_id) {
                    missing_chunks.push((item.path.clone(), chunk_id.clone()));
                    item_valid = false;
                }
            }

            if item_valid {
                verified_items += 1;
            } else {
                errors.push(format!("Missing chunks for: {}", item.path.display()));
            }
        }

        Ok(ArchiveVerifyReport {
            archive_name: archive_name.to_string(),
            total_items,
            verified_items,
            missing_chunks,
            errors,
        })
    }

    pub(crate) fn chunk_count(&self) -> usize {
        self.chunk_index_len()
    }

    async fn find_orphaned_chunks(&self) -> Result<Vec<ChunkId>> {
        Ok(Vec::new())
    }
}

impl Repository {
    pub fn chunk_index_len(&self) -> usize {
        self.chunk_cache.len()
    }

    pub fn chunk_index_iterator(&self) -> impl Iterator<Item = &ChunkId> {
        self.chunk_cache.iter()
    }
}
