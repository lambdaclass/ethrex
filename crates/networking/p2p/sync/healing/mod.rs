//! Trie Healing Module
//!
//! Heals state and storage tries during snap sync by downloading
//! missing nodes and reconciling inconsistencies from multi-pivot downloads.

pub mod state;
pub mod storage;
mod types;

use crate::sync::SyncError;
use tokio::task::JoinSet;

pub use state::heal_state_trie_wrap;
pub use storage::heal_storage_trie;

// Re-export shared types for external use
#[allow(unused_imports)]
pub use types::{HealingQueueEntry, StateHealingQueue};

/// Waits for a pending task in the JoinSet to complete, propagating any error.
/// Used to ensure only a single background DB write is in flight at a time.
async fn wait_for_pending_task(joinset: &mut JoinSet<()>) -> Result<(), SyncError> {
    if !joinset.is_empty()
        && let Some(Err(e)) = joinset.join_next().await
    {
        return Err(SyncError::JoinHandle(e));
    }
    Ok(())
}
