//! Snapshot catalog for searchable backup metadata
//!
//! The catalog provides efficient querying of backup metadata across
//! all snapshots in a repository. It indexes:
//! - Archive metadata (name, time, hostname, etc.)
//! - File paths and metadata
//! - File changes between snapshots
//!
//! The catalog uses sled for persistent storage and supports:
//! - Full-text search on file paths
//! - Filtering by date, size, type
//! - Change tracking between snapshots
//! - Efficient snapshot listing and statistics

use crate::archive::{Archive, ArchiveItem, ItemType};
use crate::chunker::ChunkId;
use crate::error::{BorgError, Result};
use serde::{Deserialize, Serialize};
use sled::Db;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Snapshot catalog database
pub struct Catalog {
    /// Main database
    db: Db,
    /// Archive metadata tree
    archives: sled::Tree,
    /// File index tree (path -> archive items)
    files: sled::Tree,
    /// Path prefix index for search
    path_index: sled::Tree,
    /// Statistics tree
    stats: sled::Tree,
}

impl Catalog {
    /// Open or create a catalog at the given path
    pub fn open(catalog_path: &Path) -> Result<Self> {
        let db = sled::open(catalog_path)?;
        
        let archives = db.open_tree("archives")?;
        let files = db.open_tree("files")?;
        let path_index = db.open_tree("path_index")?;
        let stats = db.open_tree("stats")?;

        debug!("Opened catalog at {}", catalog_path.display());

        Ok(Self {
            db,
            archives,
            files,
            path_index,
            stats,
        })
    }

    /// Index an archive in the catalog
    pub fn index_archive(&self, archive: &Archive) -> Result<()> {
        info!("Indexing archive '{}'", archive.metadata.name);

        // Store archive metadata
        let archive_entry = CatalogArchive::from_archive(archive);
        let key = archive.metadata.name.as_bytes();
        let value = bincode::serialize(&archive_entry)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        self.archives.insert(key, value)?;

        // Index all files
        for item in &archive.items {
            self.index_item(&archive.metadata.name, item)?;
        }

        // Update statistics
        self.update_stats(&archive.metadata.name, archive)?;

        self.db.flush()?;
        
        info!(
            "Indexed archive '{}' with {} items",
            archive.metadata.name,
            archive.items.len()
        );

        Ok(())
    }

    /// Index a single archive item
    fn index_item(&self, archive_name: &str, item: &ArchiveItem) -> Result<()> {
        let file_entry = CatalogFile {
            archive_name: archive_name.to_string(),
            path: item.path.clone(),
            item_type: item.item_type,
            size: item.size,
            mtime: item.attrs.mtime,
            mode: item.attrs.mode,
            uid: item.attrs.uid,
            gid: item.attrs.gid,
            chunk_ids: item.chunks.clone(),
        };

        // Key: archive_name:path
        let key = format!("{}:{}", archive_name, item.path.display());
        let value = bincode::serialize(&file_entry)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        self.files.insert(key.as_bytes(), value)?;

        // Index path components for prefix search
        let path_str = item.path.to_string_lossy().to_lowercase();
        for (i, _) in path_str.char_indices() {
            let prefix = &path_str[..=i];
            let index_key = format!("{}:{}", prefix, archive_name);
            self.path_index.insert(
                index_key.as_bytes(),
                item.path.to_string_lossy().as_bytes(),
            )?;
        }

        Ok(())
    }

    /// Update statistics for an archive
    fn update_stats(&self, archive_name: &str, archive: &Archive) -> Result<()> {
        let stats = CatalogStats {
            archive_name: archive_name.to_string(),
            total_files: archive.stats.nfiles,
            total_dirs: archive.stats.ndirs,
            total_size: archive.stats.original_size,
            compressed_size: archive.stats.compressed_size,
            deduplicated_size: archive.stats.deduplicated_size,
            total_chunks: archive.stats.nchunks,
            unique_chunks: archive.stats.nchunks_unique,
            indexed_at: chrono::Utc::now(),
        };

        let value = bincode::serialize(&stats)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        self.stats.insert(archive_name.as_bytes(), value)?;

        Ok(())
    }

