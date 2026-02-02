//! Unified error types for the snap sync protocol
//!
//! This module consolidates all snap-related errors into a unified `SnapError` type
//! for consistent error handling across server and client operations.

use crate::peer_table::PeerTableError;
use crate::rlpx::error::PeerConnectionError;
use ethrex_rlp::error::RLPDecodeError;
use ethrex_storage::error::StoreError;
use ethrex_trie::TrieError;
use std::io::ErrorKind;
use std::path::PathBuf;
use thiserror::Error;

/// Unified error type for snap sync protocol operations
#[derive(Debug, Error)]
pub enum SnapError {
    /// Storage layer errors
    #[error(transparent)]
    Store(#[from] StoreError),

    /// Protocol/connection errors
    #[error(transparent)]
    Protocol(#[from] PeerConnectionError),

    /// Trie operation errors
    #[error(transparent)]
    Trie(#[from] TrieError),

    /// RLP decoding errors
    #[error(transparent)]
    RlpDecode(#[from] RLPDecodeError),

    /// Peer table errors
    #[error(transparent)]
    PeerTable(#[from] PeerTableError),

    /// Bad request from peer (invalid or malformed request)
    #[error("Bad request: {0}")]
    BadRequest(String),

    /// Response validation failed (invalid proof, hash mismatch, etc.)
    #[error("Response validation failed: {0}")]
    ValidationError(String),

    /// Peer selection failed (no suitable peers available)
    #[error("Peer selection failed: {0}")]
    PeerSelection(String),

    /// Task queue is empty when it shouldn't be
    #[error("No tasks in queue")]
    NoTasks,

    /// Missing account data
    #[error("No account hashes available")]
    NoAccountHashes,

    /// Missing storage data
    #[error("No account storages available")]
    NoAccountStorages,

    /// Missing storage roots
    #[error("No storage roots available")]
    NoStorageRoots,

    /// Unexpected internal error (indicates a bug)
    #[error("Unexpected internal error: {0}")]
    InternalError(String),

    /// File system operation failed
    #[error("File system error: {operation} at {}: {kind:?}", path.display())]
    FileSystem {
        operation: &'static str,
        path: PathBuf,
        kind: ErrorKind,
    },

    /// Snapshot directory operations
    #[error("Snapshot directory error: {0}")]
    SnapshotDir(String),

    /// Task was spawned but panicked
    #[error("Task panicked: {0}")]
    TaskPanic(String),

    /// Invalid data received from peer
    #[error("Invalid data received")]
    InvalidData,

    /// Hash mismatch in received data
    #[error("Hash mismatch in received data")]
    InvalidHash,
}

impl SnapError {
    /// Creates a file system error for directory not existing
    pub fn dir_not_exists(path: PathBuf) -> Self {
        Self::FileSystem {
            operation: "check exists",
            path,
            kind: ErrorKind::NotFound,
        }
    }

    /// Creates a file system error for directory creation failure
    pub fn dir_create_failed(path: PathBuf) -> Self {
        Self::FileSystem {
            operation: "create directory",
            path,
            kind: ErrorKind::Other,
        }
    }

    /// Creates a file system error for write failure
    pub fn write_failed(path: PathBuf, kind: ErrorKind) -> Self {
        Self::FileSystem {
            operation: "write",
            path,
            kind,
        }
    }
}

/// Converts a tokio task JoinError into SnapError
impl From<tokio::task::JoinError> for SnapError {
    fn from(err: tokio::task::JoinError) -> Self {
        SnapError::TaskPanic(err.to_string())
    }
}

/// Error that occurs when dumping snapshots to disk
#[derive(thiserror::Error)]
#[error("Failed to dump snapshot to {}: {:?}", path.display(), error)]
pub struct DumpError {
    pub path: PathBuf,
    pub contents: Vec<u8>,
    pub error: ErrorKind,
}

impl core::fmt::Debug for DumpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DumpError")
            .field("path", &self.path)
            .field("contents_len", &self.contents.len())
            .field("error", &self.error)
            .finish()
    }
}

impl From<DumpError> for SnapError {
    fn from(err: DumpError) -> Self {
        SnapError::FileSystem {
            operation: "dump snapshot",
            path: err.path,
            kind: err.error,
        }
    }
}
