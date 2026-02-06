use crate::chunker::{Chunk, ChunkId, Chunker, ChunkerConfig};
use crate::error::{BorgError, Result};
use crate::exclusion::ExclusionList;
use crate::repository::{ArchiveRef, Repository};
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
}

impl ArchiveItem {
    /// Create a new file item
    pub fn file(path: PathBuf, meta: &Metadata, chunks: Vec<ChunkId>) -> Self {
        Self {
            path,
            item_type: ItemType::File,
            size: meta.len(),
            attrs: UnixAttributes::from_metadata(meta),
            chunks,
            symlink_target: None,
            hardlink_target: None,
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
        }
    }
}

/// Archive metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveMetadata {
    /// Archive name
    pub name: String,
    /// Creation timestamp
    pub time: chrono::DateTime<chrono::Utc>,
    /// Hostname where the backup was created
    pub hostname: String,
    /// Username who created the backup
    pub username: String,
    /// Command line used to create the backup
    pub cmdline: Vec<String>,
    /// Comment (optional)
    pub comment: Option<String>,
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
    fn on_progress(&self, processed: u64, total: u64);
    /// Called when an error occurs (non-fatal)
    fn on_error(&self, path: &Path, error: &str);
}

/// Null progress reporter (does nothing)
pub struct NullProgress;
impl BackupProgress for NullProgress {
    fn on_file_start(&self, _path: &Path) {}
    fn on_file_complete(&self, _path: &Path, _size: u64, _chunks: usize) {}
    fn on_file_skipped(&self, _path: &Path, _reason: &str) {}
    fn on_progress(&self, _processed: u64, _total: u64) {}
    fn on_error(&self, _path: &Path, _error: &str) {}
}

/// Archive creator for building new archives
pub struct ArchiveCreator<'a> {
    /// Repository to store data
    repo: &'a mut Repository,
    /// Chunker for splitting files
    chunker: Chunker,
    /// Exclusion list
    exclusions: ExclusionList,
    /// Progress reporter
    progress: Box<dyn BackupProgress>,
    /// Hard link tracking (inode -> first path)
    hardlinks: HashMap<u64, PathBuf>,
}

impl<'a> ArchiveCreator<'a> {
    /// Create a new archive creator
    pub fn new(repo: &'a mut Repository) -> Self {
        Self {
            repo,
            chunker: Chunker::with_defaults(),
            exclusions: ExclusionList::new(),
            progress: Box::new(NullProgress),
            hardlinks: HashMap::new(),
        }
    }

