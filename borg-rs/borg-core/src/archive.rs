use crate::chunker::{ChunkId, Chunker, ChunkerProfile, choose_profile_for_size};
use crate::compression::CompressionConfig;
use crate::error::{BorgError, Result};
use crate::exclusion::ExclusionList;
use crate::repository::Repository;
use chrono::{DateTime, Utc};
use rand::seq::SliceRandom;
use rand::thread_rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use tracing::{debug, info, instrument, warn};
use walkdir::WalkDir;

/// Item type in an archive
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemType {
    /// Regular file
    File,
    /// Directory
    Directory,
    /// Symbolic link
    Symlink,
    /// Hard link
    Hardlink,
    /// Block device
    BlockDevice,
    /// Character device
    CharDevice,
    /// FIFO/named pipe
    Fifo,
    /// Socket
    Socket,
}

impl ItemType {
    /// Determine item type from filesystem metadata
    pub fn from_metadata(meta: &Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            let ft = meta.file_type();
            if ft.is_file() {
                ItemType::File
            } else if ft.is_dir() {
                ItemType::Directory
            } else if ft.is_symlink() {
                ItemType::Symlink
            } else if ft.is_block_device() {
                ItemType::BlockDevice
            } else if ft.is_char_device() {
                ItemType::CharDevice
            } else if ft.is_fifo() {
                ItemType::Fifo
            } else if ft.is_socket() {
                ItemType::Socket
            } else {
                ItemType::File // Default fallback
            }
        }
        #[cfg(not(unix))]
        {
            if meta.is_file() {
                ItemType::File
            } else if meta.is_dir() {
                ItemType::Directory
            } else {
                ItemType::File
            }
        }
    }
}

/// Unix file permissions and ownership
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnixAttributes {
    /// File mode (permissions)
    pub mode: u32,
    /// Owner user ID
    pub uid: u32,
    /// Owner group ID
    pub gid: u32,
    /// Access time
    pub atime: i64,
    /// Modification time
    pub mtime: i64,
    /// Change time
    pub ctime: i64,
}

impl UnixAttributes {
    /// Create from filesystem metadata
    pub fn from_metadata(meta: &Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            Self {
                mode: meta.permissions().mode(),
                uid: meta.uid(),
                gid: meta.gid(),
                atime: meta.atime(),
                mtime: meta.mtime(),
                ctime: meta.ctime(),
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                mode: 0o644,
                uid: 0,
                gid: 0,
                atime: 0,
                mtime: 0,
                ctime: 0,
            }
        }
    }
}

/// An item (file/directory) in an archive
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveItem {
    /// Path relative to archive root
    pub path: PathBuf,
    /// Item type
    pub item_type: ItemType,
    /// File size (0 for directories)
    pub size: u64,
    /// Unix attributes
    pub attrs: UnixAttributes,
    /// Chunk IDs for file content (empty for directories)
    pub chunks: Vec<ChunkId>,
    /// Symlink target (for symlinks only)
    pub symlink_target: Option<PathBuf>,
    /// Hard link target path (for hardlinks only)
    pub hardlink_target: Option<PathBuf>,
    /// Chunker profile used for this file
    #[serde(default = "default_chunker_profile")]
    pub chunker_profile: ChunkerProfile,
}

fn default_chunker_profile() -> ChunkerProfile {
    ChunkerProfile::Size4M // For backward compatibility
}

impl ArchiveItem {
    /// Create a new file item
    pub fn file(
        path: PathBuf,
        meta: &Metadata,
        chunks: Vec<ChunkId>,
        profile: ChunkerProfile,
    ) -> Self {
        Self {
            path,
            item_type: ItemType::File,
            size: meta.len(),
            attrs: UnixAttributes::from_metadata(meta),
            chunks,
            symlink_target: None,
            hardlink_target: None,
            chunker_profile: profile,
        }
    }

    /// Create a new directory item
    pub fn directory(path: PathBuf, meta: &Metadata) -> Self {
        Self {
            path,
            item_type: ItemType::Directory,
            size: 0,
            attrs: UnixAttributes::from_metadata(meta),
            chunks: Vec::new(),
            symlink_target: None,
            hardlink_target: None,
            chunker_profile: ChunkerProfile::Default, // Not applicable
        }
    }

    /// Create a new symlink item
    pub fn symlink(path: PathBuf, meta: &Metadata, target: PathBuf) -> Self {
        Self {
            path,
            item_type: ItemType::Symlink,
            size: 0,
            attrs: UnixAttributes::from_metadata(meta),
            chunks: Vec::new(),
            symlink_target: Some(target),
            hardlink_target: None,
            chunker_profile: ChunkerProfile::Default, // Not applicable
        }
    }
}

/// Archive metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveMetadata {
    /// Archive name
    pub name: String,
    /// Creation timestamp
    pub time: DateTime<Utc>,
    /// Hostname where the backup was created
    pub hostname: String,
    /// Username who created the backup
    pub username: String,
    /// Command line used to create the backup
    pub cmdline: Vec<String>,
    /// Comment (optional)
    pub comment: Option<String>,
    /// Tags (optional)
    pub tags: Option<Vec<String>>,
    /// Original paths backed up
    pub original_paths: Option<Vec<PathBuf>>,
    /// Path mapping for restoration (target -> source_root)
    pub path_mapping: Option<HashMap<PathBuf, PathBuf>>,
    /// Compression configuration used for this archive
    pub compression: Option<CompressionConfig>,
}

/// A complete archive containing items and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Archive {
    /// Archive metadata
    pub metadata: ArchiveMetadata,
    /// All items in the archive
    pub items: Vec<ArchiveItem>,
    /// Statistics
    pub stats: ArchiveStats,
}

