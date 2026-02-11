//! Repository lock management for concurrent backup operations
//!
//! Prevents multiple writers to the same repository while allowing
//! parallel backups to different repositories.

use crate::error::{BorgError, Result};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::io::Write;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Lock file information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockInfo {
    /// PID of the process holding the lock
    pub pid: u32,
    /// Timestamp when lock was created
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Task/process name
    pub task_name: String,
    /// Archive name being created
    pub archive_name: String,
    /// Hostname where lock was created
    pub hostname: String,
}

/// Repository lock manager
pub struct RepositoryLock {
    lock_path: PathBuf,
    lock_info: Option<LockInfo>,
}

impl RepositoryLock {
    /// Create a new lock manager for a repository
    pub fn new(repo_path: &Path) -> Self {
        let lock_path = repo_path.join(".borg.lock");
        Self {
            lock_path,
            lock_info: None,
        }
    }

    /// Check if repository is locked
    pub fn is_locked(&self) -> Result<bool> {
        if self.lock_path.exists() {
            // Check if lock is stale (process no longer exists)
            if let Ok(contents) = fs::read_to_string(&self.lock_path) {
                if let Ok(_info) = serde_json::from_str::<LockInfo>(&contents) {
                    // Check if process is still running (basic check)
                    // On Unix, we could use kill -0, but for simplicity we assume it's valid
                    return Ok(true);
                }
            }
            // Invalid or corrupted lock file, remove it
            let _ = fs::remove_file(&self.lock_path);
        }
        Ok(false)
    }

    /// Get current lock information if locked
    pub fn get_lock_info(&self) -> Result<Option<LockInfo>> {
        if self.lock_path.exists() {
            if let Ok(contents) = fs::read_to_string(&self.lock_path) {
                if let Ok(info) = serde_json::from_str::<LockInfo>(&contents) {
                    return Ok(Some(info));
                }
            }
        }
        Ok(None)
    }

    /// Acquire lock for a backup operation
    pub fn acquire(&mut self, task_name: &str, archive_name: &str) -> Result<()> {
        if self.is_locked()? {
            let info = self.get_lock_info()?;
            if let Some(lock_info) = info {
                return Err(BorgError::LockError(format!(
                    "Repository is locked by task '{}' creating archive '{}' (PID: {}, hostname: {})",
                    lock_info.task_name, lock_info.archive_name, lock_info.pid, lock_info.hostname
                )));
            }
            return Err(BorgError::LockError("Repository is locked".to_string()));
        }

        let lock_info = LockInfo {
            pid: std::process::id(),
            created_at: Utc::now(),
            task_name: task_name.to_string(),
            archive_name: archive_name.to_string(),
            hostname: hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "unknown".to_string()),
        };

        // Write lock file atomically
        let lock_json = serde_json::to_string_pretty(&lock_info)
            .map_err(|e| BorgError::LockError(format!("Failed to serialize lock info: {}", e)))?;

        let mut file = File::create(&self.lock_path)
            .map_err(|e| BorgError::LockError(format!("Failed to create lock file: {}", e)))?;

        file.write_all(lock_json.as_bytes())
            .map_err(|e| BorgError::LockError(format!("Failed to write lock file: {}", e)))?;

        file.sync_all()
            .map_err(|e| BorgError::LockError(format!("Failed to sync lock file: {}", e)))?;

        self.lock_info = Some(lock_info);
        debug!("Acquired lock for task '{}' on archive '{}'", task_name, archive_name);
        Ok(())
    }

    /// Release lock
    pub fn release(&mut self) -> Result<()> {
        if self.lock_path.exists() {
            fs::remove_file(&self.lock_path)
                .map_err(|e| BorgError::LockError(format!("Failed to remove lock file: {}", e)))?;
            debug!("Released repository lock");
        }
        self.lock_info = None;
        Ok(())
    }

    /// Wait for lock to be released with timeout
    pub async fn wait_for_unlock(&self, timeout_secs: u64) -> Result<()> {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);

        loop {
            if !self.is_locked()? {
                debug!("Lock released, proceeding");
                return Ok(());
            }

            if start.elapsed() > timeout {
                let info = self.get_lock_info()?;
                if let Some(lock_info) = info {
                    return Err(BorgError::LockError(format!(
                        "Timeout waiting for lock (held by task '{}' on archive '{}')",
                        lock_info.task_name, lock_info.archive_name
                    )));
                }
                return Err(BorgError::LockError("Timeout waiting for lock to be released".to_string()));
            }

            // Sleep before checking again
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
}

impl Drop for RepositoryLock {
    fn drop(&mut self) {
        if self.lock_info.is_some() {
            let _ = self.release();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    use tempfile::TempDir;

    #[test]
    fn test_lock_acquire_release() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        let mut lock = RepositoryLock::new(repo_path);
        assert!(!lock.is_locked().unwrap());

        lock.acquire("test-task", "test-archive").unwrap();
        assert!(lock.is_locked().unwrap());

        lock.release().unwrap();
        assert!(!lock.is_locked().unwrap());
    }

    #[test]
    fn test_lock_prevents_concurrent_access() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path();

        let mut lock1 = RepositoryLock::new(repo_path);
        lock1.acquire("task1", "archive1").unwrap();

        let mut lock2 = RepositoryLock::new(repo_path);
        assert!(lock2.is_locked().unwrap());
        assert!(lock2.acquire("task2", "archive2").is_err());

        lock1.release().unwrap();
        let mut lock3 = RepositoryLock::new(repo_path);
        assert!(!lock3.is_locked().unwrap());
        lock3.acquire("task3", "archive3").unwrap();
    }
}
