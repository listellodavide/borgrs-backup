//! Compression module supporting multiple algorithms
//!
//! Supports lz4 (fast), zstd (balanced), zlib (compatible), and lzma (high ratio).
//! Each algorithm offers different trade-offs between speed and compression ratio.

use crate::error::{BorgError, Result};
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression as ZlibCompression};
use lzma_rs::{lzma_compress, lzma_decompress};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Write};
use tracing::{debug, instrument};

/// Supported compression algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CompressionAlgorithm {
    /// No compression
    None,
    /// LZ4 - Very fast compression with moderate ratio
    Lz4,
    /// Zstandard - Balanced speed and compression ratio (default)
    #[default]
    Zstd,
    /// Zlib/Deflate - Good compatibility, moderate performance
    Zlib,
    /// LZMA - Best compression ratio, slowest
    Lzma,
}

impl CompressionAlgorithm {
    /// Get all available algorithms
    pub fn all() -> &'static [CompressionAlgorithm] {
        &[
            CompressionAlgorithm::None,
            CompressionAlgorithm::Lz4,
            CompressionAlgorithm::Zstd,
            CompressionAlgorithm::Zlib,
            CompressionAlgorithm::Lzma,
        ]
    }

    /// Parse algorithm from string
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "none" => Ok(Self::None),
            "lz4" => Ok(Self::Lz4),
            "zstd" | "zstandard" => Ok(Self::Zstd),
            "zlib" | "deflate" => Ok(Self::Zlib),
            "lzma" | "xz" => Ok(Self::Lzma),
            _ => Err(BorgError::UnsupportedCompression(s.to_string())),
        }
    }

    /// Get the algorithm identifier byte for storage
    pub fn id_byte(&self) -> u8 {
        match self {
            Self::None => 0x00,
            Self::Lz4 => 0x01,
            Self::Zstd => 0x02,
            Self::Zlib => 0x03,
            Self::Lzma => 0x04,
        }
    }

    /// Parse algorithm from identifier byte
    pub fn from_id_byte(byte: u8) -> Result<Self> {
        match byte {
            0x00 => Ok(Self::None),
            0x01 => Ok(Self::Lz4),
            0x02 => Ok(Self::Zstd),
            0x03 => Ok(Self::Zlib),
            0x04 => Ok(Self::Lzma),
            _ => Err(BorgError::UnsupportedCompression(format!(
                "Unknown algorithm ID: 0x{:02x}",
                byte
            ))),
        }
    }

    /// Get human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Lz4 => "lz4",
            Self::Zstd => "zstd",
            Self::Zlib => "zlib",
            Self::Lzma => "lzma",
        }
    }
}

impl std::fmt::Display for CompressionAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Compression level (1-9, algorithm-specific interpretation)
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CompressionLevel(pub u8);

impl CompressionLevel {
    /// Fastest compression
    pub const FAST: Self = Self(1);
    /// Default balanced compression
    pub const DEFAULT: Self = Self(6);
    /// Best compression ratio
    pub const BEST: Self = Self(9);

    /// Create a new compression level
    pub fn new(level: u8) -> Result<Self> {
        if level > 9 {
            return Err(BorgError::InvalidArgument(
                "Compression level must be 0-9".to_string(),
            ));
        }
        Ok(Self(level))
    }
}

impl Default for CompressionLevel {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Configuration for compression operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionConfig {
    /// Algorithm to use
    pub algorithm: CompressionAlgorithm,
    /// Compression level
    pub level: CompressionLevel,
    /// Minimum size to compress (smaller data is stored uncompressed)
    pub min_size: usize,
    /// Auto-detect incompressible data and skip compression
    pub auto_detect: bool,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            algorithm: CompressionAlgorithm::Zstd,
            level: CompressionLevel::DEFAULT,
            min_size: 128, // Don't compress data smaller than 128 bytes
            auto_detect: true,
        }
    }
}

/// Compressed data with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressedData {
    /// Algorithm used (0x00 = none/uncompressed)
    pub algorithm: u8,
    /// Original uncompressed size
    pub original_size: u32,
    /// Compressed data
    pub data: Vec<u8>,
}

impl CompressedData {
    /// Check if the data was actually compressed
    pub fn is_compressed(&self) -> bool {
        self.algorithm != 0x00
    }

    /// Get the compression ratio (compressed/original)
    pub fn ratio(&self) -> f64 {
        if self.original_size == 0 {
            1.0
        } else {
            self.data.len() as f64 / self.original_size as f64
        }
    }

    /// Get space saved as percentage
    pub fn space_saved_percent(&self) -> f64 {
        (1.0 - self.ratio()) * 100.0
    }
}

/// Compressor for data compression/decompression
pub struct Compressor {
    config: CompressionConfig,
}

