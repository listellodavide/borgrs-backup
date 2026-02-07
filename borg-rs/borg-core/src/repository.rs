//! Repository management for storing backup data
//!
//! Implements "Borg NG" architecture:
//! - Immutable Object Storage (CAS)
//! - Append-only Snapshots
//! - No global locks, no renames

use crate::chunker::{Chunk, ChunkId};
use crate::compression::{CompressedData, Compressor, CompressionConfig};
use crate::crypto::{CryptoProvider, EncryptedData, RepositoryKey};
use crate::error::{BorgError, Result};
use crate::metadata::{Snapshot, Tree, EntryKind, RepositoryStats};
use crate::recovery::{RecoveryCodec, RecoveryProfile};
use async_trait::async_trait;
use futures_util::{StreamExt, TryStreamExt};
use opendal::Operator;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use tracing::{debug, info, instrument, warn};
use hex;

/// Legacy Manifest structure for backward compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub archives: Vec<ArchiveRef>,
}

/// Reference to an archive in the manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveRef {
    pub name: String,
    pub id: ChunkId,
    pub time: chrono::DateTime<chrono::Utc>,
}

/// Repository Descriptor (Immutable, written once at init)
/// Replaces the mutable 'config' file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoDescriptor {
    pub version: u32,
    pub id: String,
    pub created: chrono::DateTime<chrono::Utc>,
    pub storage_engine: String,
    pub encrypted: bool,
    pub compression: CompressionConfig,
    #[serde(default)]
    pub recovery_profile: Option<RecoveryProfile>,
}

impl Default for RepoDescriptor {
    fn default() -> Self {
        Self {
            version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            created: chrono::Utc::now(),
            storage_engine: "object-log-v1".to_string(),
            encrypted: true,
            compression: CompressionConfig::default(),
            recovery_profile: None,
        }
    }
}

/// Storage Engine Trait
/// Abstracts the layout strategy (e.g., ObjectLogV1 vs Posix)
#[async_trait]
pub trait StorageEngine: Send + Sync {
    /// Initialize storage layout
    async fn init(&self, descriptor: &RepoDescriptor) -> Result<()>;
    /// Load repository descriptor
    async fn load_descriptor(&self) -> Result<RepoDescriptor>;
    /// Check if object exists (for deduplication)
    async fn has_object(&self, id: &ChunkId) -> Result<bool>;
    /// Write an immutable object
    async fn put_object(&self, id: &ChunkId, data: &[u8]) -> Result<()>;
    /// Read an immutable object
    async fn get_object(&self, id: &ChunkId) -> Result<Vec<u8>>;
    /// Commit a new snapshot
    async fn put_snapshot(&self, snapshot: &Snapshot) -> Result<()>;
    /// List all snapshots
    async fn list_snapshots(&self) -> Result<Vec<Snapshot>>;
    /// List all objects (for GC)
    async fn list_objects(&self) -> Result<Vec<ChunkId>>;
    /// Delete an object (for GC)
    async fn delete_object(&self, id: &ChunkId) -> Result<()>;
    /// Delete a snapshot
    async fn delete_snapshot(&self, id: &str) -> Result<()>;
    /// Write recovery data
    async fn put_recovery(&self, id: &ChunkId, data: &[u8]) -> Result<()>;
    /// Read recovery data
    async fn get_recovery(&self, id: &ChunkId) -> Result<Vec<u8>>;
}

/// ObjectLog V1 Engine
/// Layout:
///   repo.json
///   objects/
///     aa/
///       bb... (chunk data)
///   snapshots/
///     2026-02-07T01:22:11Z.json
pub struct ObjectLogV1 {
    op: Operator,
}

impl ObjectLogV1 {
    pub fn new(op: Operator) -> Self {
        Self { op }
    }

    fn object_path(&self, id: &ChunkId) -> String {
        let hex = id.to_hex();
        format!("objects/{}/{}", &hex[..2], &hex[2..])
    }