    /// Remove an archive from the catalog
    pub fn remove_archive(&self, archive_name: &str) -> Result<()> {
        info!("Removing archive '{}' from catalog", archive_name);

        // Remove archive metadata
        self.archives.remove(archive_name.as_bytes())?;
        self.stats.remove(archive_name.as_bytes())?;

        // Remove file entries
        let prefix = format!("{}:", archive_name);
        let keys_to_remove: Vec<_> = self.files
            .scan_prefix(prefix.as_bytes())
            .filter_map(|r| r.ok())
            .map(|(k, _)| k)
            .collect();

        for key in keys_to_remove {
            self.files.remove(&key)?;
        }

        // Remove path index entries
        let index_keys_to_remove: Vec<_> = self.path_index
            .iter()
            .filter_map(|r| r.ok())
            .filter(|(k, _)| {
                String::from_utf8_lossy(&k).ends_with(&format!(":{}", archive_name))
            })
            .map(|(k, _)| k)
            .collect();

        for key in index_keys_to_remove {
            self.path_index.remove(&key)?;
        }

        self.db.flush()?;
        Ok(())
    }

    /// List all archives in the catalog
    pub fn list_archives(&self) -> Result<Vec<CatalogArchive>> {
        let mut archives = Vec::new();

        for result in self.archives.iter() {
            let (_, value) = result?;
            let archive: CatalogArchive = bincode::deserialize(&value)
                .map_err(|e| BorgError::Deserialization(e.to_string()))?;
            archives.push(archive);
        }

        // Sort by time (newest first)
        archives.sort_by(|a, b| b.time.cmp(&a.time));

        Ok(archives)
    }

    /// Get a specific archive
    pub fn get_archive(&self, name: &str) -> Result<Option<CatalogArchive>> {
        match self.archives.get(name.as_bytes())? {
            Some(value) => {
                let archive: CatalogArchive = bincode::deserialize(&value)
                    .map_err(|e| BorgError::Deserialization(e.to_string()))?;
                Ok(Some(archive))
            }
            None => Ok(None),
        }
    }