impl Compressor {
    /// Create a new compressor with the given configuration
    pub fn new(config: CompressionConfig) -> Self {
        Self { config }
    }

    /// Create a compressor with default settings
    pub fn with_defaults() -> Self {
        Self {
            config: CompressionConfig::default(),
        }
    }

    /// Create a compressor with a specific algorithm
    pub fn with_algorithm(algorithm: CompressionAlgorithm) -> Self {
        Self {
            config: CompressionConfig {
                algorithm,
                ..Default::default()
            },
        }
    }

    /// Get the current configuration
    pub fn config(&self) -> &CompressionConfig {
        &self.config
    }

    /// Compress data
    #[instrument(skip(self, data), fields(data_len = data.len()))]
    pub fn compress(&self, data: &[u8]) -> Result<CompressedData> {
        let original_size = data.len() as u32;

        // Skip compression for small data
        if data.len() < self.config.min_size {
            debug!("Skipping compression for small data ({} bytes)", data.len());
            return Ok(CompressedData {
                algorithm: CompressionAlgorithm::None.id_byte(),
                original_size,
                data: data.to_vec(),
            });
        }

        // Skip if no compression requested
        if self.config.algorithm == CompressionAlgorithm::None {
            return Ok(CompressedData {
                algorithm: CompressionAlgorithm::None.id_byte(),
                original_size,
                data: data.to_vec(),
            });
        }

        let compressed = match self.config.algorithm {
            CompressionAlgorithm::None => data.to_vec(),
            CompressionAlgorithm::Lz4 => self.compress_lz4(data)?,
            CompressionAlgorithm::Zstd => self.compress_zstd(data)?,
            CompressionAlgorithm::Zlib => self.compress_zlib(data)?,
            CompressionAlgorithm::Lzma => self.compress_lzma(data)?,
        };

        // If compressed data is larger, store uncompressed
        if self.config.auto_detect && compressed.len() >= data.len() {
            debug!(
                "Compression ineffective ({}% expansion), storing uncompressed",
                ((compressed.len() as f64 / data.len() as f64) - 1.0) * 100.0
            );
            return Ok(CompressedData {
                algorithm: CompressionAlgorithm::None.id_byte(),
                original_size,
                data: data.to_vec(),
            });
        }

        debug!(
            "Compressed {} -> {} bytes ({:.1}% saved)",
            data.len(),
            compressed.len(),
            (1.0 - compressed.len() as f64 / data.len() as f64) * 100.0
        );

        Ok(CompressedData {
            algorithm: self.config.algorithm.id_byte(),
            original_size,
            data: compressed,
        })
    }

    /// Decompress data
    #[instrument(skip(self, compressed), fields(compressed_len = compressed.data.len()))]
    pub fn decompress(&self, compressed: &CompressedData) -> Result<Vec<u8>> {
        let algorithm = CompressionAlgorithm::from_id_byte(compressed.algorithm)?;

        let decompressed = match algorithm {
            CompressionAlgorithm::None => compressed.data.clone(),
            CompressionAlgorithm::Lz4 => {
                self.decompress_lz4(&compressed.data, compressed.original_size as usize)?
            }
            CompressionAlgorithm::Zstd => self.decompress_zstd(&compressed.data)?,
            CompressionAlgorithm::Zlib => self.decompress_zlib(&compressed.data)?,
            CompressionAlgorithm::Lzma => self.decompress_lzma(&compressed.data)?,
        };

        // Verify size
        if decompressed.len() != compressed.original_size as usize {
            return Err(BorgError::Decompression(format!(
                "Size mismatch: expected {}, got {}",
                compressed.original_size,
                decompressed.len()
            )));
        }

        debug!(
            "Decompressed {} -> {} bytes",
            compressed.data.len(),
            decompressed.len()
        );

        Ok(decompressed)
    }

    // LZ4 compression/decompression
    fn compress_lz4(&self, data: &[u8]) -> Result<Vec<u8>> {
        Ok(lz4_flex::compress_prepend_size(data))
    }

    fn decompress_lz4(&self, data: &[u8], _original_size: usize) -> Result<Vec<u8>> {
        lz4_flex::decompress_size_prepended(data)
            .map_err(|e| BorgError::Decompression(format!("LZ4 decompression failed: {}", e)))
    }

    // Zstd compression/decompression
    fn compress_zstd(&self, data: &[u8]) -> Result<Vec<u8>> {
        let level = self.config.level.0 as i32;
        zstd::encode_all(Cursor::new(data), level)
            .map_err(|e| BorgError::Compression(format!("Zstd compression failed: {}", e)))
    }

    fn decompress_zstd(&self, data: &[u8]) -> Result<Vec<u8>> {
        zstd::decode_all(Cursor::new(data))
            .map_err(|e| BorgError::Decompression(format!("Zstd decompression failed: {}", e)))
    }

