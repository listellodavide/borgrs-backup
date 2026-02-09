//! Content-Defined Chunking (CDC) using FastCDC algorithm
//!
//! Implements variable-length chunking for efficient deduplication.
//! Uses the FastCDC algorithm which provides better performance than BuzHash
//! while maintaining good deduplication ratios.

use crate::error::Result;
use fastcdc::v2020::FastCDC;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::str::FromStr;
use tracing::{debug, instrument};

/// Unique identifier for a chunk (SHA-256 hash)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkId(pub [u8; 32]);

impl ChunkId {
    /// Create a new ChunkId from raw bytes
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Compute ChunkId from data
    pub fn from_data(data: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&result);
        Self(id)
    }

    /// Convert to hex string for display
    pub fn to_hex(&self) -> String {
        hex::encode(&self.0)
    }

    /// Parse from hex string
    pub fn from_hex(s: &str) -> Result<Self> {
        let bytes = hex::decode(s)
            .map_err(|e| crate::error::BorgError::InvalidArgument(format!("Invalid chunk ID hex: {}", e)))?;
        if bytes.len() != 32 {
            return Err(crate::error::BorgError::InvalidArgument(
                "Chunk ID must be 32 bytes".to_string(),
            ));
        }
        let mut id = [0u8; 32];
        id.copy_from_slice(&bytes);
        Ok(Self(id))
    }
}

impl std::fmt::Display for ChunkId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.to_hex()[..16]) // Show first 16 chars for brevity
    }
}

/// A chunk of data with its identifier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Unique identifier (content hash)
    pub id: ChunkId,
    /// Raw chunk data (before compression/encryption)
    pub data: Vec<u8>,
    /// Original size before any processing
    pub original_size: usize,
}

impl Chunk {
    /// Create a new chunk from data
    pub fn new(data: Vec<u8>) -> Self {
        let original_size = data.len();
        let id = ChunkId::from_data(&data);
        Self {
            id,
            data,
            original_size,
        }
    }
}

/// Chunker profiles for different average chunk sizes
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChunkerProfile {
    /// Dynamic profile selection based on file size
    Default,
    /// 1 MiB average chunk size
    Size1M,
    /// 4 MiB average chunk size
    Size4M,
    /// 8 MiB average chunk size
    Size8M,
    /// 16 MiB average chunk size
    Size16M,
    /// 32 MiB average chunk size
    Size32M,
    /// 64 MiB average chunk size (Limit for FastCDC v2020)
    Size64M,
}

impl FromStr for ChunkerProfile {
    type Err = crate::error::BorgError;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "default" => Ok(Self::Default),
            "1m" | "1mb" => Ok(Self::Size1M),
            "4m" | "4mb" => Ok(Self::Size4M),
            "8m" | "8mb" => Ok(Self::Size8M),
            "16m" | "16mb" => Ok(Self::Size16M),
            "32m" | "32mb" => Ok(Self::Size32M),
            "64m" | "64mb" => Ok(Self::Size64M),
            _ => Err(crate::error::BorgError::InvalidArgument(format!("Unknown chunker profile: {}", s))),
        }
    }
}

impl std::fmt::Display for ChunkerProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Default => "default",
            Self::Size1M => "1mb",
            Self::Size4M => "4mb",
            Self::Size8M => "8mb",
            Self::Size16M => "16mb",
            Self::Size32M => "32mb",
            Self::Size64M => "64mb",
        };
        write!(f, "{}", s)
    }
}

/// Dynamically choose a chunker profile based on file size in megabytes.
pub fn choose_profile_for_size(size_mb: u64) -> ChunkerProfile {
    match size_mb {
        s if s < 20 => ChunkerProfile::Size1M,
        s if s < 40 => ChunkerProfile::Size4M,
        s if s < 80 => ChunkerProfile::Size8M,
        s if s < 160 => ChunkerProfile::Size16M,
        s if s < 320 => ChunkerProfile::Size32M,
        _ => ChunkerProfile::Size64M,
    }
}

/// Configuration for the chunker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkerConfig {
    /// Minimum chunk size in bytes
    pub min_size: u32,
    /// Average chunk size in bytes
    pub avg_size: u32,
    /// Maximum chunk size in bytes
    pub max_size: u32,
}

impl Default for ChunkerConfig {
    /// The default configuration is the 4 MiB profile.
    fn default() -> Self {
        Self::from_profile(ChunkerProfile::Size4M)
    }
}