    fn recovery_path(&self, id: &ChunkId) -> String {
        let hex = id.to_hex();
        format!("recovery/{}/{}.parr", &hex[..2], &hex[2..])
    }

    fn parse_id_from_path(&self, path: &str) -> Option<ChunkId> {
        // path: objects/aa/bb...
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 3 && parts[0] == "objects" {
            let hex = format!("{}{}", parts[1], parts[2]);
            if let Ok(bytes) = hex::decode(&hex) {
                return Some(ChunkId::new(bytes.try_into().ok()?));
            }
        }
        None
    }
}

#[async_trait]
impl StorageEngine for ObjectLogV1 {
    async fn init(&self, descriptor: &RepoDescriptor) -> Result<()> {
        let data = serde_json::to_vec_pretty(descriptor)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        self.op.write("repo.json", data).await
            .map_err(|e| BorgError::Repository(e.to_string()))?;
        Ok(())
    }

    async fn load_descriptor(&self) -> Result<RepoDescriptor> {
        let data = self.op.read("repo.json").await
            .map_err(|e| BorgError::Repository(e.to_string()))?;
        serde_json::from_slice(&data.to_vec())
            .map_err(|e| BorgError::Deserialization(e.to_string()))
    }

    async fn has_object(&self, id: &ChunkId) -> Result<bool> {
        // In a real implementation, we might use a local cache here
        // to avoid HEAD requests for every chunk.
        self.op.exists(&self.object_path(id)).await
            .map_err(|e| BorgError::Repository(e.to_string()))
    }

    async fn put_object(&self, id: &ChunkId, data: &[u8]) -> Result<()> {
        self.op.write(&self.object_path(id), data.to_vec()).await
            .map_err(|e| BorgError::Repository(e.to_string()))
    }

    async fn get_object(&self, id: &ChunkId) -> Result<Vec<u8>> {
        let data = self.op.read(&self.object_path(id)).await
            .map_err(|e| BorgError::Repository(e.to_string()))?;
        Ok(data.to_vec())
    }

    async fn put_snapshot(&self, snapshot: &Snapshot) -> Result<()> {
        let path = format!("snapshots/{}.json", snapshot.id);
        let data = serde_json::to_vec_pretty(snapshot)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        self.op.write(&path, data).await
            .map_err(|e| BorgError::Repository(e.to_string()))
    }

    async fn list_snapshots(&self) -> Result<Vec<Snapshot>> {
        let entries = self.op.list("snapshots/").await
            .map_err(|e| BorgError::Repository(e.to_string()))?;

        let op = self.op.clone();
        
        // Create a stream of futures to fetch snapshots in parallel
        let futures = entries.into_iter()
            .filter(|e| e.path().ends_with(".json"))
            .map(|entry| {
                let op = op.clone();
                async move {
                    let data = op.read(entry.path()).await
                        .map_err(|e| BorgError::Repository(e.to_string()))?;
                    let snap: Snapshot = serde_json::from_slice(&data.to_vec())
                        .map_err(|e| BorgError::Deserialization(e.to_string()))?;
                    Ok::<_, BorgError>(snap)
                }
            });

        // Execute up to 10 fetches concurrently
        let snapshots: Vec<Snapshot> = futures_util::stream::iter(futures)
            .buffer_unordered(10)
            .filter_map(|res| async {
                match res {
                    Ok(snap) => Some(snap),
                    Err(e) => {
                        warn!("Failed to load snapshot: {}", e);
                        None
                    }
                }
            })
            .collect()
            .await;

        Ok(snapshots)
    }

    async fn list_objects(&self) -> Result<Vec<ChunkId>> {
        let mut chunks = Vec::new();
        // Recursive scan of objects/
        let mut lister = self.op.lister_with("objects/").recursive(true).await
            .map_err(|e| BorgError::Repository(e.to_string()))?;
        
        while let Some(entry) = lister.try_next().await.map_err(|e| BorgError::Repository(e.to_string()))? {
            if entry.metadata().mode().is_file() {
                if let Some(id) = self.parse_id_from_path(entry.path()) {
                    chunks.push(id);
                }
            }
        }
        Ok(chunks)
    }

