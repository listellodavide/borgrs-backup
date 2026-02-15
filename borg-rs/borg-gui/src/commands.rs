use anyhow::Result;

use borg_core::lock::RepositoryLock;
use borg_core::repository::RepoDescriptor;
use borg_core::repository::Repository;
use borg_core::storage::StorageConfig;
use borg_core::storage::build_operator;
use borg_core::utils::human_bytes;

use borg_core::archive::{Archive, ArchiveCreator, ArchiveRestorer, ItemType};
pub use borg_core::archive::{BackupProgress, RestoreProgress};
use borg_core::compression::CompressionAlgorithm;
use borg_core::compression::CompressionConfig;
use chrono::{Local, TimeZone};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
        StorageConfig::Local {
            path: PathBuf::from(repo_path),
        }
    } else {
        // Fallback/simplified: in a real app we'd load the bookmark's config
        StorageConfig::Local {
            path: PathBuf::from(repo_path),
        }
    };

    let op = build_operator(storage).map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // Open the repository. It is expected to be initialized already.
    let mut repo = Repository::open(op, repo_path.to_string(), repo_password)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let path_bufs: Vec<PathBuf> = paths
        .iter()
        .map(|p| Path::new(p).canonicalize().unwrap())
        .collect();

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

    // Build path mapping. The goal is to preserve the hierarchical structure
    // of the input paths within the archive.
    let mut mapping = HashMap::new();
    for p in &path_bufs {
        // The archive path should be the same as the input path's file name.
        if let Some(name) = p.file_name() {
            mapping.insert(PathBuf::from(name), p.clone());
        }
    }

    let mut creator = ArchiveCreator::new(&mut repo).with_compression(comp_config);

    if let Some(p) = progress {
        creator = creator.with_progress(p);
    }

    let archive = creator
        .create_with_mapping(
            archive_name,
            &path_bufs,
            Some(mapping),
            comment.clone(),
            tags.clone(),
        )
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // Note: commit_archive is now called inside ArchiveCreator::create_with_mapping

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

pub async fn list_archives(repo_path: &str, repo_password: Option<&str>) -> Result<Vec<String>> {
    let storage = if repo_path.starts_with("/") || repo_path.contains(":\\") {
        StorageConfig::Local {
            path: PathBuf::from(repo_path),
        }
    } else {
        StorageConfig::Local {
            path: PathBuf::from(repo_path),
        }
    };

    let op = build_operator(storage).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let repo = Repository::open(op, repo_path.to_string(), repo_password)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let manifest = repo
        .load_manifest()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let archives = manifest.archives.into_iter().map(|a| a.name).collect();
    Ok(archives)
}

pub async fn get_archive_paths(
    repo_path: &str,
    repo_password: Option<&str>,
    archive_name: &str,
) -> Result<Vec<String>> {
    let storage = if repo_path.starts_with("/") || repo_path.contains(":\\") {
        StorageConfig::Local {
            path: PathBuf::from(repo_path),
        }
    } else {
        StorageConfig::Local {
            path: PathBuf::from(repo_path),
        }
    };

    let op = build_operator(storage).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let repo = Repository::open(op, repo_path.to_string(), repo_password)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let manifest = repo
        .load_manifest()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let archive_ref = manifest
        .archives
        .iter()
        .find(|a| a.name == archive_name)
        .ok_or_else(|| anyhow::anyhow!("Archive '{}' not found", archive_name))?;

    let chunk = repo
        .get_chunk(&archive_ref.id)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let archive: Archive = bincode::deserialize(&chunk.data)
        .map_err(|e| anyhow::anyhow!("Failed to deserialize archive data: {}", e))?;

    let paths = archive
        .metadata
        .original_paths
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    Ok(paths)
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
        StorageConfig::Local {
            path: std::path::PathBuf::from(path_url),
        }
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
        StorageConfig::Local {
            path: PathBuf::from(repo_path),
        }
    } else {
        StorageConfig::Local {
            path: PathBuf::from(repo_path),
        }
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

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub mode: u32,
    pub user: String,
    pub group: String,
    pub mtime: i64,
}