/// Statistics for an archive
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArchiveStats {
    /// Total number of files
    pub nfiles: u64,
    /// Total number of directories
    pub ndirs: u64,
    /// Total original size
    pub original_size: u64,
    /// Total compressed size
    pub compressed_size: u64,
    /// Total deduplicated size (unique data)
    pub deduplicated_size: u64,
    /// Number of chunks
    pub nchunks: u64,
    /// Number of unique chunks (new data)
    pub nchunks_unique: u64,
}

/// Progress callback for backup operations
pub trait BackupProgress: Send + Sync {
    /// Called when starting to process a file
    fn on_file_start(&self, path: &Path);
    /// Called when a file is completed
    fn on_file_complete(&self, path: &Path, size: u64, chunks: usize);
    /// Called when a file is skipped (excluded)
    fn on_file_skipped(&self, path: &Path, reason: &str);
    /// Called periodically with overall progress
    fn on_progress(&self, processed: u64, total: u64, filename: Option<&str>);
    /// Called when an error occurs (non-fatal)
    fn on_error(&self, path: &Path, error: &str);
}

/// Null progress reporter (does nothing)
pub struct NullProgress;
impl BackupProgress for NullProgress {
    fn on_file_start(&self, _path: &Path) {}
    fn on_file_complete(&self, _path: &Path, _size: u64, _chunks: usize) {}
    fn on_file_skipped(&self, _path: &Path, _reason: &str) {}
    fn on_progress(&self, _processed: u64, _total: u64, _filename: Option<&str>) {}
    fn on_error(&self, _path: &Path, _error: &str) {}
}

/// Archive creator for building new archives
pub struct ArchiveCreator<'a> {
    /// Repository to store data
    repo: &'a mut Repository,
    /// Forced chunker profile, if any
    forced_chunker_profile: Option<ChunkerProfile>,
    /// Exclusion list
    exclusions: ExclusionList,
    /// Progress reporter
    progress: Box<dyn BackupProgress>,
    /// Hard link tracking (inode -> first path)
    hardlinks: HashMap<u64, PathBuf>,
    /// Compression configuration
    compression_config: Option<CompressionConfig>,
}

impl<'a> ArchiveCreator<'a> {
    /// Create a new archive creator
    pub fn new(repo: &'a mut Repository) -> Self {
        Self {
            repo,
            forced_chunker_profile: None,
            exclusions: ExclusionList::new(),
            progress: Box::new(NullProgress),
            hardlinks: HashMap::new(),
            compression_config: None,
        }
    }

    /// Set a forced chunker profile
    pub fn with_forced_chunker_profile(mut self, profile: ChunkerProfile) -> Self {
        self.forced_chunker_profile = Some(profile);
        self
    }

    /// Set exclusion list
    pub fn with_exclusions(mut self, exclusions: ExclusionList) -> Self {
        self.exclusions = exclusions;
        self
    }

    /// Set progress reporter
    pub fn with_progress(mut self, progress: Box<dyn BackupProgress>) -> Self {
        self.progress = progress;
        self
    }

    /// Set compression configuration
    pub fn with_compression(mut self, config: CompressionConfig) -> Self {
        self.compression_config = Some(config);
        self
    }