    /// Search for files by path pattern
    pub fn search_files(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let mut results = Vec::new();

        // Search in specified archives or all
        let archives = if let Some(ref archive_filter) = query.archive_filter {
            vec![archive_filter.clone()]
        } else {
            self.list_archives()?.into_iter().map(|a| a.name).collect()
        };

        for archive_name in archives {
            let prefix = format!("{}:", archive_name);
            
            for result in self.files.scan_prefix(prefix.as_bytes()) {
                let (_, value) = result?;
                let file: CatalogFile = bincode::deserialize(&value)
                    .map_err(|e| BorgError::Deserialization(e.to_string()))?;

                if self.matches_query(&file, query) {
                    results.push(SearchResult {
                        archive_name: file.archive_name.clone(),
                        path: file.path.clone(),
                        item_type: file.item_type,
                        size: file.size,
                        mtime: file.mtime,
                    });

                    if let Some(limit) = query.limit {
                        if results.len() >= limit {
                            return Ok(results);
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    /// Check if a file matches the search query
    fn matches_query(&self, file: &CatalogFile, query: &SearchQuery) -> bool {
        // Path pattern matching
        if let Some(ref pattern) = query.path_pattern {
            let path_str = file.path.to_string_lossy().to_lowercase();
            let pattern_lower = pattern.to_lowercase();
            
            let matches = if query.exact_match {
                path_str == pattern_lower
            } else if query.glob_pattern {
                // Simple glob matching
                self.glob_match(&path_str, &pattern_lower)
            } else {
                path_str.contains(&pattern_lower)
            };

            if !matches {
                return false;
            }
        }

        // Item type filter
        if let Some(item_type) = query.item_type {
            if file.item_type != item_type {
                return false;
            }
        }

        // Size filters
        if let Some(min_size) = query.min_size {
            if file.size < min_size {
                return false;
            }
        }
        if let Some(max_size) = query.max_size {
            if file.size > max_size {
                return false;
            }
        }

        // Time filters
        if let Some(after) = query.modified_after {
            if file.mtime < after {
                return false;
            }
        }
        if let Some(before) = query.modified_before {
            if file.mtime > before {
                return false;
            }
        }

        true
    }

    /// Simple glob pattern matching
    fn glob_match(&self, text: &str, pattern: &str) -> bool {
        let mut text_chars = text.chars().peekable();
        let mut pattern_chars = pattern.chars().peekable();

        while let Some(pc) = pattern_chars.next() {
            match pc {
                '*' => {
                    // Match zero or more characters
                    if pattern_chars.peek().is_none() {
                        return true;
                    }
                    // Try matching the rest of the pattern at each position
                    let rest_pattern: String = pattern_chars.collect();
                    let rest_text: String = text_chars.collect();
                    for i in 0..=rest_text.len() {
                        if self.glob_match(&rest_text[i..], &rest_pattern) {
                            return true;
                        }
                    }
                    return false;
                }
                '?' => {
                    // Match exactly one character
                    if text_chars.next().is_none() {
                        return false;
                    }
                }
                c => {
                    // Match literal character
                    if text_chars.next() != Some(c) {
                        return false;
                    }
                }
            }
        }

        text_chars.next().is_none()
    }

    /// Compare two archives and return differences
    pub fn diff_archives(&self, archive1: &str, archive2: &str) -> Result<ArchiveDiff> {
        info!("Computing diff between '{}' and '{}'", archive1, archive2);

        let mut files1: HashMap<PathBuf, CatalogFile> = HashMap::new();
        let mut files2: HashMap<PathBuf, CatalogFile> = HashMap::new();

        // Load files from archive1
        let prefix1 = format!("{}:", archive1);
        for result in self.files.scan_prefix(prefix1.as_bytes()) {
            let (_, value) = result?;
            let file: CatalogFile = bincode::deserialize(&value)
                .map_err(|e| BorgError::Deserialization(e.to_string()))?;
            files1.insert(file.path.clone(), file);
        }

        // Load files from archive2
        let prefix2 = format!("{}:", archive2);
        for result in self.files.scan_prefix(prefix2.as_bytes()) {
            let (_, value) = result?;
            let file: CatalogFile = bincode::deserialize(&value)
                .map_err(|e| BorgError::Deserialization(e.to_string()))?;
            files2.insert(file.path.clone(), file);
        }

        let all_paths: HashSet<_> = files1.keys().chain(files2.keys()).cloned().collect();

        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut modified = Vec::new();
        let mut unchanged = 0;

        for path in all_paths {
            match (files1.get(&path), files2.get(&path)) {
                (None, Some(f2)) => {
                    added.push(DiffEntry::from_file(f2, ChangeType::Added));
                }
                (Some(f1), None) => {
                    removed.push(DiffEntry::from_file(f1, ChangeType::Removed));
                }
                (Some(f1), Some(f2)) => {
                    if f1.is_different_from(f2) {
                        modified.push(DiffEntry::from_files(f1, f2));
                    } else {
                        unchanged += 1;
                    }
                }
                (None, None) => unreachable!(),
            }
        }

        // Sort by path
        added.sort_by(|a, b| a.path.cmp(&b.path));
        removed.sort_by(|a, b| a.path.cmp(&b.path));
        modified.sort_by(|a, b| a.path.cmp(&b.path));

        Ok(ArchiveDiff {
            archive1: archive1.to_string(),
            archive2: archive2.to_string(),
            added,
            removed,
            modified,
            unchanged,
        })
    }

    /// Get statistics for an archive
    pub fn get_stats(&self, archive_name: &str) -> Result<Option<CatalogStats>> {
        match self.stats.get(archive_name.as_bytes())? {
            Some(value) => {
                let stats: CatalogStats = bincode::deserialize(&value)
                    .map_err(|e| BorgError::Deserialization(e.to_string()))?;
                Ok(Some(stats))
            }
            None => Ok(None),
        }
    }

    /// Get aggregate statistics across all archives
    pub fn get_total_stats(&self) -> Result<TotalCatalogStats> {
        let mut total = TotalCatalogStats::default();

        for result in self.stats.iter() {
            let (_, value) = result?;
            let stats: CatalogStats = bincode::deserialize(&value)
                .map_err(|e| BorgError::Deserialization(e.to_string()))?;
            
            total.total_archives += 1;
            total.total_files += stats.total_files;
            total.total_dirs += stats.total_dirs;
            total.total_size += stats.total_size;
        }

        Ok(total)
    }

    /// Rebuild catalog from repository archives
    pub fn rebuild_from_archives(&self, archives: &[Archive]) -> Result<()> {
        info!("Rebuilding catalog from {} archives", archives.len());

        // Clear existing data
        self.archives.clear()?;
        self.files.clear()?;
        self.path_index.clear()?;
        self.stats.clear()?;

        // Index all archives
        for archive in archives {
            self.index_archive(archive)?;
        }

        info!("Catalog rebuild complete");
        Ok(())
    }

    /// Flush all pending writes to disk
    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }
}

/// Archive entry in the catalog
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogArchive {
    /// Archive name
    pub name: String,
    /// Creation time
    pub time: chrono::DateTime<chrono::Utc>,
    /// Hostname
    pub hostname: String,
    /// Username
    pub username: String,
    /// Comment
    pub comment: Option<String>,
    /// Number of files
    pub file_count: u64,
    /// Number of directories
    pub dir_count: u64,
    /// Total original size
    pub total_size: u64,
}

impl CatalogArchive {
    /// Create from an Archive
    pub fn from_archive(archive: &Archive) -> Self {
        Self {
            name: archive.metadata.name.clone(),
            time: archive.metadata.time,
            hostname: archive.metadata.hostname.clone(),
            username: archive.metadata.username.clone(),
            comment: archive.metadata.comment.clone(),
            file_count: archive.stats.nfiles,
            dir_count: archive.stats.ndirs,
            total_size: archive.stats.original_size,
        }
    }
}

/// File entry in the catalog
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogFile {
    /// Archive name this file belongs to
    pub archive_name: String,
    /// File path
    pub path: PathBuf,
    /// Item type
    pub item_type: ItemType,
    /// File size
    pub size: u64,
    /// Modification time
    pub mtime: i64,
    /// File mode
    pub mode: u32,
    /// Owner UID
    pub uid: u32,
    /// Owner GID
    pub gid: u32,
    /// Chunk IDs
    pub chunk_ids: Vec<ChunkId>,
}

impl CatalogFile {
    /// Check if this file is different from another
    pub fn is_different_from(&self, other: &CatalogFile) -> bool {
        self.size != other.size
            || self.mtime != other.mtime
            || self.mode != other.mode
            || self.chunk_ids != other.chunk_ids
    }
}

/// Statistics for an archive in the catalog
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogStats {
    /// Archive name
    pub archive_name: String,
    /// Total files
    pub total_files: u64,
    /// Total directories
    pub total_dirs: u64,
    /// Total original size
    pub total_size: u64,
    /// Compressed size
    pub compressed_size: u64,
    /// Deduplicated size
    pub deduplicated_size: u64,
    /// Total chunks
    pub total_chunks: u64,
    /// Unique chunks
    pub unique_chunks: u64,
    /// When this was indexed
    pub indexed_at: chrono::DateTime<chrono::Utc>,
}

/// Total statistics across all archives
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TotalCatalogStats {
    /// Number of archives
    pub total_archives: u64,
    /// Total files across all archives
    pub total_files: u64,
    /// Total directories
    pub total_dirs: u64,
    /// Total size
    pub total_size: u64,
}

/// Query for searching files
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Path pattern to search for
    pub path_pattern: Option<String>,
    /// Filter by archive name
    pub archive_filter: Option<String>,
    /// Filter by item type
    pub item_type: Option<ItemType>,
    /// Minimum file size
    pub min_size: Option<u64>,
    /// Maximum file size
    pub max_size: Option<u64>,
    /// Modified after timestamp
    pub modified_after: Option<i64>,
    /// Modified before timestamp
    pub modified_before: Option<i64>,
    /// Maximum results
    pub limit: Option<usize>,
    /// Use exact path matching
    pub exact_match: bool,
    /// Use glob pattern matching
    pub glob_pattern: bool,
}

impl SearchQuery {
    /// Create a new search query
    pub fn new() -> Self {
        Self::default()
    }

    /// Search for path pattern
    pub fn path(mut self, pattern: &str) -> Self {
        self.path_pattern = Some(pattern.to_string());
        self
    }

    /// Filter by archive
    pub fn in_archive(mut self, archive: &str) -> Self {
        self.archive_filter = Some(archive.to_string());
        self
    }

    /// Filter by file type
    pub fn file_type(mut self, item_type: ItemType) -> Self {
        self.item_type = Some(item_type);
        self
    }

    /// Minimum size filter
    pub fn min_size(mut self, size: u64) -> Self {
        self.min_size = Some(size);
        self
    }

    /// Maximum size filter
    pub fn max_size(mut self, size: u64) -> Self {
        self.max_size = Some(size);
        self
    }

    /// Limit results
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    /// Use glob pattern
    pub fn glob(mut self) -> Self {
        self.glob_pattern = true;
        self
    }

    /// Use exact matching
    pub fn exact(mut self) -> Self {
        self.exact_match = true;
        self
    }
}

/// Search result entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Archive containing this file
    pub archive_name: String,
    /// File path
    pub path: PathBuf,
    /// Item type
    pub item_type: ItemType,
    /// File size
    pub size: u64,
    /// Modification time
    pub mtime: i64,
}

/// Type of change between archives
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeType {
    /// File was added
    Added,
    /// File was removed
    Removed,
    /// File was modified
    Modified,
}

impl std::fmt::Display for ChangeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangeType::Added => write!(f, "added"),
            ChangeType::Removed => write!(f, "removed"),
            ChangeType::Modified => write!(f, "modified"),
        }
    }
}

