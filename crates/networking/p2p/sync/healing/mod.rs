//! Trie Healing Module
//!
//! Heals state and storage tries during snap sync by downloading
//! missing nodes and reconciling inconsistencies from multi-pivot downloads.

pub mod state;
pub mod storage;
mod types;

pub use state::heal_state_trie_wrap;
pub use storage::heal_storage_trie;

// Re-export shared types for external use
#[allow(unused_imports)]
pub use types::{HealingQueueEntry, StateHealingQueue};