    /// Create an archive from a list of paths
    #[instrument(skip(self, paths, name))]
    pub async fn create(
        self,
        name: &str,
        paths: &[PathBuf],
        comment: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<Archive> {
        self.create_with_mapping(name, paths, None, comment, tags)
            .await
    }

    /// Create an archive from a list of paths with optional mapping
    #[instrument(skip(self, paths, name, mapping))]
    pub async fn create_with_mapping(
        mut self,
        name: &str,
        paths: &[PathBuf],
        mapping: Option<HashMap<PathBuf, PathBuf>>,
        comment: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<Archive> {
        info!("Creating archive '{}' from {} paths", name, paths.len());

        // Check for existing archive with same name
        let manifest = self.repo.load_manifest().await?;
        if manifest.archives.iter().any(|a| a.name == name) {
            return Err(BorgError::ArchiveExists {
                name: name.to_string(),
            });
        }

        // Configure repository compressor if compression config is provided
        if let Some(config) = &self.compression_config {
            use crate::compression::Compressor;
            self.repo.set_compressor(Compressor::new(config.clone()));
        }

        let metadata = ArchiveMetadata {
            name: name.to_string(),
            time: Utc::now(),
            hostname: gethostname(),
            username: get_username(),
            cmdline: std::env::args().collect(),
            comment,
            tags,
            original_paths: Some(paths.to_vec()),
            path_mapping: mapping,
            compression: self.compression_config.clone(),
        };

        let mut items = Vec::new();
        let mut stats = ArchiveStats::default();

        // Calculate total size for progress reporting
        let mut total_size = 0;
        for path in paths {
            for entry in WalkDir::new(path)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let relative_path = entry.path().strip_prefix(path).unwrap_or(entry.path());
                if !self
                    .exclusions
                    .is_excluded(relative_path, entry.file_type().is_dir())
                {
                    if let Ok(meta) = entry.metadata() {
                        if meta.is_file() {
                            total_size += meta.len();
                        }
                    }
                }
            }
        }
        self.progress.on_progress(0, total_size, None);

        // Process each path
        for path in paths {
            self.process_path(path, path, &mut items, &mut stats, total_size)
                .await?;
        }

        let archive = Archive {
            metadata,
            items,
            stats,
        };

        // Commit the archive using the new repository method
        self.repo.commit_archive(archive.clone()).await?;

        info!(
            "Archive '{}' created: {} files, {} dirs, {} bytes",
            name, archive.stats.nfiles, archive.stats.ndirs, archive.stats.original_size
        );

        Ok(archive)
    }

    /// Process a single path (file or directory)
    async fn process_path(
        &mut self,
        root: &Path,
        path: &Path,
        items: &mut Vec<ArchiveItem>,
        stats: &mut ArchiveStats,
        total_size: u64,
    ) -> Result<()> {
        // Walk the directory tree and collect entries that are not excluded
        let entries: Vec<_> = WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let relative_path = e.path().strip_prefix(root).unwrap_or(e.path());
                let is_excluded = self
                    .exclusions
                    .is_excluded(relative_path, e.file_type().is_dir());
                if is_excluded {
                    self.progress.on_file_skipped(e.path(), "excluded");
                    false
                } else {
                    true
                }
            })
            .filter_map(|e| match e {
                Ok(entry) => Some(Ok(entry)),
                Err(e) => Some(Err(e)),
            })
            .collect();

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    let error_path = e.path().unwrap_or(path);
                    warn!("Error accessing {}: {}", error_path.display(), e);
                    self.progress.on_error(error_path, &e.to_string());
                    continue;
                }
            };

            let entry_path = entry.path();
            let relative_path = entry_path
                .strip_prefix(root)
                .unwrap_or(entry_path)
                .to_path_buf();

            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    warn!("Cannot read metadata for {}: {}", entry_path.display(), e);
                    self.progress.on_error(entry_path, &e.to_string());
                    continue;
                }
            };

            let item = self
                .process_entry(entry_path, relative_path, &meta, stats, total_size)
                .await?;
            if let Some(item) = item {
                items.push(item);
            }
        }

        Ok(())
    }

    /// Process a single filesystem entry
    async fn process_entry(
        &mut self,
        path: &Path,
        relative_path: PathBuf,
        meta: &Metadata,
        stats: &mut ArchiveStats,
        total_size: u64,
    ) -> Result<Option<ArchiveItem>> {
        let item_type = ItemType::from_metadata(meta);

        match item_type {
            ItemType::Directory => {
                stats.ndirs += 1;
                Ok(Some(ArchiveItem::directory(relative_path, meta)))
            }
            ItemType::File => {
                self.progress.on_file_start(path);

                // Check for hard links
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    let inode = meta.ino();
                    if meta.nlink() > 1 {
                        if let Some(first_path) = self.hardlinks.get(&inode) {
                            // This is a hard link to an already-seen file
                            stats.nfiles += 1;
                            let mut item = ArchiveItem::file(
                                relative_path,
                                meta,
                                Vec::new(),
                                ChunkerProfile::Default,
                            );
                            item.item_type = ItemType::Hardlink;
                            item.hardlink_target = Some(first_path.clone());
                            self.progress.on_file_complete(path, meta.len(), 0);
                            return Ok(Some(item));
                        } else {
                            // First time seeing this inode
                            self.hardlinks.insert(inode, relative_path.clone());
                        }
                    }
                }

                // Determine chunker profile
                let profile = self.forced_chunker_profile.unwrap_or_else(|| {
                    let size_mb = meta.len() / (1024 * 1024);
                    choose_profile_for_size(size_mb)
                });

                let chunker = Chunker::from_profile(profile);

                // Read and chunk the file
                let file_data = match fs::read(path) {
                    Ok(d) => d,
                    Err(e) => {
                        warn!("Cannot read file {}: {}", path.display(), e);
                        self.progress.on_error(path, &e.to_string());
                        return Ok(None);
                    }
                };

                let chunks = chunker.chunk_data(&file_data);
                let mut chunk_ids = Vec::with_capacity(chunks.len());

                for chunk in chunks {
                    let (is_new, stored_size) = self.repo.put_chunk(&chunk).await?;
                    let original_chunk_size = chunk.data.len() as u64;
                    if is_new {
                        stats.nchunks_unique += 1;
                        stats.deduplicated_size += original_chunk_size;
                        stats.compressed_size += stored_size;
                    }
                    stats.nchunks += 1;
                    chunk_ids.push(chunk.id);
                }

                stats.nfiles += 1;
                stats.original_size += meta.len();
                self.progress.on_progress(
                    stats.original_size,
                    total_size,
                    Some(relative_path.to_str().unwrap_or("")),
                );

                self.progress
                    .on_file_complete(path, meta.len(), chunk_ids.len());

                Ok(Some(ArchiveItem::file(
                    relative_path,
                    meta,
                    chunk_ids,
                    profile,
                )))
            }
            ItemType::Symlink => {
                let target = fs::read_link(path)?;
                Ok(Some(ArchiveItem::symlink(relative_path, meta, target)))
            }
            _ => {
                debug!("Skipping special file: {}", path.display());
                self.progress.on_file_skipped(path, "special file");
                Ok(None)
            }
        }
    }
}

/// Progress callback for restore operations
pub trait RestoreProgress: Send + Sync {
    /// Called when restore starts, with total files and bytes
    fn on_start(&self, total_files: u64, total_bytes: u64);
    /// Called when starting to restore a file
    fn on_file_start(&self, path: &Path, size: u64);
    /// Called when a file is completed
    fn on_file_complete(&self, path: &Path);
    /// Called when an error occurs (non-fatal)
    fn on_error(&self, path: &Path, error: &str);
    /// Called when the restore is finished
    fn on_finish(&self);
}

