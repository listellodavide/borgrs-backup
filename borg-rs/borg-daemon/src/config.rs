//! Daemon configuration management

use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Main daemon configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Global settings
    #[serde(default)]
    pub global: GlobalConfig,

    /// Backup jobs
    #[serde(default)]
    pub jobs: Vec<BackupJob>,

    /// Repository configurations
    #[serde(default)]
    pub repositories: Vec<RepositoryConfig>,
}

/// Global daemon settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    /// Maximum concurrent backup jobs
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_jobs: usize,

    /// Cache directory
    #[serde(default = "default_cache_dir")]
    pub cache_dir: PathBuf,

    /// Lock directory
    #[serde(default = "default_lock_dir")]
    pub lock_dir: PathBuf,

    /// Default compression algorithm
    #[serde(default = "default_compression")]
    pub default_compression: String,

    /// Default compression level (1-22 for zstd)
    #[serde(default = "default_compression_level")]
    pub default_compression_level: u32,

    /// Enable CPU nice value for backup processes
    #[serde(default)]
    pub nice_level: Option<i32>,

    /// Enable IO nice class/level
    #[serde(default)]
    pub ionice_class: Option<u32>,

    /// Metrics endpoint port (0 = disabled)
    #[serde(default)]
    pub metrics_port: u16,

    /// Health check interval in seconds
    #[serde(default = "default_health_interval")]
    pub health_check_interval: u64,
}

fn default_max_concurrent() -> usize { 1 }
fn default_cache_dir() -> PathBuf { PathBuf::from("/var/cache/borg-rust") }
fn default_lock_dir() -> PathBuf { PathBuf::from("/var/lock/borg-rust") }
fn default_compression() -> String { "zstd".to_string() }
fn default_compression_level() -> u32 { 3 }
fn default_health_interval() -> u64 { 60 }

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            max_concurrent_jobs: default_max_concurrent(),
            cache_dir: default_cache_dir(),
            lock_dir: default_lock_dir(),
            default_compression: default_compression(),
            default_compression_level: default_compression_level(),
            nice_level: None,
            ionice_class: None,
            metrics_port: 0,
            health_check_interval: default_health_interval(),
        }
    }
}

/// Backup job configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupJob {
    /// Unique job name
    pub name: String,

    /// Cron schedule expression (e.g., "0 2 * * *" for 2 AM daily)
    pub schedule: String,

    /// Repository name or path
    pub repository: String,

    /// Paths to back up
    pub paths: Vec<PathBuf>,

    /// Exclusion patterns
    #[serde(default)]
    pub exclude_patterns: Vec<String>,

    /// Exclusion files (like .borgignore)
    #[serde(default)]
    pub exclude_files: Vec<PathBuf>,

    /// Enable exclusion caches (CACHEDIR.TAG)
    #[serde(default = "default_true")]
    pub exclude_caches: bool,

    /// Exclude if present file patterns
    #[serde(default)]
    pub exclude_if_present: Vec<String>,

    /// Archive name template
    #[serde(default = "default_archive_name")]
    pub archive_name: String,

    /// Compression override (uses global if not set)
    pub compression: Option<String>,

    /// Compression level override
    pub compression_level: Option<u32>,

    /// One file system (don't cross mount points)
    #[serde(default)]
    pub one_file_system: bool,

    /// Read special files (device files, etc.)
    #[serde(default)]
    pub read_special: bool,

    /// Numeric owner IDs (don't store names)
    #[serde(default)]
    pub numeric_ids: bool,

    /// Don't store atime
    #[serde(default = "default_true")]
    pub noatime: bool,

    /// Exclude nodump files
    #[serde(default)]
    pub exclude_nodump: bool,

    /// Prune settings (if enabled)
    #[serde(default)]
    pub prune: Option<PruneConfig>,

    /// Pre-backup command
    #[serde(default)]
    pub pre_command: Option<String>,

    /// Post-backup command
    #[serde(default)]
    pub post_command: Option<String>,

    /// Notification settings
    #[serde(default)]
    pub notifications: NotificationConfig,

    /// Job priority (lower = higher priority)
    #[serde(default = "default_priority")]
    pub priority: u32,

    /// Whether job is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Retry configuration
    #[serde(default)]
    pub retry: RetryConfig,
}

