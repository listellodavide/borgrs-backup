//! Metadata structures for repository objects
//!
//! This module defines the core data structures used for snapshots, trees,
//! and repository statistics.

use crate::chunker::ChunkId;
use serde::{Deserialize, Serialize};

/// A snapshot represents a point-in-time backup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Unique ID of the snapshot
    pub id: String,
    /// Name of the archive
    pub name: String,
    /// Timestamp of creation
    pub time: chrono::DateTime<chrono::Utc>,
    /// Root tree ID
    pub root_tree: ChunkId,
    /// Hostname where backup was created
    pub hostname: String,
    /// Username who created the backup
    pub username: String,
    /// Command line arguments
    pub cmdline: Vec<String>,
    /// Optional comment
    pub comment: Option<String>,
    /// Optional tags
    pub tags: Option<Vec<String>>,
}

/// A tree represents a directory structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tree {
    /// List of entries in this directory
    pub entries: Vec<TreeEntry>,
}

/// An entry in a tree (file, directory, symlink, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeEntry {
    /// Name of the entry
    pub name: String,
    /// Type of entry and type-specific data
    pub kind: EntryKind,
    /// File attributes (permissions, ownership, etc.)
    pub attributes: Attributes,
}

/// Type of a tree entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EntryKind {
    /// A regular file
    File {
        /// Size of the file
        size: u64,
        /// List of chunks that make up the file
        chunks: Vec<ChunkId>,
    },
    /// A directory
    Dir {
        /// ID of the tree object for this directory
        tree: ChunkId,
    },
    /// A symbolic link
    Symlink {
        /// Target path
        target: String,
    },
}

/// File attributes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attributes {
    /// Unix mode (permissions)
    pub mode: u32,
    /// User ID
    pub uid: u32,
    /// Group ID
    pub gid: u32,
    /// User name (optional)
    pub user: Option<String>,
    /// Group name (optional)
    pub group: Option<String>,
    /// Modification time
    pub mtime: i64,
    /// Access time
    pub atime: i64,
    /// Change time
    pub ctime: i64,
}

/// Repository statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepositoryStats {
    /// Total number of chunks
    pub total_chunks: u64,
    /// Total size of all chunks (uncompressed)
    pub total_size: u64,
    /// Total size of all chunks (compressed/stored)
    pub compressed_size: u64,
}
