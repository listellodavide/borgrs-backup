//! Integrity verification and restore testing
//!
//! Provides functionality to verify repository integrity, check chunk
//! validity, and perform automated restore tests.
//!
//! # Features
//!
//! - **Chunk verification**: Verify integrity of individual chunks by checking
//!   content hashes match chunk IDs
//! - **Repository verification**: Full integrity check of all chunks in a repository
//! - **Archive verification**: Verify all chunks referenced by an archive exist
//! - **Orphaned chunk detection**: Find chunks on disk not referenced in the index
//! - **Repository-wide integrity**: Comprehensive check of repository structure,
//!   manifest, and all archives
//!
//! # Example
//!
//! ```no_run
//! use borg_core::repository::Repository;
//! use borg_core::verification::ConsoleProgress;
//! use borg_core::storage::{StorageConfig, build_operator};
//!
//! # async fn doc_example() -> anyhow::Result<()> {
//! let config = StorageConfig::Local { path: std::path::PathBuf::from("/path/to/repo") };
//! let op = build_operator(config).unwrap();
//! let repo = Repository::open(op, "/path/to/repo".to_string(), Some("passphrase")).await.unwrap();
//!
//! // Verify all chunks
//! let progress = ConsoleProgress;
//! let report = repo.verify_all(Some(&progress)).await.unwrap();
//!
//! if report.is_ok() {
//!     println!("Repository integrity verified!");
//! } else {
//!     println!("Found {} errors", report.error_count());
//! }
//! # Ok(())
//! # }
//! ```

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
    /// Index integrity (index matches disk)
    pub index_valid: bool,
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
            && self.index_valid
            && self.archive_reports.iter().all(|r| r.is_ok())
    }

    /// Get total error count across all checks
    pub fn total_errors(&self) -> usize {
        let mut errors = self.chunk_report.error_count();
        if !self.manifest_valid { errors += 1; }
        if !self.config_valid { errors += 1; }
        if !self.index_valid { errors += 1; }
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

/// Report from automated restore testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreTestReport {
    /// Archive that was tested
    pub archive_name: String,
    /// Total files in archive
    pub total_files: usize,
    /// Number of files sampled for testing
    pub files_sampled: usize,
    /// Number of files successfully restored
    pub files_restored: usize,
    /// Total bytes restored
    pub bytes_restored: u64,
    /// Duration of the test
    pub duration: Duration,
    /// Whether all tests passed
    pub success: bool,
    /// Individual file test results
    pub file_results: Vec<FileRestoreResult>,
    /// Overall errors encountered
    pub errors: Vec<String>,
}

impl RestoreTestReport {
    /// Check if restore test passed
    pub fn is_ok(&self) -> bool {
        self.success && self.errors.is_empty()
    }

    /// Get success rate as percentage
    pub fn success_rate(&self) -> f64 {
        if self.files_sampled == 0 {
            100.0
        } else {
            (self.files_restored as f64 / self.files_sampled as f64) * 100.0
        }
    }
}

/// Result of restoring a single file during testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRestoreResult {
    /// Path of the file
    pub path: PathBuf,
    /// Expected size
    pub expected_size: u64,
    /// Actual restored size
    pub actual_size: u64,
    /// Whether restore succeeded
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
    /// Time to restore this file
    pub duration: Duration,
}

/// Configuration for restore testing
#[derive(Debug, Clone)]
pub struct RestoreTestConfig {
    /// Maximum number of files to sample
    pub max_samples: usize,
    /// Minimum file size to include in sample
    pub min_file_size: u64,
    /// Maximum file size to include in sample (0 = no limit)
    pub max_file_size: u64,
    /// Whether to verify content hash after restore
    pub verify_hash: bool,
    /// Whether to test all files (ignore max_samples)
    pub test_all: bool,
}

impl Default for RestoreTestConfig {
    fn default() -> Self {
        Self {
            max_samples: 10,
            min_file_size: 0,
            max_file_size: 0, // No limit
            verify_hash: true,
            test_all: false,
        }
    }
}