/// Entry in a diff result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffEntry {
    /// File path
    pub path: PathBuf,
    /// Type of change
    pub change_type: ChangeType,
    /// Item type
    pub item_type: ItemType,
    /// Size in first archive (if present)
    pub size_before: Option<u64>,
    /// Size in second archive (if present)
    pub size_after: Option<u64>,
    /// Mtime in first archive
    pub mtime_before: Option<i64>,
    /// Mtime in second archive
    pub mtime_after: Option<i64>,
}

impl DiffEntry {
    /// Create from a single file (added or removed)
    pub fn from_file(file: &CatalogFile, change_type: ChangeType) -> Self {
        match change_type {
            ChangeType::Added => Self {
                path: file.path.clone(),
                change_type,
                item_type: file.item_type,
                size_before: None,
                size_after: Some(file.size),
                mtime_before: None,
                mtime_after: Some(file.mtime),
            },
            ChangeType::Removed => Self {
                path: file.path.clone(),
                change_type,
                item_type: file.item_type,
                size_before: Some(file.size),
                size_after: None,
                mtime_before: Some(file.mtime),
                mtime_after: None,
            },
            ChangeType::Modified => unreachable!(),
        }
    }

    /// Create from two files (modified)
    pub fn from_files(before: &CatalogFile, after: &CatalogFile) -> Self {
        Self {
            path: after.path.clone(),
            change_type: ChangeType::Modified,
            item_type: after.item_type,
            size_before: Some(before.size),
            size_after: Some(after.size),
            mtime_before: Some(before.mtime),
            mtime_after: Some(after.mtime),
        }
    }
}