    async fn delete_object(&self, id: &ChunkId) -> Result<()> {
        self.op.delete(&self.object_path(id)).await
            .map_err(|e| BorgError::Repository(e.to_string()))
    }

    async fn delete_snapshot(&self, id: &str) -> Result<()> {
        let path = format!("snapshots/{}.json", id);
        self.op.delete(&path).await
            .map_err(|e| BorgError::Repository(e.to_string()))
    }

    async fn put_recovery(&self, id: &ChunkId, data: &[u8]) -> Result<()> {
        self.op.write(&self.recovery_path(id), data.to_vec()).await
            .map_err(|e| BorgError::Repository(e.to_string()))
    }

    async fn get_recovery(&self, id: &ChunkId) -> Result<Vec<u8>> {
        let data = self.op.read(&self.recovery_path(id)).await
            .map_err(|e| BorgError::Repository(e.to_string()))?;
        Ok(data.to_vec())
    }
}

/// Repository handle
pub struct Repository {
    engine: Box<dyn StorageEngine>,
    descriptor: RepoDescriptor,
    crypto: Option<CryptoProvider>,
    compressor: Compressor,
    // Local cache of known chunks to avoid remote lookups
    // In a full implementation, this would be persisted locally (e.g. SQLite/Sled)
    pub(crate) chunk_cache: HashSet<ChunkId>,
    recovery_codec: Option<RecoveryCodec>,
}

impl Repository {
    #[instrument(skip(passphrase))]
    pub async fn init(op: Operator, path: String, passphrase: Option<&str>, descriptor: Option<RepoDescriptor>) -> Result<Self> {
        // Probe capabilities (conceptually)
        // let caps = crate::storage::probe_capabilities(&op).await;
        
        if op.exists("repo.json").await.map_err(|e| BorgError::Repository(e.to_string()))? {
            return Err(BorgError::RepositoryExists { path });
        }

        let descriptor = descriptor.unwrap_or_default();
        let engine = Box::new(ObjectLogV1::new(op.clone()));
        
        info!("Initializing new repository at {}", path);
        engine.init(&descriptor).await?;

        // Handle encryption key
        let crypto = if descriptor.encrypted {
            let passphrase = passphrase.ok_or_else(|| {
                BorgError::InvalidArgument("Passphrase required for encrypted repository".to_string())
            })?;

            let (repo_key, enc_key) = RepositoryKey::create(passphrase)?;
            let key_data = serde_json::to_string_pretty(&repo_key)
                .map_err(|e| BorgError::Serialization(e.to_string()))?;
            
            // Key is just another object, but we store it at root for bootstrap
            op.write("key", key_data).await.map_err(|e| BorgError::Repository(e.to_string()))?;

            Some(CryptoProvider::new(enc_key))
        } else {
            None
        };

        let compressor = Compressor::new(descriptor.compression.clone());

        let recovery_codec = if let Some(profile) = descriptor.recovery_profile {
            Some(RecoveryCodec::new(profile)?)
        } else {
            None
        };

        info!("Repository initialized successfully");

        Ok(Self {
            engine,
            descriptor,
            crypto,
            compressor,
            chunk_cache: HashSet::new(),
            recovery_codec,
        })
    }

