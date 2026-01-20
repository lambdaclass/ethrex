//! Block management for unfinalized state.
//!
//! This module handles the "hot" blocks (latest, safe) that are not yet
//! finalized, supporting parallel block creation and Fork Choice Updates.

mod block;
mod blockchain;
mod world_state;

pub use block::{Block, BlockId};
pub use blockchain::{Blockchain, BlockchainError};
pub use world_state::{WorldState, ReadOnlyWorldState, Account};
