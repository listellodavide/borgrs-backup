//! Repository management for storing backup data
//!
//! A repository is a directory structure containing:
//! - config: Repository configuration
//! - data/: Chunk storage (sharded by first 2 bytes of ID)
//! - index: Chunk index for deduplication
//! - hints: Optimization hints

use crate::chunker::{Chunk, ChunkId};
use crate::compression::{CompressedData, Compressor, CompressionConfig};
use crate::crypto::{CryptoProvider, EncryptedData, RepositoryKey};
use crate::error::{BorgError, Result};
use opendal::Operator;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tracing::{debug, info, instrument, warn};

/// Repository configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryConfig {
    /// Repository format version
    pub version: u32,
    /// Repository UUID
    pub id: String,
    /// Creation timestamp
    pub created: chrono::DateTime<chrono::Utc>,
    /// Last modified timestamp
    pub last_modified: chrono::DateTime<chrono::Utc>,
    /// Whether encryption is enabled
    pub encrypted: bool,
    /// Compression configuration
    pub compression: CompressionConfig,
    /// Additional segments for sharding
    pub segments_per_dir: u32,
}

impl Default for RepositoryConfig {
    fn default() -> Self {
        Self {
            version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            created: chrono::Utc::now(),
            last_modified: chrono::Utc::now(),
            encrypted: true,
            compression: CompressionConfig::default(),
            segments_per_dir: 256,
        }
    }
}

/// Simple UUID generation
mod uuid {
    use rand::RngCore;

    pub struct Uuid([u8; 16]);

    impl Uuid {
        pub fn new_v4() -> Self {
            let mut bytes = [0u8; 16];
            rand::rngs::OsRng.fill_bytes(&mut bytes);
            // Set version (4) and variant bits
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            Self(bytes)
        }

        pub fn to_string(&self) -> String {
            format!(
                "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                self.0[0], self.0[1], self.0[2], self.0[3],
                self.0[4], self.0[5],
                self.0[6], self.0[7],
                self.0[8], self.0[9],
                self.0[10], self.0[11], self.0[12], self.0[13], self.0[14], self.0[15]
            )
        }
    }
}