    #[instrument(skip(passphrase))]
    pub async fn open(op: Operator, path: String, passphrase: Option<&str>) -> Result<Self> {
        if !op.exists("repo.json").await.map_err(|e| BorgError::Repository(e.to_string()))? {
            return Err(BorgError::RepositoryNotFound { path });
        }

        info!("Opening repository at {}", path);
        let engine = Box::new(ObjectLogV1::new(op.clone()));

        let descriptor = engine.load_descriptor().await?;

        let crypto = if descriptor.encrypted {
            let passphrase = passphrase.ok_or(BorgError::InvalidPassphrase)?;

            let key_data = op.read("key").await.map_err(|e| BorgError::Repository(e.to_string()))?;
            let repo_key: RepositoryKey = serde_json::from_slice(&key_data.to_vec())
                .map_err(|e| BorgError::Deserialization(e.to_string()))?;

            let enc_key = repo_key.decrypt(passphrase)?;
            Some(CryptoProvider::new(enc_key))
        } else {
            None
        };

        let compressor = Compressor::new(descriptor.compression.clone());

        let recovery_codec = if let Some(profile) = descriptor.recovery_profile {
            Some(RecoveryCodec::new(profile)?)
        } else {
            None
        };

        Ok(Self {
            engine,
            descriptor,
            crypto,
            compressor,
            chunk_cache: HashSet::new(),
            recovery_codec,
        })
    }

    pub fn has_chunk(&self, id: &ChunkId) -> bool {
        // Check local cache first
        self.chunk_cache.contains(id)
    }