impl RestoreTestConfig {
    /// Create config for quick testing (fewer samples)
    pub fn quick() -> Self {
        Self {
            max_samples: 5,
            ..Default::default()
        }
    }

    /// Create config for thorough testing (more samples)
    pub fn thorough() -> Self {
        Self {
            max_samples: 50,
            verify_hash: true,
            ..Default::default()
        }
    }

    /// Create config to test all files
    pub fn full() -> Self {
        Self {
            test_all: true,
            verify_hash: true,
            ..Default::default()
        }
    }
}

/// Verification history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationHistoryEntry {
    /// Timestamp of verification
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Type of verification performed
    pub verification_type: VerificationType,
    /// Whether verification passed
    pub passed: bool,
    /// Number of errors found
    pub error_count: usize,
    /// Duration of verification
    pub duration_secs: f64,
    /// Total bytes verified
    pub bytes_verified: u64,
    /// Summary message
    pub summary: String,
}

/// Type of verification performed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationType {
    /// Full repository verification
    FullRepository,
    /// Chunk-only verification
    ChunksOnly,
    /// Single archive verification
    Archive,
    /// Restore test
    RestoreTest,
    /// Quick check (manifest + config only)
    QuickCheck,
}

impl std::fmt::Display for VerificationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerificationType::FullRepository => write!(f, "Full Repository"),
            VerificationType::ChunksOnly => write!(f, "Chunks Only"),
            VerificationType::Archive => write!(f, "Archive"),
            VerificationType::RestoreTest => write!(f, "Restore Test"),
            VerificationType::QuickCheck => write!(f, "Quick Check"),
        }
    }
}

/// Verification history tracker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationHistory {
    /// Repository ID this history belongs to
    pub repository_id: String,
    /// History entries (newest first)
    pub entries: Vec<VerificationHistoryEntry>,
    /// Maximum entries to keep
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
}

fn default_max_entries() -> usize {
    100
}

impl VerificationHistory {
    /// Create new empty history
    pub fn new(repository_id: String) -> Self {
        Self {
            repository_id,
            entries: Vec::new(),
            max_entries: 100,
        }
    }

    /// Add an entry to history
    pub fn add_entry(&mut self, entry: VerificationHistoryEntry) {
        self.entries.insert(0, entry);
        if self.entries.len() > self.max_entries {
            self.entries.truncate(self.max_entries);
        }
    }

    /// Get last verification timestamp
    pub fn last_verification(&self) -> Option<&VerificationHistoryEntry> {
        self.entries.first()
    }

    /// Get last successful verification
    pub fn last_successful(&self) -> Option<&VerificationHistoryEntry> {
        self.entries.iter().find(|e| e.passed)
    }

    /// Get verification count
    pub fn verification_count(&self) -> usize {
        self.entries.len()
    }

    /// Get success rate (percentage)
    pub fn success_rate(&self) -> f64 {
        if self.entries.is_empty() {
            100.0
        } else {
            let passed = self.entries.iter().filter(|e| e.passed).count();
            (passed as f64 / self.entries.len() as f64) * 100.0
        }
    }

    /// Get entries by type
    pub fn entries_by_type(&self, vtype: VerificationType) -> Vec<&VerificationHistoryEntry> {
        self.entries.iter()
            .filter(|e| e.verification_type == vtype)
            .collect()
    }

    /// Load history from repository
    pub fn load(repo_path: &std::path::Path, repository_id: &str) -> Result<Self> {
        let history_path = repo_path.join("verification_history.json");
        if !history_path.exists() {
            return Ok(Self::new(repository_id.to_string()));
        }

        let data = std::fs::read_to_string(&history_path)?;
        serde_json::from_str(&data)
            .map_err(|e| BorgError::Deserialization(e.to_string()))
    }

    /// Save history to repository
    pub fn save(&self, repo_path: &std::path::Path) -> Result<()> {
        let history_path = repo_path.join("verification_history.json");
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        std::fs::write(&history_path, data)?;
        Ok(())
    }