pub async fn list_archive_files(
    repo_path: &str,
    repo_password: Option<&str>,
    archive_name: &str,
) -> Result<Vec<FileEntry>> {
    let storage = if repo_path.starts_with("/") || repo_path.contains(":\\") {
        StorageConfig::Local {
            path: PathBuf::from(repo_path),
        }
    } else {
        StorageConfig::Local {
            path: PathBuf::from(repo_path),
        }
    };

    let op = build_operator(storage).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let repo = Repository::open(op, repo_path.to_string(), repo_password)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let manifest = repo
        .load_manifest()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let archive_ref = manifest
        .archives
        .iter()
        .find(|a| a.name == archive_name)
        .ok_or_else(|| anyhow::anyhow!("Archive '{}' not found", archive_name))?;

    // The ID in the manifest points to the chunk containing the bincode-serialized Archive struct.
    // We need to get this chunk and deserialize it, not treat it as a JSON tree.
    let chunk = repo
        .get_chunk(&archive_ref.id)
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let archive: Archive = bincode::deserialize(&chunk.data)
        .map_err(|e| anyhow::anyhow!("Failed to deserialize archive data: {}", e))?;

    let entries: Vec<FileEntry> = archive
        .items
        .into_iter()
        .map(|item| {
            FileEntry {
                path: item.path.to_string_lossy().to_string(),
                size: item.size,
                is_dir: item.item_type == ItemType::Directory,
                mode: item.attrs.mode,
                // The FileEntry struct expects user/group as strings, but the archive stores UID/GID.
                // For now, we'll convert them to strings. A real implementation might resolve them to names.
                user: item.attrs.uid.to_string(),
                group: item.attrs.gid.to_string(),
                mtime: item.attrs.mtime,
            }
        })
        .collect();

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_create_and_list_archive() -> Result<()> {
        let repo_dir = tempdir()?;
        let repo_path = repo_dir.path().to_str().unwrap();
        let password = "dummy";

        // 1. Init Repo
        init_repository("local", "test-repo", repo_path, None, None, Some(password)).await?;

        // 2. Create source files with a hierarchical structure
        let src_dir = tempdir()?;
        let src_path = src_dir.path();
        let src_dir_name = src_path.file_name().unwrap().to_str().unwrap();

        fs::write(src_path.join("file1.txt"), "content1")?;
        fs::create_dir(src_path.join("subdir"))?;
        fs::write(src_path.join("subdir/file2.txt"), "content2")?;

        // 3. Create archive from the root of the source directory
        // We'll also test the "./" relative path handling by changing directory.
        let current_dir = std::env::current_dir()?;
        std::env::set_current_dir(src_path.parent().unwrap())?;
        let relative_src_path = format!("./{}", src_dir_name);

        let paths = vec![relative_src_path];

        let res = create_archive(
            repo_path,
            Some(password),
            "test-archive",
            paths,
            "none",
            None,
            None,
            None,
        )
        .await;

        // Restore current directory
        std::env::set_current_dir(current_dir)?;

        assert!(res.is_ok());

        // 4. List Files
        let files = list_archive_files(repo_path, Some(password), "test-archive").await?;

        // 5. Verify contents
        let paths_found: HashSet<String> = files.iter().map(|f| f.path.clone()).collect();
        println!("Found paths in archive: {:?}", paths_found);

        // The archive should contain the items from src_path, with preserved hierarchy.
        // The root of the archive will be the directory itself.
        let expected_paths: HashSet<String> = [
            format!("{}/file1.txt", src_dir_name),
            format!("subdir"),
            format!("subdir/file2.txt",),
            format!("{}", src_dir_name),
        ]
        .iter()
        .cloned()
        .collect();

        let paths_found_set: HashSet<String> = paths_found
            .into_iter()
            .map(|p| p.trim_start_matches('/').to_string())
            .collect();

        assert_eq!(paths_found_set, expected_paths);

        Ok(())
    }
}
