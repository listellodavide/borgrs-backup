//! Local caching for improved backup performance
//!
//! Caches file metadata and chunk information to speed up
//! subsequent backups by detecting unchanged files.

use crate::chunker::ChunkId;
use crate::error::{BorgError, Result};
use serde::{Deserialize, Serialize};
use sled::Db;
use std::path::{Path, PathBuf};
use tracing::{debug, instrument};

/// Cached file information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedFile {
    /// File path (relative)
    pub path: PathBuf,
    /// File size
    pub size: u64,
    /// Modification time (Unix timestamp)
    pub mtime: i64,
    /// Inode number (Unix)
    pub inode: u64,
    /// Chunk IDs for this file
    pub chunks: Vec<ChunkId>,
    /// Cache entry creation time
    pub cached_at: i64,
}

/// Cache for file metadata and chunks
pub struct FileCache {
    /// Sled database
    db: Db,
    /// Repository ID this cache belongs to
    repo_id: String,
    /// Cache statistics
    stats: CacheStats,
}

/// Cache statistics
#[derive(Debug, Default, Clone)]
pub struct CacheStats {
    /// Cache hits
    pub hits: u64,
    /// Cache misses
    pub misses: u64,
    /// Entries added
    pub additions: u64,
    /// Entries evicted
    pub evictions: u64,
}

impl CacheStats {
    /// Get hit rate as percentage
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }
}

impl FileCache {
    /// Open or create a cache at the given path
    #[instrument]
    pub fn open(cache_dir: &Path, repo_id: &str) -> Result<Self> {
        let db_path = cache_dir.join(format!("{}.cache", repo_id));
        
        std::fs::create_dir_all(cache_dir)?;
        
        let db = sled::open(&db_path)?;
        
        debug!("Opened cache at {}", db_path.display());
        
        Ok(Self {
            db,
            repo_id: repo_id.to_string(),
            stats: CacheStats::default(),
        })
    }

    /// Get cached file info if it matches current file state
    #[instrument(skip(self))]
    pub fn get(&mut self, path: &Path, size: u64, mtime: i64, inode: u64) -> Option<CachedFile> {
        let key = self.make_key(path);
        
        match self.db.get(&key) {
            Ok(Some(data)) => {
                match bincode::deserialize::<CachedFile>(&data) {
                    Ok(cached) => {
                        // Check if file has changed
                        if cached.size == size && cached.mtime == mtime && cached.inode == inode {
                            self.stats.hits += 1;
                            debug!("Cache hit for {}", path.display());
                            Some(cached)
                        } else {
                            self.stats.misses += 1;
                            debug!("Cache stale for {} (size/mtime/inode changed)", path.display());
                            None
                        }
                    }
                    Err(e) => {
                        debug!("Failed to deserialize cache entry: {}", e);
                        self.stats.misses += 1;
                        None
                    }
                }
            }
            Ok(None) => {
                self.stats.misses += 1;
                debug!("Cache miss for {}", path.display());
                None
            }
            Err(e) => {
                debug!("Cache lookup error: {}", e);
                self.stats.misses += 1;
                None
            }
        }
    }

    /// Store file info in cache
    #[instrument(skip(self, chunks))]
    pub fn put(&mut self, path: &Path, size: u64, mtime: i64, inode: u64, chunks: Vec<ChunkId>) -> Result<()> {
        let key = self.make_key(path);
        
        let cached = CachedFile {
            path: path.to_path_buf(),
            size,
            mtime,
            inode,
            chunks,
            cached_at: chrono::Utc::now().timestamp(),
        };
        
        let data = bincode::serialize(&cached)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        
        self.db.insert(&key, data)?;
        self.stats.additions += 1;
        
        debug!("Cached {} chunks for {}", cached.chunks.len(), path.display());
        
        Ok(())
    }

    /// Remove a file from cache
    pub fn remove(&mut self, path: &Path) -> Result<()> {
        let key = self.make_key(path);
        self.db.remove(&key)?;
        self.stats.evictions += 1;
        Ok(())
    }

    /// Clear all cache entries
    pub fn clear(&mut self) -> Result<()> {
        self.db.clear()?;
        self.stats.evictions += self.db.len() as u64;
        debug!("Cleared cache");
        Ok(())
    }

    /// Get cache statistics
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Get number of cached entries
    pub fn len(&self) -> usize {
        self.db.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.db.is_empty()
    }