fn default_true() -> bool { true }
fn default_priority() -> u32 { 100 }
fn default_archive_name() -> String { "{hostname}-{job}-{now:%Y-%m-%d_%H:%M:%S}".to_string() }

/// Prune configuration for automatic archive cleanup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneConfig {
    /// Keep all archives within this time span
    #[serde(default)]
    pub keep_within: Option<String>,

    /// Number of hourly archives to keep
    #[serde(default)]
    pub keep_hourly: Option<u32>,

    /// Number of daily archives to keep
    #[serde(default)]
    pub keep_daily: Option<u32>,

    /// Number of weekly archives to keep
    #[serde(default)]
    pub keep_weekly: Option<u32>,

    /// Number of monthly archives to keep
    #[serde(default)]
    pub keep_monthly: Option<u32>,

    /// Number of yearly archives to keep
    #[serde(default)]
    pub keep_yearly: Option<u32>,

    /// Prefix to match archives for pruning
    #[serde(default)]
    pub prefix: Option<String>,
}

/// Notification settings
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationConfig {
    /// Send notification on success
    #[serde(default)]
    pub on_success: bool,

    /// Send notification on failure
    #[serde(default = "default_true")]
    pub on_failure: bool,

    /// Notification command template
    #[serde(default)]
    pub command: Option<String>,

    /// Email address for notifications
    #[serde(default)]
    pub email: Option<String>,

    /// Webhook URL for notifications
    #[serde(default)]
    pub webhook_url: Option<String>,
}

/// Retry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum retry attempts
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Initial delay between retries (seconds)
    #[serde(default = "default_retry_delay")]
    pub initial_delay: u64,

    /// Maximum delay between retries (seconds)
    #[serde(default = "default_max_retry_delay")]
    pub max_delay: u64,

    /// Exponential backoff multiplier
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
}

fn default_max_retries() -> u32 { 3 }
fn default_retry_delay() -> u64 { 60 }
fn default_max_retry_delay() -> u64 { 3600 }
fn default_backoff_multiplier() -> f64 { 2.0 }

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            initial_delay: default_retry_delay(),
            max_delay: default_max_retry_delay(),
            backoff_multiplier: default_backoff_multiplier(),
        }
    }
}

/// Repository configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryConfig {
    /// Repository name (for reference in jobs)
    pub name: String,

    /// Repository path (local or remote)
    pub path: String,

    /// Encryption passphrase (or reference to env var/file)
    #[serde(default)]
    pub passphrase: Option<PassphraseConfig>,

    /// SSH key path for remote repositories
    #[serde(default)]
    pub ssh_key: Option<PathBuf>,

    /// SSH options
    #[serde(default)]
    pub ssh_options: Option<String>,

    /// Remote rate limit (KB/s, 0 = unlimited)
    #[serde(default)]
    pub remote_rate_limit: u64,

    /// Repository check interval (days, 0 = disabled)
    #[serde(default)]
    pub check_interval_days: u32,
}

/// Passphrase configuration options
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PassphraseConfig {
    /// Direct passphrase (not recommended)
    #[serde(rename = "direct")]
    Direct { value: String },

    /// Environment variable
    #[serde(rename = "env")]
    Environment { var: String },

    /// File containing passphrase
    #[serde(rename = "file")]
    File { path: PathBuf },

    /// systemd credential
    #[serde(rename = "credential")]
    Credential { name: String },

    /// External command
    #[serde(rename = "command")]
    Command { cmd: String },
}

