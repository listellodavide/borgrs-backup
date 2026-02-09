use anyhow::Result;

use borg_core::repository::RepoDescriptor;
use borg_core::storage::StorageConfig;
use borg_core::storage::build_operator;
use borg_core::repository::Repository;
use borg_core::lock::RepositoryLock;

pub use borg_core::archive::{BackupProgress, RestoreProgress};
use borg_core::archive::{ArchiveCreator, ArchiveRestorer};
use borg_core::compression::CompressionConfig;
use borg_core::compression::CompressionAlgorithm;
use std::path::PathBuf;
use std::collections::HashMap;
use chrono::{Local, TimeZone};

pub fn generate_managed_archive_name() -> String {
    let now = Local::now();
    format!("managed-{}", now.format("%Y%m%d-%H%M%S"))
}

pub struct CreateArchiveResult {
    pub name: String,
    pub date: String,
    pub size: String,
    pub hostname: String,
    pub comment: String,
    pub tags: String,
}

fn human_bytes(size: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"]; 
    let mut s = size as f64;
    let mut idx = 0usize;
    while s >= 1024.0 && idx < UNITS.len() - 1 {
        s /= 1024.0;
        idx += 1;
    }
    if idx == 0 { format!("{} {}", size, UNITS[idx]) } else { format!("{:.1} {}", s, UNITS[idx]) }
}

pub async fn create_archive(
    repo_path: &str,
    repo_password: Option<&str>,
    archive_name: &str,
    paths: Vec<String>,
    compression: &str,
    comment: Option<String>,
    tags: Option<Vec<String>>,
    progress: Option<Box<dyn BackupProgress>>,
) -> Result<CreateArchiveResult> {
    let repo_path_buf = PathBuf::from(repo_path);
    let mut lock = RepositoryLock::new(&repo_path_buf);
    lock.acquire("create_archive", archive_name)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let storage = if repo_path.starts_with("/") || repo_path.contains(":\\") {
        StorageConfig::Local { path: PathBuf::from(repo_path) }
    } else {
         // Fallback/simplified: in a real app we'd load the bookmark's config
         StorageConfig::Local { path: PathBuf::from(repo_path) }
    };

    let op = build_operator(storage).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let mut repo = Repository::open(op, repo_path.to_string(), repo_password)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let path_bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    
    // Parse compression
    let comp_config = if compression.is_empty() || compression == "none" {
        CompressionConfig::default()
    } else {
        let parts: Vec<&str> = compression.split(',').collect();
        let algo = CompressionAlgorithm::from_str(parts[0]).unwrap_or(CompressionAlgorithm::Zstd);
        let level = if parts.len() > 1 {
            parts[1].parse().unwrap_or(3)
        } else {
            3
        };
        CompressionConfig {
            algorithm: algo,
            level: borg_core::compression::CompressionLevel::new(level).unwrap_or_default(),
            auto_detect: false,
            min_size: 1024,
        }
    };

    // Build path mapping (pointer to map of such paths)
    // For each source path, we map its final component name in the archive to its absolute path
    let mut mapping = HashMap::new();
    for p in &path_bufs {
        if let Some(name) = p.file_name() {
             mapping.insert(PathBuf::from(name), p.clone());
        }
    }

    let mut creator = ArchiveCreator::new(&mut repo)
        .with_compression(comp_config);
    
    if let Some(p) = progress {
        creator = creator.with_progress(p);
    }
    
    let archive = creator
        .create_with_mapping(archive_name, &path_bufs, Some(mapping), comment.clone(), tags.clone())
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // Build result summary
    let time_local = Local.from_utc_datetime(&archive.metadata.time.naive_utc());
    let date = time_local.format("%Y-%m-%d %H:%M").to_string();
    let size = human_bytes(archive.stats.original_size);
    let hostname = archive.metadata.hostname.clone();
    let comment_str = comment.unwrap_or_default();
    let tags_str = tags.map(|v| v.join(", ")).unwrap_or_default();

    Ok(CreateArchiveResult {
        name: archive.metadata.name,
        date,
        size,
        hostname,
        comment: comment_str,
        tags: tags_str,
    })
}

pub async fn list_archives(repo_path: &str) -> Result<Vec<String>> {
    // Placeholder for borg-core integration
    println!("Listing archives for repo: {}", repo_path);
    Ok(vec!["daily-2023-10-24".to_string(), "daily-2023-10-23".to_string()])
}

/// Initialize a repository using borg-core APIs based on wizard inputs.
///
/// Supported backends:
/// - local: `path_url` is an absolute filesystem path
/// - remote (experimental): if `path_url` starts with `s3://bucket/prefix`, it will use S3; otherwise,
///   it will attempt WebDAV with the given URL as endpoint and "/" as root.
pub async fn init_repository(
    repo_type: &str,
    _repo_name: &str,
    path_url: &str,
    access_key: Option<&str>,
    secret_key: Option<&str>,
    password: Option<&str>,
) -> Result<()> {
    // Build StorageConfig
    let storage = if repo_type == "local" {
        StorageConfig::Local { path: std::path::PathBuf::from(path_url) }
    } else {
        // Very small parser: support s3://bucket/prefix
        if let Some(rest) = path_url.strip_prefix("s3://") {
            let mut parts = rest.splitn(2, '/');
            let bucket = parts.next().unwrap_or("").to_string();
            let prefix = parts.next().unwrap_or("").to_string();
            StorageConfig::S3 {
                bucket,
                prefix,
                region: None,
                endpoint: None,
                access_key: access_key.map(str::to_string),
                secret_key: secret_key.map(str::to_string),
                session_token: None,
            }
        } else {
            // Fallback to WebDAV-like; treat full URL as endpoint, use root "/"
            StorageConfig::WebDav {
                endpoint: path_url.to_string(),
                root: "/".to_string(),
                username: access_key.map(str::to_string),
                password: secret_key.map(str::to_string),
            }
        }
    };

    let op = build_operator(storage).map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // Determine encryption based on password presence
    let mut desc = RepoDescriptor::default();
    desc.encrypted = password.is_some();

    // Note: Repository::init also writes the descriptor to storage
    let _repo = Repository::init(op, path_url.to_string(), password, Some(desc))
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    Ok(())
}


/// Restore an archive with optional original path mode and GUI progress.
pub async fn restore_archive(
    repo_path: &str,
    repo_password: Option<&str>,
    archive_name: &str,
    use_original_paths: bool,
    dest_path: Option<String>,
    progress: Option<Box<dyn RestoreProgress>>,
) -> Result<()> {
    let storage = if repo_path.starts_with("/") || repo_path.contains(":\\") {
        StorageConfig::Local { path: PathBuf::from(repo_path) }
    } else {
        StorageConfig::Local { path: PathBuf::from(repo_path) }
    };

    let op = build_operator(storage).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let repo = Repository::open(op, repo_path.to_string(), repo_password)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let restorer = if let Some(p) = progress {
        ArchiveRestorer::new(&repo).with_progress(p)
    } else {
        ArchiveRestorer::new(&repo)
    };

    if use_original_paths {
        // Restore using the same paths recorded during backup
        let dest = std::path::Path::new("/");
        let defaults = vec![PathBuf::from("::defaults")];
        let _stats = restorer
            .restore_paths(archive_name, dest, &defaults)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    } else {
        // Restore everything under chosen destination
        let dest = PathBuf::from(dest_path.unwrap_or_else(|| ".".to_string()));
        let _stats = restorer
            .restore(archive_name, &dest)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    }

    Ok(())
}