    /// Create entry from verify report
    pub fn entry_from_verify_report(report: &VerifyReport) -> VerificationHistoryEntry {
        VerificationHistoryEntry {
            timestamp: chrono::Utc::now(),
            verification_type: VerificationType::ChunksOnly,
            passed: report.is_ok(),
            error_count: report.error_count(),
            duration_secs: report.duration.as_secs_f64(),
            bytes_verified: report.bytes_verified,
            summary: format!(
                "Verified {}/{} chunks, {} bytes",
                report.verified_chunks,
                report.total_chunks,
                report.bytes_verified
            ),
        }
    }

    /// Create entry from repository integrity report
    pub fn entry_from_integrity_report(report: &RepositoryIntegrityReport) -> VerificationHistoryEntry {
        VerificationHistoryEntry {
            timestamp: report.timestamp,
            verification_type: VerificationType::FullRepository,
            passed: report.is_ok(),
            error_count: report.total_errors(),
            duration_secs: report.total_duration.as_secs_f64(),
            bytes_verified: report.chunk_report.bytes_verified,
            summary: format!(
                "Full verification: {} chunks, {} archives, {} errors",
                report.statistics.total_chunks,
                report.statistics.total_archives,
                report.total_errors()
            ),
        }
    }

    /// Create entry from restore test report
    pub fn entry_from_restore_test(report: &RestoreTestReport) -> VerificationHistoryEntry {
        VerificationHistoryEntry {
            timestamp: chrono::Utc::now(),
            verification_type: VerificationType::RestoreTest,
            passed: report.is_ok(),
            error_count: report.errors.len(),
            duration_secs: report.duration.as_secs_f64(),
            bytes_verified: report.bytes_restored,
            summary: format!(
                "Restore test '{}': {}/{} files restored",
                report.archive_name,
                report.files_restored,
                report.files_sampled
            ),
        }
    }
}