impl ChunkerConfig {
    /// Create a configuration from a chunker profile.
    pub fn from_profile(profile: ChunkerProfile) -> Self {
        match profile {
            ChunkerProfile::Default => Self::from_profile(ChunkerProfile::Size4M), // Default to 4M if not specified
            ChunkerProfile::Size1M => Self {
                min_size: 256 * 1024,      // 256 KiB
                avg_size: 1024 * 1024,     // 1 MiB
                max_size: 4 * 1024 * 1024, // 4 MiB
            },
            ChunkerProfile::Size4M => Self {
                min_size: 1024 * 1024,       // 1 MiB
                avg_size: 4 * 1024 * 1024,     // 4 MiB
                max_size: 16 * 1024 * 1024,    // 16 MiB
            },
            ChunkerProfile::Size8M => Self {
                min_size: 2 * 1024 * 1024,     // 2 MiB
                avg_size: 8 * 1024 * 1024,     // 8 MiB
                max_size: 32 * 1024 * 1024,    // 32 MiB
            },
            ChunkerProfile::Size16M => Self {
                min_size: 4 * 1024 * 1024,     // 4 MiB
                avg_size: 16 * 1024 * 1024,    // 16 MiB
                max_size: 64 * 1024 * 1024,    // 64 MiB
            },
            ChunkerProfile::Size32M => Self {
                min_size: 8 * 1024 * 1024,     // 8 MiB
                avg_size: 32 * 1024 * 1024,    // 32 MiB
                max_size: 64 * 1024 * 1024,   // 64 MiB
            },
            ChunkerProfile::Size64M => Self {
                min_size: 16 * 1024 * 1024,    // 16 MiB
                avg_size: 64 * 1024 * 1024,    // 64 MiB
                max_size: 64 * 1024 * 1024,    // 64 MiB (Limit)
            },
        }
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        const MINIMUM_MAX: u32 = 64 * 1024 * 1024; // FastCDC v2020 limit

        if self.min_size == 0 {
            return Err(crate::error::BorgError::InvalidArgument(
                "min_size must be greater than 0".to_string(),
            ));
        }
        if self.min_size > MINIMUM_MAX {
             return Err(crate::error::BorgError::InvalidArgument(
                format!("min_size ({}) must be <= {} bytes (FastCDC v2020 limit)", self.min_size, MINIMUM_MAX),
            ));
        }
        if self.avg_size > MINIMUM_MAX * 2 { // Some reasonable upper bound for avg
             return Err(crate::error::BorgError::InvalidArgument(
                format!("avg_size ({}) is too large", self.avg_size),
            ));
        }
        if self.max_size > MINIMUM_MAX {
             return Err(crate::error::BorgError::InvalidArgument(
                format!("max_size ({}) must be <= {} bytes (FastCDC v2020 limit)", self.max_size, MINIMUM_MAX),
            ));
        }
        if self.avg_size < self.min_size {
            return Err(crate::error::BorgError::InvalidArgument(
                "avg_size must be >= min_size".to_string(),
            ));
        }
        if self.max_size < self.avg_size && self.max_size < MINIMUM_MAX {
            // max_size can be equal to avg_size if we are at the limit
            return Err(crate::error::BorgError::InvalidArgument(
                "max_size must be >= avg_size".to_string(),
            ));
        }
        Ok(())
    }
}

/// Content-Defined Chunker for splitting data into variable-length chunks
pub struct Chunker {
    config: ChunkerConfig,
}

