//! Error types for Borg-Core

use thiserror::Error;

/// Central error type for all Borg-Rust operations
#[derive(Error, Debug)]
pub enum BorgError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Repository error: {0}")]
    Repository(String),

    #[error("Repository not found: {path}")]
    RepositoryNotFound { path: String },

    #[error("Repository already exists: {path}")]
    RepositoryExists { path: String },

    #[error("Repository locked by another process")]
    RepositoryLocked,

    #[error("Lock error: {0}")]
    LockError(String),

    #[error("Archive not found: {name}")]
    ArchiveNotFound { name: String },

    #[error("Archive already exists: {name}")]
    ArchiveExists { name: String },

    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("Decryption error: {0}")]
    Decryption(String),

    #[error("Invalid passphrase")]
    InvalidPassphrase,

    #[error("Key derivation error: {0}")]
    KeyDerivation(String),

    #[error("Compression error: {0}")]
    Compression(String),

    #[error("Decompression error: {0}")]
    Decompression(String),

    #[error("Unsupported compression algorithm: {0}")]
    UnsupportedCompression(String),

    #[error("Chunking error: {0}")]
    Chunking(String),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),

    #[error("Remote connection error: {0}")]
    RemoteConnection(String),

    #[error("SSH authentication failed: {0}")]
    SshAuth(String),

    #[error("Remote repository error: {0}")]
    RemoteRepository(String),

    #[error("Integrity check failed: expected {expected}, got {actual}")]
    IntegrityCheck { expected: String, actual: String },

    #[error("Exclusion pattern error: {0}")]
    ExclusionPattern(String),

    #[error("Path traversal error: {0}")]
    PathTraversal(String),

    #[error("Permission denied: {path}")]
    PermissionDenied { path: String },

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("Operation cancelled")]
    Cancelled,

    #[error("Timeout after {seconds} seconds")]
    Timeout { seconds: u64 },

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type alias using BorgError
pub type Result<T> = std::result::Result<T, BorgError>;

impl From<bincode::Error> for BorgError {
    fn from(e: bincode::Error) -> Self {
        BorgError::Serialization(e.to_string())
    }
}

impl From<sled::Error> for BorgError {
    fn from(e: sled::Error) -> Self {
        BorgError::Cache(e.to_string())
    }
}

impl From<globset::Error> for BorgError {
    fn from(e: globset::Error) -> Self {
        BorgError::ExclusionPattern(e.to_string())
    }
}

impl From<aes_gcm::Error> for BorgError {
    fn from(_: aes_gcm::Error) -> Self {
        BorgError::Encryption("AES-GCM operation failed".to_string())
    }
}