/// Null progress reporter for restore
pub struct NullRestoreProgress;
impl RestoreProgress for NullRestoreProgress {
    fn on_start(&self, _total_files: u64, _total_bytes: u64) {}
    fn on_file_start(&self, _path: &Path, _size: u64) {}
    fn on_file_complete(&self, _path: &Path) {}
    fn on_error(&self, _path: &Path, _error: &str) {}
    fn on_finish(&self) {}
}

/// Archive restorer for restoring backups
pub struct ArchiveRestorer<'a> {
    /// Repository to read from
    repo: &'a Repository,
    progress: Box<dyn RestoreProgress>,
}

impl<'a> ArchiveRestorer<'a> {
    /// Create a new restorer
    pub fn new(repo: &'a Repository) -> Self {
        Self {
            repo,
            progress: Box::new(NullRestoreProgress),
        }
    }

    /// Set progress reporter
    pub fn with_progress(mut self, progress: Box<dyn RestoreProgress>) -> Self {
        self.progress = progress;
        self
    }

    /// Restore an archive to a destination path
    #[instrument(skip(self))]
    pub async fn restore(&self, archive_name: &str, dest: &Path) -> Result<RestoreStats> {
        self.restore_paths(archive_name, dest, &[]).await
    }

    /// Restore specific paths from an archive to a destination
    #[instrument(skip(self, paths))]
    pub async fn restore_paths(
        &self,
        archive_name: &str,
        dest: &Path,
        paths: &[PathBuf],
    ) -> Result<RestoreStats> {
        info!(
            "Restoring {} paths from archive '{}' to {}",
            paths.len(),
            archive_name,
            dest.display()
        );

        let archive = self.load_archive(archive_name).await?;
        let mut stats = RestoreStats::default();

        fs::create_dir_all(dest)?;

        // Filter items based on requested paths
        let items: Vec<_> = if paths.is_empty() {
            archive.items.clone()
        } else if paths.len() == 1 && paths[0].to_str() == Some("::defaults") {
            // Use original paths if they exist
            if let Some(orig_paths) = &archive.metadata.original_paths {
                archive
                    .items
                    .into_iter()
                    .filter(|item| {
                        orig_paths.iter().any(|req_path| {
                            item.path == *req_path || item.path.starts_with(req_path)
                        })
                    })
                    .collect()
            } else {
                archive.items.clone()
            }
        } else {
            archive
                .items
                .into_iter()
                .filter(|item| {
                    paths
                        .iter()
                        .any(|req_path| item.path == *req_path || item.path.starts_with(req_path))
                })
                .collect()
        };

        // Determine effective destination based on absolute path request
        let is_absolute_restore = dest.is_absolute() && dest.to_str() != Some("/");

        // Calculate totals for progress bar
        let total_files = items
            .iter()
            .filter(|i| i.item_type == ItemType::File)
            .count() as u64;
        let total_bytes = items
            .iter()
            .filter(|i| i.item_type == ItemType::File)
            .map(|i| i.size)
            .sum();
        self.progress.on_start(total_files, total_bytes);

        // Sort items to ensure directories are created before their contents
        let mut items = items;
        items.sort_by(|a, b| a.path.cmp(&b.path));

        // First pass: Restore directories, files, and symlinks
        for item in &items {
            if item.item_type == ItemType::Hardlink {
                continue;
            }

            let target_path = if is_absolute_restore {
                // If dest is absolute, it becomes the new root /
                // item.path is typically relative (or stripped prefix)
                dest.join(&item.path)
            } else {
                dest.join(&item.path)
            };

            match item.item_type {
                ItemType::Directory => {
                    if let Err(e) = fs::create_dir_all(&target_path) {
                        let err_msg = format!(
                            "Failed to create directory {}: {}",
                            target_path.display(),
                            e
                        );
                        warn!("{}", err_msg);
                        self.progress.on_error(&target_path, &err_msg);
                        continue;
                    }
                    if let Err(e) = self.restore_attributes(&target_path, &item.attrs) {
                        let err_msg = format!(
                            "Failed to restore attributes for {}: {}",
                            target_path.display(),
                            e
                        );
                        warn!("{}", err_msg);
                        self.progress.on_error(&target_path, &err_msg);
                    }
                    stats.dirs_restored += 1;
                }
                ItemType::File => {
                    self.progress.on_file_start(&target_path, item.size);
                    if let Some(parent) = target_path.parent() {
                        if !parent.exists() {
                            if let Err(e) = fs::create_dir_all(parent) {
                                let err_msg = format!(
                                    "Failed to create parent directory {}: {}",
                                    parent.display(),
                                    e
                                );
                                warn!("{}", err_msg);
                                self.progress.on_error(&target_path, &err_msg);
                                continue;
                            }
                        }
                    }

                    // Reconstruct file from chunks
                    let mut file_data = Vec::new();
                    let mut error_occurred = false;
                    for chunk_id in &item.chunks {
                        match self.repo.get_chunk(chunk_id).await {
                            Ok(chunk) => file_data.extend_from_slice(&chunk.data),
                            Err(e) => {
                                self.progress.on_error(&target_path, &e.to_string());
                                error_occurred = true;
                                break;
                            }
                        }
                    }

                    if error_occurred {
                        continue;
                    }

                    if let Err(e) = fs::write(&target_path, &file_data) {
                        self.progress.on_error(&target_path, &e.to_string());
                        continue;
                    }
                    if let Err(e) = self.restore_attributes(&target_path, &item.attrs) {
                        self.progress.on_error(&target_path, &e.to_string());
                    }
                    stats.files_restored += 1;
                    stats.bytes_restored += file_data.len() as u64;
                    self.progress.on_file_complete(&target_path);
                }
                ItemType::Symlink => {
                    if let Some(target) = &item.symlink_target {
                        if let Some(parent) = target_path.parent() {
                            if !parent.exists() {
                                if let Err(e) = fs::create_dir_all(parent) {
                                    let err_msg = format!(
                                        "Failed to create parent directory for symlink {}: {}",
                                        target_path.display(),
                                        e
                                    );
                                    warn!("{}", err_msg);
                                    self.progress.on_error(&target_path, &err_msg);
                                    continue;
                                }
                            }
                        }
                        #[cfg(unix)]
                        if let Err(e) = std::os::unix::fs::symlink(target, &target_path) {
                            let err_msg = format!(
                                "Failed to create symlink {}: {}",
                                target_path.display(),
                                e
                            );
                            warn!("{}", err_msg);
                            self.progress.on_error(&target_path, &err_msg);
                        }
                        stats.symlinks_restored += 1;
                    }
                }
                _ => {}
            }
        }

        // Second pass: Restore hardlinks
        for item in &items {
            if item.item_type == ItemType::Hardlink {
                let target_path = dest.join(&item.path);
                if let Some(link_target) = &item.hardlink_target {
                    let link_source = dest.join(link_target);
                    if let Some(parent) = target_path.parent() {
                        if !parent.exists() {
                            if let Err(e) = fs::create_dir_all(parent) {
                                let err_msg = format!(
                                    "Failed to create parent directory for hardlink {}: {}",
                                    target_path.display(),
                                    e
                                );
                                warn!("{}", err_msg);
                                self.progress.on_error(&target_path, &err_msg);
                                continue;
                            }
                        }
                    }
                    if link_source.exists() {
                        if let Err(e) = fs::hard_link(&link_source, &target_path) {
                            let err_msg = format!(
                                "Failed to create hardlink from {} to {}: {}",
                                link_source.display(),
                                target_path.display(),
                                e
                            );
                            warn!("{}", err_msg);
                            self.progress.on_error(&target_path, &err_msg);
                        }
                        stats.hardlinks_restored += 1;
                    } else {
                        let err_msg =
                            format!("Hardlink source not found: {}", link_source.display());
                        warn!("{}", err_msg);
                        self.progress.on_error(&target_path, &err_msg);
                    }
                }
            }
        }

        self.progress.on_finish();
        info!(
            "Restored {} files, {} dirs, {} bytes",
            stats.files_restored, stats.dirs_restored, stats.bytes_restored
        );

        Ok(stats)
    }