impl DaemonConfig {
    /// Load configuration from a YAML file
    pub async fn load(path: &Path) -> Result<Self> {
        let content = tokio::fs::read_to_string(path)
            .await
            .context("Failed to read configuration file")?;

        let config: DaemonConfig = serde_yaml::from_str(&content)
            .context("Failed to parse configuration file")?;

        config.validate()?;

        Ok(config)
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        // Validate jobs
        for job in &self.jobs {
            // Check cron expression is valid
            job.schedule.parse::<cron::Schedule>()
                .map_err(|e| anyhow::anyhow!("Invalid cron schedule for job '{}': {}", job.name, e))?;

            // Check paths exist
            for path in &job.paths {
                if !path.exists() {
                    tracing::warn!("Path does not exist for job '{}': {:?}", job.name, path);
                }
            }

            // Validate repository reference
            if !job.repository.starts_with('/') && !job.repository.contains(':') {
                // It's a named repository reference
                let repo_exists = self.repositories.iter()
                    .any(|r| r.name == job.repository);
                if !repo_exists {
                    anyhow::bail!("Job '{}' references unknown repository: {}", 
                        job.name, job.repository);
                }
            }
        }

        // Check for duplicate job names
        let mut seen_names = std::collections::HashSet::new();
        for job in &self.jobs {
            if !seen_names.insert(&job.name) {
                anyhow::bail!("Duplicate job name: {}", job.name);
            }
        }

        // Check for duplicate repository names
        seen_names.clear();
        for repo in &self.repositories {
            if !seen_names.insert(&repo.name) {
                anyhow::bail!("Duplicate repository name: {}", repo.name);
            }
        }

        Ok(())
    }

    /// Get repository configuration by name or path
    pub fn get_repository(&self, name_or_path: &str) -> Option<&RepositoryConfig> {
        self.repositories.iter().find(|r| r.name == name_or_path)
    }

    /// Reload configuration from file
    pub async fn reload(&mut self, path: &Path) -> Result<()> {
        let new_config = Self::load(path).await?;
        *self = new_config;
        Ok(())
    }
}

impl PassphraseConfig {
    /// Resolve the passphrase to its actual value
    pub async fn resolve(&self) -> Result<String> {
        match self {
            PassphraseConfig::Direct { value } => Ok(value.clone()),
            
            PassphraseConfig::Environment { var } => {
                std::env::var(var)
                    .context(format!("Environment variable {} not set", var))
            }
            
            PassphraseConfig::File { path } => {
                tokio::fs::read_to_string(path)
                    .await
                    .map(|s| s.trim().to_string())
                    .context(format!("Failed to read passphrase file: {:?}", path))
            }
            
            PassphraseConfig::Credential { name } => {
                // systemd credentials are exposed via files in $CREDENTIALS_DIRECTORY
                let cred_dir = std::env::var("CREDENTIALS_DIRECTORY")
                    .context("CREDENTIALS_DIRECTORY not set")?;
                let cred_path = PathBuf::from(cred_dir).join(name);
                tokio::fs::read_to_string(&cred_path)
                    .await
                    .map(|s| s.trim().to_string())
                    .context(format!("Failed to read credential: {}", name))
            }
            
            PassphraseConfig::Command { cmd } => {
                let output = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(cmd)
                    .output()
                    .await
                    .context("Failed to execute passphrase command")?;
                
                if !output.status.success() {
                    anyhow::bail!("Passphrase command failed with status: {}", output.status);
                }
                
                String::from_utf8(output.stdout)
                    .map(|s| s.trim().to_string())
                    .context("Passphrase command output is not valid UTF-8")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = GlobalConfig::default();
        assert_eq!(config.max_concurrent_jobs, 1);
        assert_eq!(config.default_compression, "zstd");
    }

    #[tokio::test]
    async fn test_passphrase_env() {
        std::env::set_var("TEST_BORG_PASS", "secret123");
        let config = PassphraseConfig::Environment { 
            var: "TEST_BORG_PASS".to_string() 
        };
        let resolved = config.resolve().await.unwrap();
        assert_eq!(resolved, "secret123");
        std::env::remove_var("TEST_BORG_PASS");
    }
}
