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
use serde::{Deserialize, Serialize};
use nix::libc;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
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
    /// Path to the repository root
    path: PathBuf,
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
    /// Initialize a new repository at the given path
    #[instrument(skip(passphrase))]
    pub fn init(path: &Path, passphrase: Option<&str>, config: Option<RepositoryConfig>) -> Result<Self> {
        if path.exists() {
            return Err(BorgError::RepositoryExists {
                path: path.display().to_string(),
            });
        }

        let config = config.unwrap_or_default();
        
        info!("Initializing new repository at {}", path.display());

        // Create directory structure
        fs::create_dir_all(path)?;
        fs::create_dir_all(path.join("data"))?;

        // Create segment directories
        for i in 0..=255 {
            fs::create_dir_all(path.join("data").join(format!("{:02x}", i)))?;
        }

        // Save configuration
        let config_path = path.join("config");
        let config_data = serde_json::to_string_pretty(&config)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        fs::write(&config_path, config_data)?;

        // Handle encryption key
        let crypto = if config.encrypted {
            let passphrase = passphrase.ok_or_else(|| {
                BorgError::InvalidArgument("Passphrase required for encrypted repository".to_string())
            })?;

            let (repo_key, enc_key) = RepositoryKey::create(passphrase)?;
            let key_path = path.join("key");
            let key_data = serde_json::to_string_pretty(&repo_key)
                .map_err(|e| BorgError::Serialization(e.to_string()))?;
            fs::write(&key_path, key_data)?;

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
        let manifest_path = path.join("manifest");
        let manifest_data = serde_json::to_string_pretty(&manifest)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        fs::write(&manifest_path, manifest_data)?;

        let compressor = Compressor::new(config.compression.clone());

        info!("Repository initialized successfully");

        Ok(Self {
            path: path.to_path_buf(),
            config,
            crypto,
            compressor,
            chunk_index: HashSet::new(),
            lock: None,
        })
    }

    /// Open an existing repository
    #[instrument(skip(passphrase))]
    pub fn open(path: &Path, passphrase: Option<&str>) -> Result<Self> {
        if !path.exists() {
            return Err(BorgError::RepositoryNotFound {
                path: path.display().to_string(),
            });
        }

        info!("Opening repository at {}", path.display());

        // Load configuration
        let config_path = path.join("config");
        let config_data = fs::read_to_string(&config_path)?;
        let config: RepositoryConfig = serde_json::from_str(&config_data)
            .map_err(|e| BorgError::Deserialization(e.to_string()))?;

        // Load encryption key if encrypted
        let crypto = if config.encrypted {
            let passphrase = passphrase.ok_or(BorgError::InvalidPassphrase)?;

            let key_path = path.join("key");
            let key_data = fs::read_to_string(&key_path)?;
            let repo_key: RepositoryKey = serde_json::from_str(&key_data)
                .map_err(|e| BorgError::Deserialization(e.to_string()))?;

            let enc_key = repo_key.decrypt(passphrase)?;
            Some(CryptoProvider::new(enc_key))
        } else {
            None
        };

        let compressor = Compressor::new(config.compression.clone());

        // Load chunk index
        let chunk_index = Self::load_chunk_index(path)?;
        debug!("Loaded {} chunks from index", chunk_index.len());

        Ok(Self {
            path: path.to_path_buf(),
            config,
            crypto,
            compressor,
            chunk_index,
            lock: None,
        })
    }

    /// Load chunk index from disk
    fn load_chunk_index(path: &Path) -> Result<HashSet<ChunkId>> {
        let index_path = path.join("index");
        if !index_path.exists() {
            return Ok(HashSet::new());
        }

        let file = File::open(&index_path)?;
        let reader = BufReader::new(file);
        let index: Vec<[u8; 32]> = bincode::deserialize_from(reader)
            .map_err(|e| BorgError::Deserialization(e.to_string()))?;

        Ok(index.into_iter().map(ChunkId::new).collect())
    }

    /// Save chunk index to disk
    fn save_chunk_index(&self) -> Result<()> {
        let index_path = self.path.join("index");
        let file = File::create(&index_path)?;
        let writer = BufWriter::new(file);
        
        let index: Vec<[u8; 32]> = self.chunk_index.iter().map(|id| id.0).collect();
        bincode::serialize_into(writer, &index)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;

        Ok(())
    }

    /// Acquire a lock on the repository
    #[instrument(skip(self))]
    pub fn lock(&mut self, exclusive: bool) -> Result<()> {
        let lock_path = self.path.join("lock");

        // Check for existing lock
        if lock_path.exists() {
            let lock_data = fs::read_to_string(&lock_path)?;
            let existing_lock: LockInfo = serde_json::from_str(&lock_data)
                .map_err(|e| BorgError::Deserialization(e.to_string()))?;

            // Check if the lock is stale (process no longer exists)
            if !Self::is_process_alive(existing_lock.pid) {
                warn!("Removing stale lock from PID {}", existing_lock.pid);
                fs::remove_file(&lock_path)?;
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
        fs::write(&lock_path, lock_data)?;

        self.lock = Some(lock_info);
        debug!("Acquired {} lock", if exclusive { "exclusive" } else { "shared" });

        Ok(())
    }

    /// Release the repository lock
    pub fn unlock(&mut self) -> Result<()> {
        if self.lock.is_some() {
            let lock_path = self.path.join("lock");
            if lock_path.exists() {
                fs::remove_file(&lock_path)?;
            }
            self.lock = None;
            debug!("Released lock");
        }
        Ok(())
    }

    /// Check if a process is still alive
    #[cfg(unix)]
    fn is_process_alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    #[cfg(not(unix))]
    fn is_process_alive(_pid: u32) -> bool {
        // On non-Unix systems, assume process is alive
        true
    }

    /// Check if a chunk already exists in the repository
    pub fn has_chunk(&self, id: &ChunkId) -> bool {
        self.chunk_index.contains(id)
    }

    /// Get the path where a chunk would be stored
    fn chunk_path(&self, id: &ChunkId) -> PathBuf {
        let hex = id.to_hex();
        self.path
            .join("data")
            .join(&hex[..2])
            .join(&hex[2..])
    }

    /// Store a chunk in the repository
    #[instrument(skip(self, chunk), fields(chunk_id = %chunk.id))]
    pub fn put_chunk(&mut self, chunk: &Chunk) -> Result<bool> {
        // Check for deduplication
        if self.has_chunk(&chunk.id) {
            debug!("Chunk already exists, deduplicating");
            return Ok(false);
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

        // Write to disk
        let chunk_path = self.chunk_path(&chunk.id);
        if let Some(parent) = chunk_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&chunk_path, &data_to_store)?;

        // Update index
        self.chunk_index.insert(chunk.id.clone());

        debug!(
            "Stored chunk ({} -> {} bytes)",
            chunk.data.len(),
            data_to_store.len()
        );

        Ok(true)
    }

    /// Retrieve a chunk from the repository
    #[instrument(skip(self), fields(chunk_id = %id))]
    pub fn get_chunk(&self, id: &ChunkId) -> Result<Chunk> {
        let chunk_path = self.chunk_path(id);
        
        if !chunk_path.exists() {
            return Err(BorgError::Repository(format!(
                "Chunk not found: {}",
                id
            )));
        }

        let stored_data = fs::read(&chunk_path)?;

        // Decrypt if enabled
        let compressed: CompressedData = if let Some(ref crypto) = self.crypto {
            let encrypted: EncryptedData = bincode::deserialize(&stored_data)
                .map_err(|e| BorgError::Deserialization(e.to_string()))?;
            let decrypted = crypto.decrypt(&encrypted)?;
            bincode::deserialize(&decrypted)
                .map_err(|e| BorgError::Deserialization(e.to_string()))?
        } else {
            bincode::deserialize(&stored_data)
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
    pub fn load_manifest(&self) -> Result<Manifest> {
        let manifest_path = self.path.join("manifest");
        let manifest_data = fs::read_to_string(&manifest_path)?;
        serde_json::from_str(&manifest_data)
            .map_err(|e| BorgError::Deserialization(e.to_string()))
    }

    /// Save the manifest
    pub fn save_manifest(&self, manifest: &Manifest) -> Result<()> {
        let manifest_path = self.path.join("manifest");
        let manifest_data = serde_json::to_string_pretty(manifest)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        fs::write(&manifest_path, manifest_data)?;
        Ok(())
    }

    /// Get repository configuration
    pub fn config(&self) -> &RepositoryConfig {
        &self.config
    }

    /// Get repository path
    pub fn path(&self) -> &Path {
        &self.path
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
    pub fn commit(&self) -> Result<()> {
        self.save_chunk_index()?;
        // Update config timestamp
        let mut config = self.config.clone();
        config.last_modified = chrono::Utc::now();
        let config_path = self.path.join("config");
        let config_data = serde_json::to_string_pretty(&config)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        fs::write(&config_path, config_data)?;
        Ok(())
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        if let Err(e) = self.unlock() {
            warn!("Failed to release lock on drop: {}", e);
        }
    }
}

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
        nix::unistd::gethostname()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown".to_string())
    }
    #[cfg(not(unix))]
    {
        "unknown".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_repository_init_and_open() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().join("test-repo");

        // Initialize
        let _repo = Repository::init(&repo_path, Some("test-passphrase"), None).unwrap();

        // Open
        let repo = Repository::open(&repo_path, Some("test-passphrase")).unwrap();
        assert!(repo.config().encrypted);
    }

    #[test]
    fn test_chunk_storage() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().join("test-repo");

        let mut repo = Repository::init(&repo_path, Some("passphrase"), None).unwrap();

        let chunk = Chunk::new(b"Hello, Borg-Rust!".to_vec());
        let chunk_id = chunk.id.clone();

        // Store
        let is_new = repo.put_chunk(&chunk).unwrap();
        assert!(is_new);

        // Retrieve
        let retrieved = repo.get_chunk(&chunk_id).unwrap();
        assert_eq!(chunk.data, retrieved.data);

        // Deduplication
        let is_new = repo.put_chunk(&chunk).unwrap();
        assert!(!is_new);
    }

    #[test]
    fn test_unencrypted_repository() {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().join("test-repo");

        let config = RepositoryConfig {
            encrypted: false,
            ..Default::default()
        };

        let mut repo = Repository::init(&repo_path, None, Some(config)).unwrap();
        
        let chunk = Chunk::new(b"Unencrypted data".to_vec());
        repo.put_chunk(&chunk).unwrap();
        
        let retrieved = repo.get_chunk(&chunk.id).unwrap();
        assert_eq!(chunk.data, retrieved.data);
    }
}