    #[instrument(skip(self, chunk), fields(chunk_id = %chunk.id))]
    pub async fn put_chunk(&mut self, chunk: &Chunk) -> Result<(bool, u64)> {
        // Check for deduplication
        if self.has_chunk(&chunk.id) {
            debug!("Chunk already exists, deduplicating");
            return Ok((false, 0));
        }
        
        // Double check with remote (optional, but good for safety)
        if self.engine.has_object(&chunk.id).await? {
            self.chunk_cache.insert(chunk.id.clone());
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
        self.engine.put_object(&chunk.id, &data_to_store).await?;

        // Generate and write recovery data if enabled
        if let Some(codec) = &self.recovery_codec {
            let parity = codec.encode(&data_to_store)?;
            self.engine.put_recovery(&chunk.id, &parity).await?;
        }

        // Update index
        self.chunk_cache.insert(chunk.id.clone());

        debug!(
            "Stored chunk ({} -> {} bytes)",
            chunk.data.len(),
            data_to_store.len()
        );

        Ok((true, data_to_store.len() as u64))
    }

    #[instrument(skip(self), fields(chunk_id = %id))]
    pub async fn get_chunk(&self, id: &ChunkId) -> Result<Chunk> {
        let stored_data = self.engine.get_object(id).await?;

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
            // Integrity check failed, attempt recovery if enabled
            if let Some(codec) = &self.recovery_codec {
                warn!("Chunk {} corrupted, attempting recovery...", id);
                let parity = self.engine.get_recovery(id).await?;
                let recovered_data = codec.reconstruct(&stored_data, &parity)?;
                
                // Verify recovered data
                // Note: We recovered the *encrypted/compressed* blob. We need to decrypt/decompress again to check ID?
                // No, we should check if the recovered blob matches what we expect? 
                // Actually, we can just proceed with the recovered blob as if it was what we read.
                // But we need to re-run the decryption/decompression pipeline on the recovered blob.
                // For simplicity, we return a recursive call or just re-run the logic.
                // Since we are at the end of the function, let's just return the recovered chunk if it passes.
                // However, we need to decrypt/decompress the *recovered* blob.
                // Let's recurse once? No, infinite loop risk.
                // Let's just re-process `recovered_data` (which is `stored_data` equivalent).
                
                // Decrypt recovered
                let compressed: CompressedData = if let Some(ref crypto) = self.crypto {
                    let encrypted: EncryptedData = bincode::deserialize(&recovered_data)
                        .map_err(|e| BorgError::Deserialization(e.to_string()))?;
                    let decrypted = crypto.decrypt(&encrypted)?;
                    bincode::deserialize(&decrypted)
                        .map_err(|e| BorgError::Deserialization(e.to_string()))?
                } else {
                    bincode::deserialize(&recovered_data)
                        .map_err(|e| BorgError::Deserialization(e.to_string()))?
                };

                // Decompress recovered
                let data = self.compressor.decompress(&compressed)?;
                
                // Verify integrity of recovered data
                let computed_id = ChunkId::from_data(&data);
                if computed_id != *id {
                     return Err(BorgError::IntegrityCheck {
                        expected: id.to_hex(),
                        actual: computed_id.to_hex(),
                    });
                }
                
                info!("Chunk {} successfully recovered", id);
                return Ok(Chunk {
                    id: id.clone(),
                    data,
                    original_size: compressed.original_size as usize,
                });
            }

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

    /// Commit a new snapshot
    pub async fn commit_snapshot(&self, snapshot: &Snapshot) -> Result<()> {
        self.engine.put_snapshot(snapshot).await
    }

    /// Commit an archive (compatibility wrapper)
    /// This creates a Snapshot from the archive metadata and stores it.
    pub async fn commit_archive(&mut self, archive: crate::archive::Archive) -> Result<()> {
        // In the new model, the archive metadata itself is stored as a chunk (already done in ArchiveCreator)
        // We just need to create a Snapshot pointing to it.
        // However, ArchiveCreator stores the archive struct as a chunk.
        // We need to find the root tree ID.
        // For now, let's assume the archive chunk IS the root of the snapshot for compatibility.

        // Re-serialize to get ID (inefficient but safe)
        let archive_data = bincode::serialize(&archive)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        let archive_chunk = Chunk::new(archive_data);
        let archive_id = archive_chunk.id.clone();

        // Ensure it's stored
        self.put_chunk(&archive_chunk).await?;

        let snapshot = Snapshot {
            id: uuid::Uuid::new_v4().to_string(),
            name: archive.metadata.name.clone(),
            time: archive.metadata.time,
            root_tree: archive_id, // Pointing to the archive metadata chunk
            hostname: archive.metadata.hostname,
            username: archive.metadata.username,
            cmdline: archive.metadata.cmdline,
            comment: archive.metadata.comment,
        };

        self.commit_snapshot(&snapshot).await
    }

    /// Load manifest (compatibility wrapper)
    /// Constructs a Manifest from the list of snapshots.
    pub async fn load_manifest(&self) -> Result<Manifest> {
        let snapshots = self.engine.list_snapshots().await?;

        let archives = snapshots.into_iter().map(|s| ArchiveRef {
            name: s.name,
            id: s.root_tree,
            time: s.time,
        }).collect();

        Ok(Manifest {
            version: 1,
            timestamp: chrono::Utc::now(),
            archives,
        })
    }

    /// Delete an archive by name
    pub async fn delete_archive(&mut self, name: &str) -> Result<()> {
        // Find the snapshot corresponding to the archive name
        let snapshots = self.engine.list_snapshots().await?;
        if let Some(snapshot) = snapshots.iter().find(|s| s.name == name) {
            // Delete the snapshot file
            // Note: This doesn't delete the data chunks immediately. GC is required.
            // The engine trait doesn't have delete_snapshot, but we can infer the path or add it.
            // For ObjectLogV1, snapshots are stored as "snapshots/{id}.json"
            // We can add delete_snapshot to StorageEngine trait or just use delete_object if we expose it.
            // But delete_object takes a ChunkId, and snapshot ID is a String (UUID).
            // Let's add delete_snapshot to StorageEngine.

            // Since I can't easily change the trait definition and all impls in one go without more context,
            // I'll use a workaround for ObjectLogV1 if possible, or just assume I can add it.
            // Actually, I can add it to the trait right now in this file.
            self.engine.delete_snapshot(&snapshot.id).await
        } else {
            Err(BorgError::ArchiveNotFound { name: name.to_string() })
        }
    }

    /// Retrieve and parse a Tree object
    pub async fn get_tree(&self, id: &ChunkId) -> Result<Tree> {
        let chunk = self.get_chunk(id).await?;
        serde_json::from_slice(&chunk.data)
            .map_err(|e| BorgError::Deserialization(format!("Failed to parse tree: {}", e)))
    }

    /// Store a Tree object
    pub async fn put_tree(&mut self, tree: &Tree) -> Result<ChunkId> {
        let data = serde_json::to_vec(tree)
            .map_err(|e| BorgError::Serialization(e.to_string()))?;
        let chunk = Chunk::new(data);
        let id = chunk.id.clone();
        self.put_chunk(&chunk).await?;
        Ok(id)
    }

    /// Garbage Collection (Mark and Sweep)
    /// 
    /// 1. Lists all snapshots
    /// 2. Traverses all trees to find reachable chunks
    /// 3. Lists all chunks in storage
    /// 4. Deletes chunks that are not reachable
    #[instrument(skip(self))]
    pub async fn gc(&self) -> Result<RepositoryStats> {
        info!("Starting Garbage Collection...");

        // 1. Mark Phase
        let mut reachable = HashSet::new();
        let snapshots = self.engine.list_snapshots().await?;
        
        info!("Scanning {} snapshots...", snapshots.len());
        for snap in &snapshots {
            debug!("Traversing snapshot {}", snap.id);
            self.traverse_tree(&snap.root_tree, &mut reachable).await?;
        }

        // 2. Sweep Phase
        let all_objects = self.engine.list_objects().await?;
        let total_objects = all_objects.len();
        let mut deleted_count = 0;
        let _reclaimed_size = 0; // Note: we don't know size without stat, simplified

        info!("Checking {} objects...", total_objects);
        for id in all_objects {
            if !reachable.contains(&id) {
                // Prune
                self.engine.delete_object(&id).await?;
                deleted_count += 1;
            }
        }

        info!("GC Complete. Deleted {} objects.", deleted_count);
        
        Ok(RepositoryStats {
            total_chunks: (total_objects - deleted_count) as u64,
            total_size: 0, // TODO: Track actual size
            compressed_size: 0,
        })
    }

    /// Helper to traverse a tree and mark reachable chunks
    async fn traverse_tree(&self, root_id: &ChunkId, reachable: &mut HashSet<ChunkId>) -> Result<()> {
        let mut stack = VecDeque::new();
        stack.push_back(root_id.clone());

        while let Some(id) = stack.pop_front() {
            if !reachable.insert(id.clone()) {
                continue; // Already visited
            }

            // If this chunk is a Tree, we need to parse it and add children
            // Note: In a real system we need to know if a chunk is a Tree or File content.
            // Here we attempt to parse as Tree, if fails, assume it's data (leaf).
            // A better approach is to have typed pointers in the parent.
            if let Ok(tree) = self.get_tree(&id).await {
                for entry in tree.entries {
                    match entry.kind {
                        EntryKind::Dir { tree } => stack.push_back(tree),
                        EntryKind::File { chunks, .. } => {
                            for chunk_id in chunks {
                                reachable.insert(chunk_id);
                            }
                        }
                        EntryKind::Symlink { .. } => {}
                    }
                }
            }
        }
        Ok(())
    }

    pub fn descriptor(&self) -> &RepoDescriptor {
        &self.descriptor
    }

    pub fn stats(&self) -> RepositoryStats {
        RepositoryStats {
            total_chunks: self.chunk_cache.len() as u64,
            // Would need to iterate to get actual sizes
            total_size: 0,
            compressed_size: 0,
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
        let _repo = Repository::init(op.clone(), "test-repo".to_string(), Some("test-passphrase"), None).await.expect("Init failed");

        // Open
        let repo = Repository::open(op, "test-repo".to_string(), Some("test-passphrase")).await.expect("Open failed");
        assert!(repo.descriptor().encrypted);
    }

    #[tokio::test]
    async fn test_chunk_storage() {
        let temp_dir = TempDir::new().unwrap();
        let op = get_test_op(&temp_dir).await;

        let mut repo = Repository::init(op, "test-repo".to_string(), Some("passphrase"), None).await.unwrap();

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