    /// Load an archive by name
    pub async fn load_archive(&self, name: &str) -> Result<Archive> {
        let manifest = self.repo.load_manifest().await?;

        let archive_ref = manifest
            .archives
            .iter()
            .find(|a| a.name == name)
            .ok_or_else(|| BorgError::ArchiveNotFound {
                name: name.to_string(),
            })?;

        let chunk = self.repo.get_chunk(&archive_ref.id).await?;

        // Try deserializing as current version
        if let Ok(archive) = bincode::deserialize::<Archive>(&chunk.data) {
            return Ok(archive);
        }

        // Fallback definitions for older versions
        #[derive(Deserialize)]
        struct ArchiveMetadataV1 {
            name: String,
            time: DateTime<Utc>,
            hostname: String,
            username: String,
            cmdline: Vec<String>,
        }

        #[derive(Deserialize)]
        struct ArchiveItemV1 {
            path: PathBuf,
            item_type: ItemType,
            size: u64,
            attrs: UnixAttributes,
            chunks: Vec<ChunkId>,
            symlink_target: Option<PathBuf>,
        }

        let convert_item = |item: ArchiveItemV1| -> ArchiveItem {
            ArchiveItem {
                path: item.path,
                item_type: item.item_type,
                size: item.size,
                attrs: item.attrs,
                chunks: item.chunks,
                symlink_target: item.symlink_target,
                hardlink_target: None,
                chunker_profile: default_chunker_profile(),
            }
        };

        // Try V1 (Old Metadata, Old Item)
        #[derive(Deserialize)]
        struct ArchiveV1 {
            metadata: ArchiveMetadataV1,
            items: Vec<ArchiveItemV1>,
            stats: ArchiveStats,
        }

        if let Ok(v1) = bincode::deserialize::<ArchiveV1>(&chunk.data) {
            return Ok(Archive {
                metadata: ArchiveMetadata {
                    name: v1.metadata.name,
                    time: v1.metadata.time,
                    hostname: v1.metadata.hostname,
                    username: v1.metadata.username,
                    cmdline: v1.metadata.cmdline,
                    comment: None,
                    tags: None,
                    original_paths: None,
                    path_mapping: None,
                    compression: None,
                },
                items: v1.items.into_iter().map(convert_item).collect(),
                stats: v1.stats,
            });
        }

        // Try Mixed (Current Metadata, Old Item)
        #[derive(Deserialize)]
        struct ArchiveMixed {
            metadata: ArchiveMetadata,
            items: Vec<ArchiveItemV1>,
            stats: ArchiveStats,
        }

        if let Ok(mixed) = bincode::deserialize::<ArchiveMixed>(&chunk.data) {
            return Ok(Archive {
                metadata: mixed.metadata,
                items: mixed.items.into_iter().map(convert_item).collect(),
                stats: mixed.stats,
            });
        }

        Err(BorgError::Deserialization(
            "Failed to deserialize archive (unknown format)".to_string(),
        ))
    }