/// Manifest containing all archives in the repository
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Manifest {
    /// Version of the manifest format
    pub version: u32,
    /// Repository ID (must match config)
    pub repository_id: String,
    /// List of archive IDs
    pub archives: Vec<ArchiveRef>,
    /// Timestamp of last modification
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Reference to an archive
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveRef {
    /// Archive name
    pub name: String,
    /// Archive ID (chunk containing archive metadata)
    pub id: ChunkId,
    /// Creation time
    pub time: chrono::DateTime<chrono::Utc>,
}

/// Lock file for concurrent access control
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockInfo {
    /// PID of the process holding the lock
    pub pid: u32,
    /// Hostname of the machine
    pub hostname: String,
    /// Lock acquisition time
    pub time: chrono::DateTime<chrono::Utc>,
    /// Whether this is an exclusive lock
    pub exclusive: bool,
}

/// Repository handle for backup operations
pub struct Repository {
    /// OpenDAL operator for storage access
    pub op: Operator,
    /// Repository configuration
    config: RepositoryConfig,
    /// Encryption provider (if enabled)
    crypto: Option<CryptoProvider>,
    /// Compression provider
    compressor: Compressor,
    /// Set of known chunk IDs (loaded from index)
    pub(crate) chunk_index: HashSet<ChunkId>,
    /// Lock information
    lock: Option<LockInfo>,
}

impl Repository {
    /// Initialize a new repository using the given operator
    #[instrument(skip(passphrase))]
    pub async fn init(op: Operator, passphrase: Option<&str>, config: Option<RepositoryConfig>) -> Result<Self> {
        if op.exists("config").await.map_err(|e| BorgError::Repository(e.to_string()))? {
            return Err(BorgError::RepositoryExists {
                path: "remote".to_string(),
            });
        }

        let config = config.unwrap_or_default();
        
        info!("Initializing new repository");

        // Create directory structure (not strictly necessary with some OpenDAL backends but good for layout)
        op.create_dir("data/").await.map_err(|e| BorgError::Repository(e.to_string()))?;

        // Save configuration
        let config_data = serde_json::to_string_pretty(&config)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        op.write("config", config_data).await.map_err(|e| BorgError::Repository(e.to_string()))?;

        // Handle encryption key
        let crypto = if config.encrypted {
            let passphrase = passphrase.ok_or_else(|| {
                BorgError::InvalidArgument("Passphrase required for encrypted repository".to_string())
            })?;

            let (repo_key, enc_key) = RepositoryKey::create(passphrase)?;
            let key_data = serde_json::to_string_pretty(&repo_key)
                .map_err(|e| BorgError::Serialization(e.to_string()))?;
            op.write("key", key_data).await.map_err(|e| BorgError::Repository(e.to_string()))?;

            Some(CryptoProvider::new(enc_key))
        } else {
            None
        };

        // Initialize empty manifest
        let manifest = Manifest {
            version: 1,
            repository_id: config.id.clone(),
            archives: Vec::new(),
            timestamp: chrono::Utc::now(),
        };
        let manifest_data = serde_json::to_string_pretty(&manifest)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        op.write("manifest", manifest_data).await.map_err(|e| BorgError::Repository(e.to_string()))?;

        let compressor = Compressor::new(config.compression.clone());

        info!("Repository initialized successfully");

        Ok(Self {
            op,
            config,
            crypto,
            compressor,
            chunk_index: HashSet::new(),
            lock: None,
        })
    }

    /// Open an existing repository
    #[instrument(skip(passphrase))]
    pub async fn open(op: Operator, passphrase: Option<&str>) -> Result<Self> {
        if !op.exists("config").await.map_err(|e| BorgError::Repository(e.to_string()))? {
            return Err(BorgError::RepositoryNotFound {
                path: "remote".to_string(),
            });
        }

        info!("Opening repository");

        // Load configuration
        let config_data = op.read("config").await.map_err(|e| BorgError::Repository(e.to_string()))?;
        let config: RepositoryConfig = serde_json::from_slice(&config_data.to_vec())
            .map_err(|e| BorgError::Deserialization(e.to_string()))?;

        // Load encryption key if encrypted
        let crypto = if config.encrypted {
            let passphrase = passphrase.ok_or(BorgError::InvalidPassphrase)?;

            let key_data = op.read("key").await.map_err(|e| BorgError::Repository(e.to_string()))?;
            let repo_key: RepositoryKey = serde_json::from_slice(&key_data.to_vec())
                .map_err(|e| BorgError::Deserialization(e.to_string()))?;

            let enc_key = repo_key.decrypt(passphrase)?;
            Some(CryptoProvider::new(enc_key))
        } else {
            None
        };

        let compressor = Compressor::new(config.compression.clone());

        // Load chunk index
        let chunk_index = Self::load_chunk_index(&op).await?;
        debug!("Loaded {} chunks from index", chunk_index.len());

        Ok(Self {
            op,
            config,
            crypto,
            compressor,
            chunk_index,
            lock: None,
        })
    }

    /// Load chunk index from storage
    async fn load_chunk_index(op: &Operator) -> Result<HashSet<ChunkId>> {
        if !op.exists("index").await.map_err(|e| BorgError::Repository(e.to_string()))? {
            return Ok(HashSet::new());
        }

        let index_data = op.read("index").await.map_err(|e| BorgError::Repository(e.to_string()))?;
        let index: Vec<[u8; 32]> = bincode::deserialize(&index_data.to_vec())
            .map_err(|e| BorgError::Deserialization(e.to_string()))?;

        Ok(index.into_iter().map(ChunkId::new).collect())
    }

    /// Save chunk index to storage
    async fn save_chunk_index(&self) -> Result<()> {
        let index: Vec<[u8; 32]> = self.chunk_index.iter().map(|id| id.0).collect();
        let index_data = bincode::serialize(&index)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        
        self.op.write("index", index_data).await.map_err(|e| BorgError::Repository(e.to_string()))?;

        Ok(())
    }

    /// Acquire a lock on the repository
    #[instrument(skip(self))]
    pub async fn lock(&mut self, exclusive: bool) -> Result<()> {
        // Check for existing lock
        if self.op.exists("lock").await.map_err(|e| BorgError::Repository(e.to_string()))? {
            let lock_data = self.op.read("lock").await.map_err(|e| BorgError::Repository(e.to_string()))?;
            let existing_lock: LockInfo = serde_json::from_slice(&lock_data.to_vec())
                .map_err(|e| BorgError::Deserialization(e.to_string()))?;

            // Check if the lock is stale (process no longer exists)
            if !Self::is_process_alive(existing_lock.pid) {
                warn!("Removing stale lock from PID {}", existing_lock.pid);
                self.op.delete("lock").await.map_err(|e| BorgError::Repository(e.to_string()))?;
            } else if exclusive || existing_lock.exclusive {
                return Err(BorgError::RepositoryLocked);
            }
        }

        let lock_info = LockInfo {
            pid: std::process::id(),
            hostname: gethostname(),
            time: chrono::Utc::now(),
            exclusive,
        };

        let lock_data = serde_json::to_string_pretty(&lock_info)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        self.op.write("lock", lock_data).await.map_err(|e| BorgError::Repository(e.to_string()))?;

        self.lock = Some(lock_info);
        debug!("Acquired {} lock", if exclusive { "exclusive" } else { "shared" });

        Ok(())
    }

    /// Release the repository lock
    pub async fn unlock(&mut self) -> Result<()> {
        if self.lock.is_some() {
            if self.op.exists("lock").await.unwrap_or(false) {
                self.op.delete("lock").await.map_err(|e| BorgError::Repository(e.to_string()))?;
            }
            self.lock = None;
            debug!("Released lock");
        }
        Ok(())
    }

    /// Check if a process is still alive (Mocked for simplified Cross-Platform)
    fn is_process_alive(_pid: u32) -> bool {
        // In a real implementation, we would use platform-specific APIs
        // or a crate like `sysinfo`. For now, we assume alive to be safe,
        // or always true for remote backends where PID doesn't make sense.
        true
    }

    /// Check if a chunk already exists in the repository
    pub fn has_chunk(&self, id: &ChunkId) -> bool {
        self.chunk_index.contains(id)
    }

    /// Get the path where a chunk would be stored
    fn chunk_path(&self, id: &ChunkId) -> String {
        let hex = id.to_hex();
        format!("data/{}/{}", &hex[..2], &hex[2..])
    }

    /// Store a chunk in the repository
    #[instrument(skip(self, chunk), fields(chunk_id = %chunk.id))]
    pub async fn put_chunk(&mut self, chunk: &Chunk) -> Result<(bool, u64)> {
        // Check for deduplication
        if self.has_chunk(&chunk.id) {
            debug!("Chunk already exists, deduplicating");
            return Ok((false, 0));
        }

        // Compress
        let compressed = self.compressor.compress(&chunk.data)?;

        // Encrypt if enabled
        let data_to_store = if let Some(ref crypto) = self.crypto {
            let encrypted = crypto.encrypt(&bincode::serialize(&compressed)
                .map_err(|e| BorgError::Serialization(e.to_string()))?)?;
            bincode::serialize(&encrypted)
                .map_err(|e| BorgError::Serialization(e.to_string()))?
        } else {
            bincode::serialize(&compressed)
                .map_err(|e| BorgError::Serialization(e.to_string()))?
        };

        // Write to storage
        let chunk_path = self.chunk_path(&chunk.id);
        self.op.write(&chunk_path, data_to_store.clone()).await.map_err(|e| BorgError::Repository(e.to_string()))?;

        // Update index
        self.chunk_index.insert(chunk.id.clone());

        debug!(
            "Stored chunk ({} -> {} bytes)",
            chunk.data.len(),
            data_to_store.len()
        );

        Ok((true, data_to_store.len() as u64))
    }

    /// Retrieve a chunk from the repository
    #[instrument(skip(self), fields(chunk_id = %id))]
    pub async fn get_chunk(&self, id: &ChunkId) -> Result<Chunk> {
        let chunk_path = self.chunk_path(id);
        
        let stored_data = self.op.read(&chunk_path).await.map_err(|e| BorgError::Repository(format!(
            "Chunk not found or error reading: {} ({})",
            id, e
        )))?;

        // Decrypt if enabled
        let compressed: CompressedData = if let Some(ref crypto) = self.crypto {
            let encrypted: EncryptedData = bincode::deserialize(&stored_data.to_vec())
                .map_err(|e| BorgError::Deserialization(e.to_string()))?;
            let decrypted = crypto.decrypt(&encrypted)?;
            bincode::deserialize(&decrypted)
                .map_err(|e| BorgError::Deserialization(e.to_string()))?
        } else {
            bincode::deserialize(&stored_data.to_vec())
                .map_err(|e| BorgError::Deserialization(e.to_string()))?
        };

        // Decompress
        let data = self.compressor.decompress(&compressed)?;

        // Verify integrity
        let computed_id = ChunkId::from_data(&data);
        if computed_id != *id {
            return Err(BorgError::IntegrityCheck {
                expected: id.to_hex(),
                actual: computed_id.to_hex(),
            });
        }

        Ok(Chunk {
            id: id.clone(),
            data,
            original_size: compressed.original_size as usize,
        })
    }

    /// Load the manifest
    pub async fn load_manifest(&self) -> Result<Manifest> {
        let manifest_data = self.op.read("manifest").await.map_err(|e| BorgError::Repository(e.to_string()))?;
        serde_json::from_slice(&manifest_data.to_vec())
            .map_err(|e| BorgError::Deserialization(e.to_string()))
    }

    /// Save the manifest
    pub async fn save_manifest(&self, manifest: &Manifest) -> Result<()> {
        let manifest_data = serde_json::to_string_pretty(manifest)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        self.op.write("manifest", manifest_data).await.map_err(|e| BorgError::Repository(e.to_string()))?;
        Ok(())
    }

    /// Get repository configuration
    pub fn config(&self) -> &RepositoryConfig {
        &self.config
    }

    /// Get statistics about the repository
    pub fn stats(&self) -> RepositoryStats {
        RepositoryStats {
            total_chunks: self.chunk_index.len() as u64,
            // Would need to iterate to get actual sizes
            total_size: 0,
            compressed_size: 0,
        }
    }

    /// Commit changes (save index, etc.)
    pub async fn commit(&self) -> Result<()> {
        self.save_chunk_index().await?;
        // Update config timestamp
        let mut config = self.config.clone();
        config.last_modified = chrono::Utc::now();
        let config_data = serde_json::to_string_pretty(&config)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        self.op.write("config", config_data).await.map_err(|e| BorgError::Repository(e.to_string()))?;
        Ok(())
    }

    /// Delete an archive by name
    pub async fn delete_archive(&mut self, name: &str) -> Result<()> {
        let mut manifest = self.load_manifest().await?;
        
        if let Some(pos) = manifest.archives.iter().position(|a| a.name == name) {
            let archive_ref = manifest.archives.remove(pos);
            
            // Delete the archive metadata chunk
            // In OpenDAL, we just use the ID as the path under chunks/
            self.op.delete(&format!("data/chunks/{}", archive_ref.id.to_hex())).await
                .map_err(|e| BorgError::Repository(e.to_string()))?;
            
            manifest.timestamp = chrono::Utc::now();
            self.save_manifest(&manifest).await?;
            self.commit().await?;
            
            info!("Deleted archive '{}'", name);
            Ok(())
        } else {
            Err(BorgError::ArchiveNotFound {
                name: name.to_string(),
            })
        }
    }
}

// NOTE: Drop impl removed because it's sync and Repository is now async.
// Locks will need to be explicitly managed or we need an async drop strategy.

/// Repository statistics
#[derive(Debug, Clone, Default)]
pub struct RepositoryStats {
    /// Total number of chunks
    pub total_chunks: u64,
    /// Total original size of all chunks
    pub total_size: u64,
    /// Total compressed size
    pub compressed_size: u64,
}

impl RepositoryStats {
    /// Get compression ratio
    pub fn compression_ratio(&self) -> f64 {
        if self.total_size == 0 {
            1.0
        } else {
            self.compressed_size as f64 / self.total_size as f64
        }
    }
}

/// Get the system hostname
fn gethostname() -> String {
    #[cfg(unix)]
    {
        // Simple hostname fallback without nix
        std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string())
    }
    #[cfg(not(unix))]
    {
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use crate::storage::{StorageConfig, build_operator};
    use crate::chunker::Chunk;

    async fn get_test_op(temp_dir: &TempDir) -> Operator {
        let repo_path = temp_dir.path().join("test-repo");
        build_operator(StorageConfig::Local { path: repo_path }).unwrap()
    }

    #[tokio::test]
    async fn test_repository_init_and_open() {
        let temp_dir = TempDir::new().unwrap();
        let op = get_test_op(&temp_dir).await;

        // Initialize
        let _repo = Repository::init(op.clone(), Some("test-passphrase"), None).await.unwrap();

        // Open
        let repo = Repository::open(op, Some("test-passphrase")).await.unwrap();
        assert!(repo.config().encrypted);
    }

    #[tokio::test]
    async fn test_chunk_storage() {
        let temp_dir = TempDir::new().unwrap();
        let op = get_test_op(&temp_dir).await;

        let mut repo = Repository::init(op, Some("passphrase"), None).await.unwrap();

        let chunk = Chunk::new(b"Hello, Borg-Rust!".to_vec());
        let chunk_id = chunk.id.clone();

        // Store
        let (is_new, _) = repo.put_chunk(&chunk).await.unwrap();
        assert!(is_new);

        // Retrieve
        let retrieved = repo.get_chunk(&chunk_id).await.unwrap();
        assert_eq!(chunk.data, retrieved.data);

        // Deduplication
        let (is_new, _) = repo.put_chunk(&chunk).await.unwrap();
        assert!(!is_new);
    }
}