impl Chunker {
    /// Create a new chunker with the given configuration
    pub fn new(config: ChunkerConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self { config })
    }

    /// Create a chunker with default settings
    pub fn with_defaults() -> Self {
        Self::from_profile(ChunkerProfile::Default)
    }

    /// Create a chunker for a specific profile
    pub fn from_profile(profile: ChunkerProfile) -> Self {
        let config = ChunkerConfig::from_profile(profile);
        // We trust our profile definitions, but validate anyway to be sure
        if let Err(e) = config.validate() {
            panic!("Invalid chunker profile {}: {}", profile, e);
        }
        Self { config }
    }

    /// Get the current configuration
    pub fn config(&self) -> &ChunkerConfig {
        &self.config
    }

    /// Chunk a byte slice into variable-length chunks
    #[instrument(skip(self, data), fields(data_len = data.len()))]
    pub fn chunk_data(&self, data: &[u8]) -> Vec<Chunk> {
        if data.is_empty() {
            return Vec::new();
        }

        // FastCDC v2020 has specific limits. 
        // We use a robust builder pattern or careful initialization.
        let chunker = FastCDC::new(
            data,
            self.config.min_size,
            self.config.avg_size,
            self.config.max_size,
        );

        let mut chunks = Vec::new();
        
        // Wrap iteration in a way that we could catch issues if it was possible, 
        // but here we just iterate. FastCDC::new already performed assertions in some versions.
        for chunk_info in chunker {
            let chunk_data = data[chunk_info.offset..chunk_info.offset + chunk_info.length].to_vec();
            chunks.push(Chunk::new(chunk_data));
        }

        debug!(
            "Chunked {} bytes into {} chunks",
            data.len(),
            chunks.len()
        );

        chunks
    }

    /// Chunk data from a reader
    #[instrument(skip(self, reader))]
    pub fn chunk_reader<R: Read>(&self, mut reader: R) -> Result<Vec<Chunk>> {
        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer)?;
        
        // FastCDC v2020 might panic in new() if params are invalid despite our validation.
        // In the future, we could use catch_unwind if we want to be truly "nasa level" 
        // but for now, we rely on the validated config.
        Ok(self.chunk_data(&buffer))
    }

    /// Stream chunks from a reader (memory-efficient for large files)
    #[instrument(skip(self, reader, callback))]
    pub fn stream_chunks<R, F>(&self, mut reader: R, mut callback: F) -> Result<ChunkStats>
    where
        R: Read,
        F: FnMut(Chunk) -> Result<()>,
    {
        let mut stats = ChunkStats::default();
        let mut buffer = vec![0u8; self.config.max_size as usize * 2];
        let mut pending = Vec::new();

        loop {
            let bytes_read = reader.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }

            pending.extend_from_slice(&buffer[..bytes_read]);
            stats.bytes_processed += bytes_read as u64;

            // Only chunk when we have enough data
            if pending.len() >= self.config.max_size as usize {
                let chunks = self.chunk_data(&pending);

                // Keep the last chunk as it might be incomplete
                if chunks.len() > 1 {
                    let mut processed_in_this_batch = 0;
                    let num_chunks = chunks.len();

                    for chunk in chunks.into_iter().take(num_chunks - 1) {
                        stats.chunk_count += 1;
                        stats.total_chunk_bytes += chunk.original_size as u64;
                        processed_in_this_batch += chunk.original_size;
                        callback(chunk)?;
                    }

                    pending = pending[processed_in_this_batch..].to_vec();
                }
            }
        }

        // Process remaining data
        if !pending.is_empty() {
            for chunk in self.chunk_data(&pending) {
                stats.chunk_count += 1;
                stats.total_chunk_bytes += chunk.original_size as u64;
                callback(chunk)?;
            }
        }

        Ok(stats)
    }
}

/// Statistics about chunking operations
#[derive(Debug, Default, Clone)]
pub struct ChunkStats {
    /// Total bytes processed
    pub bytes_processed: u64,
    /// Number of chunks created
    pub chunk_count: u64,
    /// Total size of all chunks
    pub total_chunk_bytes: u64,
}

impl ChunkStats {
    /// Calculate average chunk size
    pub fn avg_chunk_size(&self) -> f64 {
        if self.chunk_count == 0 {
            0.0
        } else {
            self.total_chunk_bytes as f64 / self.chunk_count as f64
        }
    }
}

// Add hex crate for encoding
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    pub fn decode(s: &str) -> std::result::Result<Vec<u8>, String> {
        if s.len() % 2 != 0 {
            return Err("Invalid hex string length".to_string());
        }
        (0..s.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&s[i..i + 2], 16)
                    .map_err(|e| format!("Invalid hex character: {}", e))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_empty_data() {
        let chunker = Chunker::from_profile(ChunkerProfile::Default);
        let chunks = chunker.chunk_data(&[]);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_small_data() {
        let chunker = Chunker::from_profile(ChunkerProfile::Default);
        let data = vec![0u8; 1024]; // 1 KiB
        let chunks = chunker.chunk_data(&data);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].original_size, 1024);
    }

    #[test]
    fn test_chunk_id_deterministic() {
        let data = b"Hello, World!";
        let id1 = ChunkId::from_data(data);
        let id2 = ChunkId::from_data(data);
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_chunk_id_hex_roundtrip() {
        let data = b"test data";
        let id = ChunkId::from_data(data);
        let hex = id.to_hex();
        let parsed = ChunkId::from_hex(&hex).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_config_validation() {
        let config = ChunkerConfig::from_profile(ChunkerProfile::Default);
        assert!(config.validate().is_ok());

        let mut invalid_config = ChunkerConfig::from_profile(ChunkerProfile::Default);
        invalid_config.min_size = 0;
        assert!(invalid_config.validate().is_err());

        let mut invalid_config_2 = ChunkerConfig::from_profile(ChunkerProfile::Default);
        invalid_config_2.avg_size = invalid_config_2.min_size - 1;
        assert!(invalid_config_2.validate().is_err());
    }

    #[test]
    fn test_choose_profile_for_size() {
        assert_eq!(choose_profile_for_size(10), ChunkerProfile::Size1M);
        assert_eq!(choose_profile_for_size(30), ChunkerProfile::Size4M);
        assert_eq!(choose_profile_for_size(70), ChunkerProfile::Size8M);
        assert_eq!(choose_profile_for_size(150), ChunkerProfile::Size16M);
        assert_eq!(choose_profile_for_size(300), ChunkerProfile::Size32M);
        assert_eq!(choose_profile_for_size(1000), ChunkerProfile::Size64M);
    }
}