    /// Restore file attributes (permissions, ownership, times)
    fn restore_attributes(&self, path: &Path, attrs: &UnixAttributes) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            // Set permissions
            let perms = fs::Permissions::from_mode(attrs.mode);
            let _ = fs::set_permissions(path, perms);

            // Time restoration usually requires more crates or platform specific calls
            // We'll skip for now to maintain simple cross-platform compatibility
        }
        Ok(())
    }
}

/// Statistics for restoration operations
#[derive(Debug, Default, Clone)]
pub struct RestoreStats {
    /// Files restored
    pub files_restored: u64,
    /// Directories restored
    pub dirs_restored: u64,
    /// Symlinks restored
    pub symlinks_restored: u64,
    /// Hard links restored
    pub hardlinks_restored: u64,
    /// Total bytes restored
    pub bytes_restored: u64,
}

fn get_ntp_timestamp(servers: &[&str]) -> DateTime<Utc> {
    let mut rng = thread_rng();
    let mut list: Vec<_> = servers.to_vec();
    list.shuffle(&mut rng);

    for server in list {
        let target = format!("{}:123", server);
        if let Ok(client) = ntp_client::Client::new().target(&target) {
            if let Ok(response) = client.request() {
                if let Some(datetime) = response.get_datetime_utc() {
                    return datetime;
                }
            }
        }
    }
    Utc::now() // local fallback
}

// backup_archive_ptintime_<hostname>_<username>_<YYYY-MM-DDTHH:mm:ss.sssZ>
pub fn default_archive_unique_name() -> String {
    let ntp_servers = [
        "ntp1.inrim.it",
        "ntp2.inrim.it",
        "0.it.pool.ntp.org",
        "1.it.pool.ntp.org",
        "0.ch.pool.ntp.org",
        "1.ch.pool.ntp.org",
    ];

    let ts = get_ntp_timestamp(&ntp_servers);
    let hostname = gethostname();
    let username = get_username();

    format!(
        "backup_archive_ptintime_{}_{}_{}",
        hostname,
        username,
        ts.format("%Y-%m-%dT%H:%M:%S%.3fZ")
    )
}

/// Returns current time as: "HH:MM DD.MM.YYYY" (24h)
pub fn current_time_hh_mm_dd_mm_yyyy() -> String {
    let ntp_servers = [
        "ntp1.inrim.it",
        "ntp2.inrim.it",
        "0.it.pool.ntp.org",
        "1.it.pool.ntp.org",
        "0.ch.pool.ntp.org",
        "1.ch.pool.ntp.org",
    ];

    let ts = get_ntp_timestamp(&ntp_servers);

    ts.format("%H:%M %d.%m.%Y").to_string()
}

/// Returns current time formatted for archive names.
pub fn current_time_for_archive_name() -> String {
    let ntp_servers = [
        "ntp1.inrim.it",
        "ntp2.inrim.it",
        "0.it.pool.ntp.org",
        "1.it.pool.ntp.org",
        "0.ch.pool.ntp.org",
        "1.ch.pool.ntp.org",
    ];
    let ts = get_ntp_timestamp(&ntp_servers);
    ts.format("%Y-%m-%dT%H-%M-%S").to_string()
}

/// Get the system hostname
fn gethostname() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .or_else(|| std::env::var("HOSTNAME").ok()) // Linux/macOS fallback
        .or_else(|| std::env::var("COMPUTERNAME").ok()) // Windows fallback
        .unwrap_or_else(|| "unknown".to_string())
}

