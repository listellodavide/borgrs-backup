//! # Borg-Core Library
//!
//! Core functionality for the Borg-Rust backup system.
//! This library provides deduplication, encryption, compression,
//! and repository management capabilities.

pub mod chunker;
pub mod compression;
pub mod crypto;
pub mod repository;
pub mod cache;
pub mod archive;
pub mod exclusion;
pub mod remote;
pub mod error;

pub use error::{BorgError, Result};

/// Re-export commonly used types
pub mod prelude {
    pub use crate::chunker::{Chunker, ChunkerConfig, Chunk};
    pub use crate::compression::{Compressor, CompressionAlgorithm};
    pub use crate::crypto::{CryptoProvider, EncryptionKey};
    pub use crate::repository::{Repository, RepositoryConfig};
    pub use crate::archive::{Archive, ArchiveItem};
    pub use crate::exclusion::ExclusionList;
    pub use crate::error::{BorgError, Result};
}