    /// Set chunker configuration
    pub fn with_chunker_config(mut self, config: ChunkerConfig) -> Result<Self> {
        self.chunker = Chunker::new(config)?;
        Ok(self)
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

    /// Create an archive from a list of paths
    #[instrument(skip(self, paths, name))]
    pub async fn create(
        mut self,
        name: &str,
        paths: &[PathBuf],
        comment: Option<String>,
    ) -> Result<Archive> {
        info!("Creating archive '{}' from {} paths", name, paths.len());

        // Check for existing archive with same name
        let manifest = self.repo.load_manifest().await?;
        if manifest.archives.iter().any(|a| a.name == name) {
            return Err(BorgError::ArchiveExists {
                name: name.to_string(),
            });
        }

        let metadata = ArchiveMetadata {
            name: name.to_string(),
            time: chrono::Utc::now(),
            hostname: gethostname(),
            username: get_username(),
            cmdline: std::env::args().collect(),
            comment,
        };

        let mut items = Vec::new();
        let mut stats = ArchiveStats::default();

        // Process each path
        for path in paths {
            self.process_path(path, path, &mut items, &mut stats).await?;
        }

        let archive = Archive {
            metadata,
            items,
            stats,
        };

        // Store archive metadata as a chunk
        let archive_data = bincode::serialize(&archive)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        let archive_chunk = Chunk::new(archive_data);
        let archive_id = archive_chunk.id.clone();
        let _ = self.repo.put_chunk(&archive_chunk).await?;

        // Update manifest
        let mut manifest = self.repo.load_manifest().await?;
        manifest.archives.push(ArchiveRef {
            name: name.to_string(),
            id: archive_id,
            time: archive.metadata.time,
        });
        manifest.timestamp = chrono::Utc::now();
        self.repo.save_manifest(&manifest).await?;

        // Commit changes
        self.repo.commit().await?;

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
    ) -> Result<()> {
        // Walk the directory tree and collect entries that are not excluded
        let entries: Vec<_> = WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| {
                match e {
                    Ok(entry) => {
                        let is_excluded = self.exclusions.is_excluded(entry.path(), entry.file_type().is_dir());
                        if is_excluded {
                            self.progress.on_file_skipped(entry.path(), "excluded");
                            None
                        } else {
                            Some(Ok(entry))
                        }
                    }
                    Err(e) => Some(Err(e)),
                }
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

            let item = self.process_entry(entry_path, relative_path, &meta, stats).await?;
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
                            let mut item = ArchiveItem::file(relative_path, meta, Vec::new());
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

                // Read and chunk the file
                let file_data = match fs::read(path) {
                    Ok(d) => d,
                    Err(e) => {
                        warn!("Cannot read file {}: {}", path.display(), e);
                        self.progress.on_error(path, &e.to_string());
                        return Ok(None);
                    }
                };

                let chunks = self.chunker.chunk_data(&file_data);
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

                self.progress.on_file_complete(path, meta.len(), chunk_ids.len());

                Ok(Some(ArchiveItem::file(relative_path, meta, chunk_ids)))
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

/// Archive extractor for restoring backups
pub struct ArchiveExtractor<'a> {
    /// Repository to read from
    repo: &'a Repository,
}

impl<'a> ArchiveExtractor<'a> {
    /// Create a new extractor
    pub fn new(repo: &'a Repository) -> Self {
        Self { repo }
    }

    /// Extract an archive to a destination path
    #[instrument(skip(self))]
    pub async fn extract(&self, archive_name: &str, dest: &Path) -> Result<ExtractStats> {
        info!("Extracting archive '{}' to {}", archive_name, dest.display());

        let archive = self.load_archive(archive_name).await?;
        let mut stats = ExtractStats::default();

        fs::create_dir_all(dest)?;

        // Sort items to ensure directories are created before their contents
        let mut items = archive.items.clone();
        items.sort_by(|a, b| a.path.cmp(&b.path));

        for item in &items {
            let target_path = dest.join(&item.path);
            
            match item.item_type {
                ItemType::Directory => {
                    fs::create_dir_all(&target_path)?;
                    self.restore_attributes(&target_path, &item.attrs)?;
                    stats.dirs_extracted += 1;
                }
                ItemType::File => {
                    if let Some(parent) = target_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    
                    // Reconstruct file from chunks
                    let mut file_data = Vec::new();
                    for chunk_id in &item.chunks {
                        let chunk = self.repo.get_chunk(chunk_id).await?;
                        file_data.extend_from_slice(&chunk.data);
                    }
                    
                    fs::write(&target_path, &file_data)?;
                    self.restore_attributes(&target_path, &item.attrs)?;
                    stats.files_extracted += 1;
                    stats.bytes_extracted += file_data.len() as u64;
                }
                ItemType::Symlink => {
                    if let Some(target) = &item.symlink_target {
                        if let Some(parent) = target_path.parent() {
                            fs::create_dir_all(parent)?;
                        }
                        #[cfg(unix)]
                        std::os::unix::fs::symlink(target, &target_path)?;
                        stats.symlinks_extracted += 1;
                    }
                }
                ItemType::Hardlink => {
                    if let Some(link_target) = &item.hardlink_target {
                        let link_source = dest.join(link_target);
                        if let Some(parent) = target_path.parent() {
                            fs::create_dir_all(parent)?;
                        }
                        std::fs::hard_link(&link_source, &target_path)?;
                        stats.hardlinks_extracted += 1;
                    }
                }
                _ => {
                    debug!("Skipping special file during extraction: {}", item.path.display());
                }
            }
        }

        info!(
            "Extracted {} files, {} dirs, {} bytes",
            stats.files_extracted, stats.dirs_extracted, stats.bytes_extracted
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
        let archive: Archive = bincode::deserialize(&chunk.data)
            .map_err(|e| BorgError::Deserialization(e.to_string()))?;

        Ok(archive)
    }

    /// Restore file attributes (permissions, ownership, times)
    fn restore_attributes(&self, path: &Path, attrs: &UnixAttributes) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            
            // Set permissions
            let perms = std::fs::Permissions::from_mode(attrs.mode);
            let _ = fs::set_permissions(path, perms);

            // Time restoration usually requires more crates or platform specific calls
            // We'll skip for now to maintain simple cross-platform compatibility
        }
        Ok(())
    }
}

/// Statistics for extraction operations
#[derive(Debug, Default, Clone)]
pub struct ExtractStats {
    /// Files extracted
    pub files_extracted: u64,
    /// Directories extracted
    pub dirs_extracted: u64,
    /// Symlinks extracted
    pub symlinks_extracted: u64,
    /// Hard links extracted
    pub hardlinks_extracted: u64,
    /// Total bytes extracted
    pub bytes_extracted: u64,
}

/// Get the system hostname
fn gethostname() -> String {
    #[cfg(unix)]
    {
        std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string())
    }
    #[cfg(not(unix))]
    {
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".to_string())
    }
}

/// Get current username
fn get_username() -> String {
    #[cfg(unix)]
    {
        std::env::var("USER").unwrap_or_else(|_| "unknown".to_string())
    }
    #[cfg(not(unix))]
    {
        std::env::var("USERNAME").unwrap_or_else(|_| "unknown".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use crate::storage::{StorageConfig, build_operator};

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
        let mut repo = Repository::init(op, Some("passphrase"), None).await.unwrap();

        // Create archive
        let creator = ArchiveCreator::new(&mut repo);
        let archive = creator
            .create("test-archive", &[source_dir.clone()], None)
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
        };

        let metadata = ArchiveMetadata {
            name: "test-archive".to_string(),
            time: chrono::Utc::now(),
            hostname: "localhost".to_string(),
            username: "user".to_string(),
            cmdline: vec!["borg".to_string(), "create".to_string()],
            comment: Some("test comment".to_string()),
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
    }
}