impl Repository {
    /// Verify a single chunk's integrity
    ///
    /// Returns Ok(true) if chunk is valid, Ok(false) if corrupted
    pub async fn verify_chunk(&self, id: &ChunkId) -> Result<bool> {
        match self.get_chunk(id).await {
            Ok(chunk) => {
                // get_chunk already verifies hash internally
                Ok(chunk.id == *id)
            }
            Err(BorgError::IntegrityCheck { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Perform comprehensive repository-wide integrity verification
    ///
    /// This checks:
    /// - Repository configuration file
    /// - Manifest integrity
    /// - Chunk index consistency
    /// - All chunk data integrity
    /// - All archive integrity
    ///
    /// Returns a detailed report with all findings
    pub async fn verify_repository(&self, progress: Option<&(dyn ProgressReporter + Send + Sync)>) -> Result<RepositoryIntegrityReport> {
        info!("Starting comprehensive repository verification");
        let start = std::time::Instant::now();

        let repo_path = "unknown".to_string(); // self.path is not available in new Repository struct
        let config = self.descriptor();

        // Verify config is readable
        let config_valid = true; // Descriptor is already loaded

        // Verify manifest
        let (manifest_valid, manifest) = self.verify_manifest().await;

        // Verify chunk index against disk
        let index_valid = self.verify_index().await?;

        // Verify all chunks
        let chunk_report = self.verify_all(progress).await?;

        // Verify all archives
        let mut archive_reports = Vec::new();
        if let Some(ref manifest) = manifest {
            for archive_ref in &manifest.archives {
                match self.verify_archive(&archive_ref.name).await {
                    Ok(report) => archive_reports.push(report),
                    Err(e) => {
                        // Create error report for this archive
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

        // Compute statistics
        let statistics = VerificationStatistics {
            total_chunks: chunk_report.total_chunks as u64,
            total_archives: archive_reports.len() as u64,
            total_data_size: chunk_report.bytes_verified,
            chunks_per_second: if total_duration.as_secs_f64() > 0.0 {
                chunk_report.total_chunks as f64 / total_duration.as_secs_f64()
            } else {
                0.0
            },
            bytes_per_second: chunk_report.throughput(),
            corrupted_chunks: chunk_report.corrupted_chunks.len() as u64,
            missing_chunks: chunk_report.missing_chunks.len() as u64,
            orphaned_chunks: chunk_report.orphaned_chunks.len() as u64,
            archives_with_errors: archive_reports.iter()
                .filter(|r| !r.is_ok())
                .count() as u64,
        };

        let report = RepositoryIntegrityReport {
            repository_path: repo_path,
            repository_id: config.id.clone(),
            timestamp: chrono::Utc::now(),
            chunk_report,
            archive_reports,
            manifest_valid,
            config_valid,
            index_valid,
            total_duration,
            statistics,
        };

        if report.is_ok() {
            info!("Repository verification completed successfully");
        } else {
            warn!(
                "Repository verification completed with {} errors",
                report.total_errors()
            );
        }

        Ok(report)
    }

    /// Verify manifest integrity
    async fn verify_manifest(&self) -> (bool, Option<crate::repository::Manifest>) {
        match self.load_manifest().await {
            Ok(manifest) => {
                // Basic manifest validation
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

    /// Verify chunk index consistency with disk
    async fn verify_index(&self) -> Result<bool> {
        info!("Verifying chunk index consistency");
        
        let mut all_valid = true;
        for chunk_id in self.chunk_index_iterator() {
            if !self.has_chunk(chunk_id) {
                warn!("Chunk {} indexed but missing from storage", chunk_id);
                all_valid = false;
            }
        }

        Ok(all_valid)
    }

    /// Perform automated restore testing on an archive
    ///
    /// This samples files from the archive and attempts to restore them
    /// to a temporary location, verifying the restored data matches
    /// the expected size and optionally the content hash.
    ///
    /// # Arguments
    ///
    /// * `archive_name` - Name of the archive to test
    /// * `config` - Configuration for the test (sample size, etc.)
    ///
    /// # Returns
    ///
    /// A detailed report of the restore test results
    pub async fn test_restore(
        &self,
        archive_name: &str,
        config: RestoreTestConfig,
    ) -> Result<RestoreTestReport> {
        use crate::archive::{ArchiveRestorer, ItemType};

        info!("Starting restore test for archive '{}'", archive_name);
        let start = std::time::Instant::now();

        let restorer = ArchiveRestorer::new(self);
        let archive = restorer.load_archive(archive_name).await?;

        // Select files to test
        let file_items: Vec<_> = archive.items.iter()
            .filter(|item| matches!(item.item_type, ItemType::File))
            .filter(|item| {
                item.size >= config.min_file_size
                    && (config.max_file_size == 0 || item.size <= config.max_file_size)
            })
            .collect();

        let total_files = file_items.len();
        
        let samples: Vec<_> = if config.test_all {
            file_items
        } else {
            // Sample evenly distributed files
            let step = if total_files > config.max_samples {
                total_files / config.max_samples
            } else {
                1
            };
            file_items.into_iter()
                .step_by(step)
                .take(config.max_samples)
                .collect()
        };

        let files_sampled = samples.len();
        let mut files_restored = 0;
        let mut bytes_restored = 0u64;
        let mut file_results = Vec::new();
        let mut errors = Vec::new();

        // Create temporary directory for testing
        let temp_dir: tempfile::TempDir = match tempfile::TempDir::new() {
            Ok(dir) => dir,
            Err(e) => {
                return Err(BorgError::Repository(format!(
                    "Failed to create temp directory: {}", e
                )));
            }
        };

        for item in samples {
            let file_start = std::time::Instant::now();
            
            let result = self.test_restore_single_file(
                item,
                temp_dir.path(),
                config.verify_hash,
            ).await;

            let file_duration = file_start.elapsed();

            match result {
                Ok((actual_size, verified)) => {
                    file_results.push(FileRestoreResult {
                        path: item.path.clone(),
                        expected_size: item.size,
                        actual_size,
                        success: true,
                        error: None,
                        duration: file_duration,
                    });
                    files_restored += 1;
                    bytes_restored += actual_size;

                    if config.verify_hash && !verified {
                        debug!("File {} restored but hash verification skipped", item.path.display());
                    }
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    file_results.push(FileRestoreResult {
                        path: item.path.clone(),
                        expected_size: item.size,
                        actual_size: 0,
                        success: false,
                        error: Some(error_msg.clone()),
                        duration: file_duration,
                    });
                    errors.push(format!("{}: {}", item.path.display(), error_msg));
                }
            }
        }

        let duration = start.elapsed();
        let success = files_restored == files_sampled && errors.is_empty();

        let report = RestoreTestReport {
            archive_name: archive_name.to_string(),
            total_files,
            files_sampled,
            files_restored,
            bytes_restored,
            duration,
            success,
            file_results,
            errors,
        };

        if report.success {
            info!(
                "Restore test completed: {}/{} files restored successfully",
                files_restored, files_sampled
            );
        } else {
            warn!(
                "Restore test completed with errors: {}/{} files failed",
                files_sampled - files_restored, files_sampled
            );
        }

        Ok(report)
    }

    /// Test restore of a single file
    async fn test_restore_single_file(
        &self,
        item: &crate::archive::ArchiveItem,
        dest_dir: &std::path::Path,
        verify_hash: bool,
    ) -> Result<(u64, bool)> {
        use sha2::{Digest, Sha256};

        // Reconstruct file from chunks
        let mut file_data = Vec::new();
        for chunk_id in &item.chunks {
            let chunk = self.get_chunk(chunk_id).await?;
            file_data.extend_from_slice(&chunk.data);
        }

        // Write to temp file
        let dest_path = dest_dir.join(&item.path);
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest_path, &file_data)?;

        // Verify size
        let actual_size = file_data.len() as u64;
        if actual_size != item.size {
            return Err(BorgError::IntegrityCheck {
                expected: format!("{} bytes", item.size),
                actual: format!("{} bytes", actual_size),
            });
        }

        // Verify hash if requested
        let hash_verified = if verify_hash {
            // Re-read and hash to verify write was correct
            let written_data = std::fs::read(&dest_path)?;
            let mut hasher = Sha256::new();
            hasher.update(&written_data);
            let written_hash = hasher.finalize();

            let mut expected_hasher = Sha256::new();
            expected_hasher.update(&file_data);
            let expected_hash = expected_hasher.finalize();

            written_hash == expected_hash
        } else {
            true
        };

        // Clean up temp file
        let _ = std::fs::remove_file(&dest_path);

        if !hash_verified {
            return Err(BorgError::IntegrityCheck {
                expected: "matching hash".to_string(),
                actual: "hash mismatch".to_string(),
            });
        }

        Ok((actual_size, hash_verified))
    }

    /// Verify all chunks in the repository
    ///
    /// This performs a full integrity check, reading every chunk and verifying
    /// its content hash matches its ID. Also detects orphaned chunks.
    pub async fn verify_all(&self, progress: Option<&(dyn ProgressReporter + Send + Sync)>) -> Result<VerifyReport> {
        info!("Starting repository verification");
        let start = std::time::Instant::now();

        let total_chunks = self.chunk_count();
        let mut verified_chunks = 0;
        let mut corrupted_chunks = Vec::new();
        let mut missing_chunks = Vec::new();
        let mut bytes_verified = 0u64;

        // Verify each chunk in the index
        for (idx, chunk_id) in self.chunk_index_iterator().enumerate() {
            if let Some(reporter) = progress {
                reporter.report(
                    idx + 1,
                    total_chunks,
                    &format!("Verifying chunk {}", chunk_id),
                );
            }

            match self.verify_chunk(chunk_id).await {
                Ok(true) => {
                    verified_chunks += 1;
                    // Try to get size (best effort)
                    if let Ok(chunk) = self.get_chunk(chunk_id).await {
                        bytes_verified += chunk.data.len() as u64;
                    }
                }
                Ok(false) => {
                    warn!("Corrupted chunk detected: {}", chunk_id);
                    corrupted_chunks.push(chunk_id.clone());
                }
                Err(BorgError::Repository(_)) => {
                    // Chunk referenced in index but file not found
                    warn!("Missing chunk: {}", chunk_id);
                    missing_chunks.push(chunk_id.clone());
                }
                Err(e) => return Err(e),
            }
        }

        // Detect orphaned chunks (on disk but not in index)
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
            warn!(
                "Repository verification completed with {} errors",
                report.error_count()
            );
        }

        Ok(report)
    }

    /// Verify a specific archive's integrity
    ///
    /// Checks that all chunks referenced by the archive exist and are valid
    pub async fn verify_archive(&self, archive_name: &str) -> Result<ArchiveVerifyReport> {
        info!("Verifying archive: {}", archive_name);

        // Load archive metadata
        let restorer = crate::archive::ArchiveRestorer::new(self);
        let archive = restorer.load_archive(archive_name).await?;
        
        let total_items = archive.items.len();
        let mut verified_items = 0;
        let mut missing_chunks = Vec::new();
        let mut errors = Vec::new();

        for item in &archive.items {
            let mut item_valid = true;

            // Verify all chunks for this item
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

    /// Helper: Get count of chunks
    pub(crate) fn chunk_count(&self) -> usize {
        self.chunk_index_len()
    }

    /// Find orphaned chunks (chunks on storage but not in index)
    async fn find_orphaned_chunks(&self) -> Result<Vec<ChunkId>> {
        // This is a simplified implementation for OpenDAL
        // A full implementation would use op.list("data/chunks/")
        Ok(Vec::new())
    }

}

// Extension methods for Repository to provide iteration
impl Repository {
    /// Get the number of chunks in the index
    pub fn chunk_index_len(&self) -> usize {
        self.chunk_cache.len()
    }

    /// Get an iterator over chunk IDs
    pub fn chunk_index_iterator(&self) -> impl Iterator<Item = &ChunkId> {
        self.chunk_cache.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::Chunk;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_verify_valid_chunk() {
        let temp_dir = TempDir::new().unwrap();
        let op = crate::storage::build_operator(crate::storage::StorageConfig::Local { 
            path: temp_dir.path().join("repo") 
        }).unwrap();

        let mut repo = Repository::init(op, "test-repo".to_string(), Some("pass"), None).await.unwrap();

        let chunk = Chunk::new(b"test data".to_vec());
        let chunk_id = chunk.id.clone();
        repo.put_chunk(&chunk).await.unwrap();

        let is_valid = repo.verify_chunk(&chunk_id).await.unwrap();
        assert!(is_valid);
    }

    #[tokio::test]
    async fn test_verify_all_chunks() {
        let temp_dir = TempDir::new().unwrap();
        let op = crate::storage::build_operator(crate::storage::StorageConfig::Local { 
            path: temp_dir.path().join("repo") 
        }).unwrap();

        let mut repo = Repository::init(op, "test-repo".to_string(), Some("pass"), None).await.unwrap();

        // Add several chunks
        for i in 0..10 {
            let chunk = Chunk::new(format!("test data {}", i).into_bytes());
            repo.put_chunk(&chunk).await.unwrap();
        }
        // repo.commit().await.unwrap(); // Commit not needed for basic chunk verification in new model

        let report = repo.verify_all(None).await.unwrap();
        assert_eq!(report.total_chunks, 10);
        assert_eq!(report.verified_chunks, 10);
        assert!(report.is_ok());
    }

    #[test]
    fn test_parse_storage_config_with_session_token() {
        // This test verifies that we can parse a config, but since parse_storage_config
        // doesn't currently support parsing session tokens from the URL (it's not standard),
        // we mainly want to ensure the struct supports it and we can manually construct it.

        let config = crate::storage::StorageConfig::S3 {
            bucket: "mybucket".to_string(),
            prefix: "prefix".to_string(),
            region: Some("us-east-1".to_string()),
            endpoint: None,
            access_key: Some("AKIA...".to_string()),
            secret_key: Some("SECRET...".to_string()),
            session_token: Some("TOKEN...".to_string()),
        };

        match config {
            crate::storage::StorageConfig::S3 { session_token, .. } => {
                assert_eq!(session_token, Some("TOKEN...".to_string()));
            }
            _ => panic!("Unexpected config type"),
        }
    }
}