    /// Flush cache to disk
    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }

    /// Create a cache key from a path
    fn make_key(&self, path: &Path) -> Vec<u8> {
        format!("{}:{}", self.repo_id, path.display()).into_bytes()
    }

    /// Iterate over all cached files
    pub fn iter(&self) -> impl Iterator<Item = CachedFile> + '_ {
        self.db.iter().filter_map(|result| {
            result.ok().and_then(|(_, data)| {
                bincode::deserialize(&data).ok()
            })
        })
    }

    /// Remove entries older than the given age (in seconds)
    pub fn evict_old(&mut self, max_age_secs: i64) -> Result<u64> {
        let cutoff = chrono::Utc::now().timestamp() - max_age_secs;
        let mut evicted = 0;
        
        let keys_to_remove: Vec<Vec<u8>> = self.db.iter()
            .filter_map(|result| {
                result.ok().and_then(|(key, data)| {
                    bincode::deserialize::<CachedFile>(&data)
                        .ok()
                        .filter(|cached| cached.cached_at < cutoff)
                        .map(|_| key.to_vec())
                })
            })
            .collect();
        
        for key in keys_to_remove {
            self.db.remove(&key)?;
            evicted += 1;
        }
        
        self.stats.evictions += evicted;
        debug!("Evicted {} old cache entries", evicted);
        
        Ok(evicted)
    }
}

/// Chunk index for the repository
pub struct ChunkIndex {
    /// Sled database
    db: Db,
}

impl ChunkIndex {
    /// Open or create a chunk index
    pub fn open(cache_dir: &Path, repo_id: &str) -> Result<Self> {
        let db_path = cache_dir.join(format!("{}.chunks", repo_id));
        
        std::fs::create_dir_all(cache_dir)?;
        
        let db = sled::open(&db_path)?;
        
        Ok(Self { db })
    }

    /// Check if a chunk exists
    pub fn contains(&self, id: &ChunkId) -> bool {
        self.db.contains_key(&id.0).unwrap_or(false)
    }

    /// Add a chunk to the index
    pub fn insert(&self, id: &ChunkId, size: u64) -> Result<()> {
        let data = bincode::serialize(&ChunkInfo { size, refcount: 1 })
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        self.db.insert(&id.0, data)?;
        Ok(())
    }

    /// Increment reference count for a chunk
    pub fn increment_refcount(&self, id: &ChunkId) -> Result<()> {
        if let Some(data) = self.db.get(&id.0)? {
            let mut info: ChunkInfo = bincode::deserialize(&data)
                .map_err(|e| BorgError::Deserialization(e.to_string()))?;
            info.refcount += 1;
            let new_data = bincode::serialize(&info)
                .map_err(|e| BorgError::Serialization(e.to_string()))?;
            self.db.insert(&id.0, new_data)?;
        }
        Ok(())
    }

    /// Get chunk info
    pub fn get(&self, id: &ChunkId) -> Option<ChunkInfo> {
        self.db.get(&id.0).ok().flatten().and_then(|data| {
            bincode::deserialize(&data).ok()
        })
    }

    /// Get total number of chunks
    pub fn len(&self) -> usize {
        self.db.len()
    }

    /// Check if index is empty
    pub fn is_empty(&self) -> bool {
        self.db.is_empty()
    }

    /// Flush to disk
    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }
}

/// Information about a stored chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkInfo {
    /// Size of the chunk
    pub size: u64,
    /// Reference count
    pub refcount: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_file_cache() {
        let temp_dir = TempDir::new().unwrap();
        let mut cache = FileCache::open(temp_dir.path(), "test-repo").unwrap();

        let path = Path::new("/test/file.txt");
        let chunks = vec![
            ChunkId::from_data(b"chunk1"),
            ChunkId::from_data(b"chunk2"),
        ];

        // Store
        cache.put(path, 1000, 12345, 67890, chunks.clone()).unwrap();

        // Retrieve (matching)
        let cached = cache.get(path, 1000, 12345, 67890).unwrap();
        assert_eq!(cached.chunks.len(), 2);
        assert_eq!(cache.stats().hits, 1);

        // Retrieve (changed)
        assert!(cache.get(path, 2000, 12345, 67890).is_none());
        assert_eq!(cache.stats().misses, 1);
    }

    #[test]
    fn test_chunk_index() {
        let temp_dir = TempDir::new().unwrap();
        let index = ChunkIndex::open(temp_dir.path(), "test-repo").unwrap();

        let chunk_id = ChunkId::from_data(b"test data");

        assert!(!index.contains(&chunk_id));

        index.insert(&chunk_id, 1000).unwrap();
        assert!(index.contains(&chunk_id));

        let info = index.get(&chunk_id).unwrap();
        assert_eq!(info.size, 1000);
        assert_eq!(info.refcount, 1);

        index.increment_refcount(&chunk_id).unwrap();
        let info = index.get(&chunk_id).unwrap();
        assert_eq!(info.refcount, 2);
    }
}