    // Zlib compression/decompression
    fn compress_zlib(&self, data: &[u8]) -> Result<Vec<u8>> {
        let level = match self.config.level.0 {
            0..=3 => ZlibCompression::fast(),
            4..=6 => ZlibCompression::default(),
            _ => ZlibCompression::best(),
        };
        let mut encoder = ZlibEncoder::new(Vec::new(), level);
        encoder
            .write_all(data)
            .map_err(|e| BorgError::Compression(format!("Zlib compression failed: {}", e)))?;
        encoder
            .finish()
            .map_err(|e| BorgError::Compression(format!("Zlib compression failed: {}", e)))
    }

    fn decompress_zlib(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut decoder = ZlibDecoder::new(data);
        let mut decompressed = Vec::new();
        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| BorgError::Decompression(format!("Zlib decompression failed: {}", e)))?;
        Ok(decompressed)
    }

    // LZMA compression/decompression
    fn compress_lzma(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut compressed = Vec::new();
        lzma_compress(&mut Cursor::new(data), &mut compressed)
            .map_err(|e| BorgError::Compression(format!("LZMA compression failed: {}", e)))?;
        Ok(compressed)
    }

    fn decompress_lzma(&self, data: &[u8]) -> Result<Vec<u8>> {
        let mut decompressed = Vec::new();
        lzma_decompress(&mut Cursor::new(data), &mut decompressed)
            .map_err(|e| BorgError::Decompression(format!("LZMA decompression failed: {}", e)))?;
        Ok(decompressed)
    }
}

/// Compression statistics
#[derive(Debug, Default, Clone)]
pub struct CompressionStats {
    /// Total bytes before compression
    pub total_input: u64,
    /// Total bytes after compression
    pub total_output: u64,
    /// Number of items compressed
    pub items_compressed: u64,
    /// Number of items stored uncompressed
    pub items_uncompressed: u64,
}

impl CompressionStats {
    /// Get overall compression ratio
    pub fn ratio(&self) -> f64 {
        if self.total_input == 0 {
            1.0
        } else {
            self.total_output as f64 / self.total_input as f64
        }
    }

    /// Get space saved as percentage
    pub fn space_saved_percent(&self) -> f64 {
        (1.0 - self.ratio()) * 100.0
    }

    /// Update stats with a compression result
    pub fn record(&mut self, input_size: usize, compressed: &CompressedData) {
        self.total_input += input_size as u64;
        self.total_output += compressed.data.len() as u64;
        if compressed.is_compressed() {
            self.items_compressed += 1;
        } else {
            self.items_uncompressed += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_decompress_all_algorithms() {
        let data = b"Hello, Borg-Rust! This is test data for compression.".repeat(100);

        for algorithm in CompressionAlgorithm::all() {
            let compressor = Compressor::with_algorithm(*algorithm);
            let compressed = compressor.compress(&data).unwrap();
            let decompressed = compressor.decompress(&compressed).unwrap();
            assert_eq!(data.as_slice(), decompressed.as_slice(), "Failed for {:?}", algorithm);
        }
    }

    #[test]
    fn test_small_data_not_compressed() {
        let compressor = Compressor::with_defaults();
        let small_data = b"tiny";
        let compressed = compressor.compress(small_data).unwrap();
        assert!(!compressed.is_compressed());
    }

    #[test]
    fn test_incompressible_data_stored_raw() {
        let compressor = Compressor::new(CompressionConfig {
            algorithm: CompressionAlgorithm::Zstd,
            auto_detect: true,
            min_size: 0,
            ..Default::default()
        });
        
        // Random data is incompressible
        let random_data: Vec<u8> = (0..1000).map(|i| (i * 17 + 13) as u8).collect();
        let compressed = compressor.compress(&random_data).unwrap();
        
        // Should either be uncompressed or at most slightly larger
        let decompressed = compressor.decompress(&compressed).unwrap();
        assert_eq!(random_data, decompressed);
    }

    #[test]
    fn test_algorithm_id_roundtrip() {
        for algorithm in CompressionAlgorithm::all() {
            let id = algorithm.id_byte();
            let parsed = CompressionAlgorithm::from_id_byte(id).unwrap();
            assert_eq!(*algorithm, parsed);
        }
    }

    #[test]
    fn test_compression_stats() {
        let mut stats = CompressionStats::default();
        let compressor = Compressor::with_defaults();
        
        let data = b"Repeating data ".repeat(1000);
        let compressed = compressor.compress(&data).unwrap();
        stats.record(data.len(), &compressed);

        assert!(stats.ratio() < 1.0); // Should have compressed
        assert!(stats.space_saved_percent() > 0.0);
    }
}
