//! Errors the `pbtsnap/1` server can produce.

use ethereum_types::H256;
use ethrex_storage::error::StoreError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PbtSnapError {
    #[error(transparent)]
    Store(#[from] StoreError),

    /// The requested root is not one this node can answer for: unknown, on an
    /// abandoned branch, before the binary-tree activation, or below the layer
    /// window.
    ///
    /// **A client MUST read this as "choose a new pivot", never as "this state
    /// does not exist."** Serving depth is bounded by the layer window
    /// (roughly `DB_COMMIT_THRESHOLD` blocks while running, and only what
    /// reached disk after a restart), so a stale pivot is the expected case on
    /// any sync slower than that window rather than a sign of a bad peer.
    #[error("cannot serve pbtsnap leaf ranges at root {0:#x}")]
    UnservableRoot(H256),

    #[error("pbtsnap serving task panicked: {0}")]
    TaskPanic(String),
}