/// Result of comparing two archives
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveDiff {
    /// First archive name
    pub archive1: String,
    /// Second archive name
    pub archive2: String,
    /// Files added (in archive2 but not archive1)
    pub added: Vec<DiffEntry>,
    /// Files removed (in archive1 but not archive2)
    pub removed: Vec<DiffEntry>,
    /// Files modified
    pub modified: Vec<DiffEntry>,
    /// Count of unchanged files
    pub unchanged: usize,
}

impl ArchiveDiff {
    /// Get total number of changes
    pub fn total_changes(&self) -> usize {
        self.added.len() + self.removed.len() + self.modified.len()
    }

    /// Check if there are any changes
    pub fn has_changes(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty() || !self.modified.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_archive(name: &str, files: Vec<(&str, u64)>) -> Archive {
        use crate::archive::{ArchiveMetadata, ArchiveStats, UnixAttributes};

        let items: Vec<ArchiveItem> = files
            .into_iter()
            .map(|(path, size)| ArchiveItem {
                path: PathBuf::from(path),
                item_type: ItemType::File,
                size,
                attrs: UnixAttributes {
                    mode: 0o644,
                    uid: 1000,
                    gid: 1000,
                    atime: 0,
                    mtime: 0,
                    ctime: 0,
                },
                chunks: vec![],
                symlink_target: None,
                hardlink_target: None,
            })
            .collect();

        Archive {
            metadata: ArchiveMetadata {
                name: name.to_string(),
                time: chrono::Utc::now(),
                hostname: "test".to_string(),
                username: "user".to_string(),
                cmdline: vec![],
                comment: None,
                tags: None,
            },
            items,
            stats: ArchiveStats::default(),
        }
    }

    #[test]
    fn test_catalog_index_and_search() {
        let temp_dir = TempDir::new().unwrap();
        let catalog = Catalog::open(temp_dir.path()).unwrap();

        let archive = create_test_archive("test-archive", vec![
            ("file1.txt", 100),
            ("dir/file2.txt", 200),
            ("dir/subdir/file3.txt", 300),
        ]);

        catalog.index_archive(&archive).unwrap();

        // Search for files
        let query = SearchQuery::new().path("file");
        let results = catalog.search_files(&query).unwrap();
        assert_eq!(results.len(), 3);

        // Search with pattern
        let query = SearchQuery::new().path("subdir");
        let results = catalog.search_files(&query).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_archive_diff() {
        let temp_dir = TempDir::new().unwrap();
        let catalog = Catalog::open(temp_dir.path()).unwrap();

        let archive1 = create_test_archive("archive1", vec![
            ("file1.txt", 100),
            ("file2.txt", 200),
        ]);

        let archive2 = create_test_archive("archive2", vec![
            ("file2.txt", 250), // Modified
            ("file3.txt", 300), // Added
        ]);

        catalog.index_archive(&archive1).unwrap();
        catalog.index_archive(&archive2).unwrap();

        let diff = catalog.diff_archives("archive1", "archive2").unwrap();
        
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.modified.len(), 1);
    }
}