/// Get current username
fn get_username() -> String {
    #[cfg(unix)]
    {
        use libc::{getpwuid, getuid};
        use std::ffi::CStr;

        unsafe {
            let uid = getuid();
            let pw = getpwuid(uid);
            if !pw.is_null() && !(*pw).pw_name.is_null() {
                let name = CStr::from_ptr((*pw).pw_name);
                return name.to_string_lossy().into_owned();
            }
        }
        std::env::var("USER").unwrap_or_else(|_| "unknown".to_string())
    }

    #[cfg(windows)]
    {
        std::env::var("USERNAME").unwrap_or_else(|_| "unknown".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{StorageConfig, build_operator};
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_archive_creation() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().join("repo");
        let source_dir = temp_dir.path().join("source");

        // Create source files
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("file1.txt"), "Hello").unwrap();
        fs::write(source_dir.join("file2.txt"), "World").unwrap();
        fs::create_dir_all(source_dir.join("subdir")).unwrap();
        fs::write(source_dir.join("subdir/nested.txt"), "Nested").unwrap();

        // Initialize repository
        let op = build_operator(StorageConfig::Local { path: repo_path }).unwrap();
        let mut repo = Repository::init(op, "test-repo".to_string(), Some("passphrase"), None)
            .await
            .unwrap();

        // Create archive
        let creator = ArchiveCreator::new(&mut repo);
        let archive = creator
            .create("test-archive", &[source_dir.clone()], None, None)
            .await
            .unwrap();

        assert_eq!(archive.stats.nfiles, 3);
        assert_eq!(archive.stats.ndirs, 2); // source + subdir
    }

    #[test]
    fn test_archive_bincode_roundtrip() {
        let item = ArchiveItem {
            path: PathBuf::from("test/file"),
            item_type: ItemType::File,
            size: 123,
            attrs: UnixAttributes {
                mode: 0o644,
                uid: 1000,
                gid: 1000,
                atime: 100,
                mtime: 100,
                ctime: 100,
            },
            chunks: vec![ChunkId::new([1u8; 32])],
            symlink_target: None,
            hardlink_target: None,
            chunker_profile: ChunkerProfile::Size8M,
        };

        let metadata = ArchiveMetadata {
            name: "test-archive".to_string(),
            time: Utc::now(),
            hostname: "localhost".to_string(),
            username: "user".to_string(),
            cmdline: vec!["borg".to_string(), "create".to_string()],
            comment: Some("test comment".to_string()),
            tags: Some(vec!["tag1".to_string(), "tag2".to_string()]),
            original_paths: Some(vec![PathBuf::from("/tmp/test")]),
            path_mapping: None,
            compression: None,
        };

        let archive = Archive {
            metadata,
            items: vec![item],
            stats: ArchiveStats::default(),
        };

        // Serialize
        let encoded = bincode::serialize(&archive).unwrap();

        // Deserialize
        let decoded: Archive = bincode::deserialize(&encoded).unwrap();

        assert_eq!(decoded.metadata.name, "test-archive");
        assert_eq!(decoded.items.len(), 1);
        assert_eq!(decoded.items[0].path, PathBuf::from("test/file"));
        assert_eq!(decoded.items[0].symlink_target, None);
        assert_eq!(
            decoded.metadata.tags,
            Some(vec!["tag1".to_string(), "tag2".to_string()])
        );
        assert_eq!(decoded.items[0].chunker_profile, ChunkerProfile::Size8M);
        assert_eq!(
            decoded.metadata.original_paths,
            Some(vec![PathBuf::from("/tmp/test")])
        );
    }

    #[tokio::test]
    async fn test_empty_archive() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().join("repo");
        let source_dir = temp_dir.path().join("source");
        fs::create_dir_all(&source_dir).unwrap();

        let op = build_operator(StorageConfig::Local { path: repo_path }).unwrap();
        let mut repo = Repository::init(op, "test-repo".to_string(), Some("passphrase"), None)
            .await
            .unwrap();

        let creator = ArchiveCreator::new(&mut repo);
        let archive = creator
            .create("empty-archive", &[source_dir.clone()], None, None)
            .await
            .unwrap();

        assert_eq!(archive.stats.nfiles, 0);
        assert_eq!(archive.stats.ndirs, 1); // Just the root dir
    }

    #[tokio::test]
    async fn test_restore_missing_parents() {
        use crate::storage::StorageConfig;
        use crate::storage::build_operator;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().join("repo");
        let source_dir = temp_dir.path().join("source");

        // Deep nested source
        let deep_nested = source_dir.join("a/b/c");
        fs::create_dir_all(&deep_nested).unwrap();
        fs::write(deep_nested.join("file.txt"), "content").unwrap();

        // Create archive
        let op = build_operator(StorageConfig::Local { path: repo_path }).unwrap();
        let mut repo = Repository::init(
            op.clone(),
            "test-repo".to_string(),
            Some("passphrase"),
            None,
        )
        .await
        .unwrap();
        let creator = ArchiveCreator::new(&mut repo);
        creator
            .create("test", &[source_dir.clone()], None, None)
            .await
            .unwrap();

        // Restore to a path where intermediate directories don't exist
        let restore_root = temp_dir.path().join("restore_root");
        let deep_restore_dest = restore_root.join("x/y/z");
        // Note: we don't create deep_restore_dest or its parents x/y

        let repo = Repository::open(op, "test-repo".to_string(), Some("passphrase"))
            .await
            .unwrap();
        let restorer = ArchiveRestorer::new(&repo);

        // This should trigger fs::create_dir_all(dest) which creates x/y/z
        let stats = restorer.restore("test", &deep_restore_dest).await.unwrap();

        assert!(deep_restore_dest.exists());
        assert!(deep_restore_dest.join("a/b/c/file.txt").exists());
        assert_eq!(stats.files_restored, 1);
    }

    #[tokio::test]
    async fn test_archive_restoration() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().join("repo");
        let source_dir = temp_dir.path().join("source");
        let restore_dir = temp_dir.path().join("restore");

        // Setup source
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("data.txt"), "Important Data").unwrap();
        fs::create_dir_all(source_dir.join("folder")).unwrap();
        fs::write(source_dir.join("folder/sub.txt"), "Sub Data").unwrap();

        // Create archive
        let op = build_operator(StorageConfig::Local { path: repo_path }).unwrap();
        let mut repo = Repository::init(
            op.clone(),
            "test-repo".to_string(),
            Some("passphrase"),
            None,
        )
        .await
        .unwrap();
        let creator = ArchiveCreator::new(&mut repo);
        creator
            .create("backup1", &[source_dir.clone()], None, None)
            .await
            .unwrap();

        // Restore archive
        let repo = Repository::open(op, "test-repo".to_string(), Some("passphrase"))
            .await
            .unwrap();
        let restorer = ArchiveRestorer::new(&repo);
        let stats = restorer.restore("backup1", &restore_dir).await.unwrap();

        assert_eq!(stats.files_restored, 2);
        assert_eq!(stats.dirs_restored, 2);

        // Verify content
        let restored_data = fs::read_to_string(restore_dir.join("data.txt")).unwrap();
        assert_eq!(restored_data, "Important Data");

        let restored_sub = fs::read_to_string(restore_dir.join("folder/sub.txt")).unwrap();
        assert_eq!(restored_sub, "Sub Data");
    }

    #[tokio::test]
    async fn test_symlink_support() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let temp_dir = TempDir::new().unwrap();
            let repo_path = temp_dir.path().join("repo");
            let source_dir = temp_dir.path().join("source");
            let restore_dir = temp_dir.path().join("restore");

            fs::create_dir_all(&source_dir).unwrap();
            fs::write(source_dir.join("target.txt"), "Target").unwrap();
            symlink("target.txt", source_dir.join("link.txt")).unwrap();

            let op = build_operator(StorageConfig::Local { path: repo_path }).unwrap();
            let mut repo = Repository::init(
                op.clone(),
                "test-repo".to_string(),
                Some("passphrase"),
                None,
            )
            .await
            .unwrap();

            let creator = ArchiveCreator::new(&mut repo);
            let archive = creator
                .create("symlink-test", &[source_dir.clone()], None, None)
                .await
                .unwrap();

            // Check archive stats
            // 1 file, 1 dir (root), 1 symlink
            assert_eq!(archive.stats.nfiles, 1);

            // Restore
            let repo = Repository::open(op, "test-repo".to_string(), Some("passphrase"))
                .await
                .unwrap();
            let restorer = ArchiveRestorer::new(&repo);
            let stats = restorer
                .restore("symlink-test", &restore_dir)
                .await
                .unwrap();

            assert_eq!(stats.symlinks_restored, 1);

            // Verify symlink
            let link_path = restore_dir.join("link.txt");
            assert!(
                fs::symlink_metadata(&link_path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            let target = fs::read_link(&link_path).unwrap();
            assert_eq!(target, PathBuf::from("target.txt"));
        }
    }

    #[tokio::test]
    async fn test_hardlink_support() {
        #[cfg(unix)]
        {
            let temp_dir = TempDir::new().unwrap();
            let repo_path = temp_dir.path().join("repo");
            let source_dir = temp_dir.path().join("source");
            let restore_dir = temp_dir.path().join("restore");

            fs::create_dir_all(&source_dir).unwrap();
            let file1 = source_dir.join("file1.txt");
            let file2 = source_dir.join("file2.txt");

            fs::write(&file1, "Shared Content").unwrap();
            fs::hard_link(&file1, &file2).unwrap();

            let op = build_operator(StorageConfig::Local { path: repo_path }).unwrap();
            let mut repo = Repository::init(
                op.clone(),
                "test-repo".to_string(),
                Some("passphrase"),
                None,
            )
            .await
            .unwrap();

            let creator = ArchiveCreator::new(&mut repo);
            let archive = creator
                .create("hardlink-test", &[source_dir.clone()], None, None)
                .await
                .unwrap();

            // Should have 2 files, but deduplicated chunks
            assert_eq!(archive.stats.nfiles, 2);
            // Only one file's worth of chunks should be unique
            assert_eq!(archive.stats.nchunks_unique, archive.stats.nchunks);

            // Restore
            let repo = Repository::open(op, "test-repo".to_string(), Some("passphrase"))
                .await
                .unwrap();
            let restorer = ArchiveRestorer::new(&repo);
            let stats = restorer
                .restore("hardlink-test", &restore_dir)
                .await
                .unwrap();

            assert_eq!(stats.hardlinks_restored, 1);

            // Verify hardlink relationship
            let r_file1 = restore_dir.join("file1.txt");
            let r_file2 = restore_dir.join("file2.txt");

            use std::os::unix::fs::MetadataExt;
            let meta1 = fs::metadata(&r_file1).unwrap();
            let meta2 = fs::metadata(&r_file2).unwrap();
            assert_eq!(meta1.ino(), meta2.ino());
        }
    }

    #[tokio::test]
    async fn test_exclusion_patterns() {
        use crate::exclusion::ExclusionPattern;
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().join("repo");
        let source_dir = temp_dir.path().join("source");

        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("include.txt"), "Keep").unwrap();
        fs::write(source_dir.join("exclude.tmp"), "Ignore").unwrap();
        fs::create_dir_all(source_dir.join("node_modules")).unwrap();
        fs::write(source_dir.join("node_modules/lib.js"), "Code").unwrap();

        let op = build_operator(StorageConfig::Local { path: repo_path }).unwrap();
        let mut repo = Repository::init(op, "test-repo".to_string(), Some("passphrase"), None)
            .await
            .unwrap();

        let mut exclusions = ExclusionList::new();
        exclusions
            .add_pattern(ExclusionPattern::glob("*.tmp"))
            .unwrap();
        exclusions
            .add_pattern(ExclusionPattern::glob("**/node_modules"))
            .unwrap();

        let creator = ArchiveCreator::new(&mut repo).with_exclusions(exclusions);
        let archive = creator
            .create("exclude-test", &[source_dir.clone()], None, None)
            .await
            .unwrap();

        // Should only contain include.txt and the root dir
        assert_eq!(archive.stats.nfiles, 1);

        // Verify items
        let has_tmp = archive
            .items
            .iter()
            .any(|i| i.path.to_string_lossy().contains("exclude.tmp"));
        let has_node = archive
            .items
            .iter()
            .any(|i| i.path.to_string_lossy().contains("node_modules"));
        assert!(!has_tmp);
        assert!(!has_node);
    }

    #[tokio::test]
    async fn test_duplicate_archive_name() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().join("repo");
        let source_dir = temp_dir.path().join("source");
        fs::create_dir_all(&source_dir).unwrap();

        let op = build_operator(StorageConfig::Local { path: repo_path }).unwrap();
        let mut repo = Repository::init(op, "test-repo".to_string(), Some("passphrase"), None)
            .await
            .unwrap();

        let creator = ArchiveCreator::new(&mut repo);
        creator
            .create("dup-test", &[source_dir.clone()], None, None)
            .await
            .unwrap();

        let creator = ArchiveCreator::new(&mut repo);
        let result = creator
            .create("dup-test", &[source_dir.clone()], None, None)
            .await;

        assert!(matches!(result, Err(BorgError::ArchiveExists { .. })));
    }
}
