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

impl Repository {
    /// Verify a single chunk's integrity
    ///
    /// Returns Ok(true) if chunk is valid, Ok(false) if corrupted
    pub fn verify_chunk(&self, id: &ChunkId) -> Result<bool> {
        match self.get_chunk(id) {
            Ok(chunk) => {
                // get_chunk already verifies hash internally
                Ok(chunk.id == *id)
            }
            Err(BorgError::IntegrityCheck { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Verify all chunks in the repository
    ///
    /// This performs a full integrity check, reading every chunk and verifying
    /// its content hash matches its ID. Also detects orphaned chunks.
    pub fn verify_all(&self, progress: Option<&dyn ProgressReporter>) -> Result<VerifyReport> {
        info!("Starting repository verification");
        let start = std::time::Instant::now();

        let total_chunks = self.chunk_count();
        let mut verified_chunks = 0;
        let mut corrupted_chunks = Vec::new();
        let mut missing_chunks = Vec::new();
        let mut bytes_verified = 0u64;

        // Verify each chunk in the index
        for (idx, chunk_id) in self.chunk_index_iter().enumerate() {
            if let Some(reporter) = progress {
                reporter.report(
                    idx + 1,
                    total_chunks,
                    &format!("Verifying chunk {}", chunk_id),
                );
            }

            match self.verify_chunk(chunk_id) {
                Ok(true) => {
                    verified_chunks += 1;
                    // Try to get size (best effort)
                    if let Ok(chunk) = self.get_chunk(chunk_id) {
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
        let orphaned_chunks = self.find_orphaned_chunks()?;
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
    pub fn verify_archive(&self, archive_name: &str) -> Result<ArchiveVerifyReport> {
        info!("Verifying archive: {}", archive_name);

        // Load archive metadata
        let archive = self.load_archive(archive_name)?;
        
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

    /// Helper: Iterate over chunk IDs in the index
    pub(crate) fn chunk_index_iter(&self) -> impl Iterator<Item = &ChunkId> {
        self.chunk_index_iterator()
    }

    /// Find orphaned chunks (chunks on disk but not in index)
    fn find_orphaned_chunks(&self) -> Result<Vec<ChunkId>> {
        let mut orphaned = Vec::new();
        let data_dir = self.path().join("data");

        // Walk through all chunk files
        for shard in 0..=255u8 {
            let shard_dir = data_dir.join(format!("{:02x}", shard));
            if !shard_dir.exists() {
                continue;
            }

            let entries = std::fs::read_dir(&shard_dir)?;
            for entry in entries {
                let entry = entry?;
                let file_name = entry.file_name();
                let file_name_str = file_name.to_string_lossy();

                // Reconstruct chunk ID from filename
                let hex_id = format!("{:02x}{}", shard, file_name_str);
                if let Ok(chunk_id) = ChunkId::from_hex(&hex_id) {
                    if !self.has_chunk(&chunk_id) {
                        orphaned.push(chunk_id);
                    }
                }
            }
        }

        Ok(orphaned)
    }

    /// Helper method to load an archive
    /// NOTE: This is a stub - will be implemented when we do Feature 2
    fn load_archive(&self, archive_name: &str) -> Result<crate::archive::Archive> {
        // For now, return a placeholder error
        // This will be properly implemented in the archive module enhancement
        Err(BorgError::Repository(format!(
            "Archive loading not yet implemented: {}",
            archive_name
        )))
    }
}

// Extension methods for Repository to provide iteration
impl Repository {
    /// Get the number of chunks in the index
    pub fn chunk_index_len(&self) -> usize {
        self.chunk_index.len()
    }

    /// Get an iterator over chunk IDs
    pub fn chunk_index_iterator(&self) -> impl Iterator<Item = &ChunkId> {
        self.chunk_index.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::Chunk;
    use tempfile::TempDir;

    #[test]
    fn test_verify_valid_chunk() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().join("test-repo");

        let mut repo = Repository::init(&repo_path, Some("pass"), None).unwrap();

        let chunk = Chunk::new(b"test data".to_vec());
        let chunk_id = chunk.id.clone();
        repo.put_chunk(&chunk).unwrap();

        let is_valid = repo.verify_chunk(&chunk_id).unwrap();
        assert!(is_valid);
    }

    #[test]
    fn test_verify_all_chunks() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().join("test-repo");

        let mut repo = Repository::init(&repo_path, Some("pass"), None).unwrap();

        // Add several chunks
        for i in 0..10 {
            let chunk = Chunk::new(format!("test data {}", i).into_bytes());
            repo.put_chunk(&chunk).unwrap();
        }
        repo.commit().unwrap();

        let report = repo.verify_all(None).unwrap();
        assert_eq!(report.total_chunks, 10);
        assert_eq!(report.verified_chunks, 10);
        assert!(report.is_ok());
    }
}
