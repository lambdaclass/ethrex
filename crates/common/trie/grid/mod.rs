//! Grid-based trie implementation for efficient state root computation.
//!
//! This module implements Erigon's grid-based trie pipelining algorithm.
//! The key innovation is replacing recursive tree traversal with an iterative
//! grid state machine that processes keys in sorted order.
//!
//! # Architecture
//!
//! The grid is a 128x16 matrix where:
//! - Rows correspond to depth levels (0-127 for 64-byte account + storage keys)
//! - Columns correspond to nibble values (0-15)
//!
//! Each cell in the grid can store:
//! - Extension path (for extension/leaf nodes)
//! - Node hash (computed during fold)
//! - Value (for leaf nodes)
//!
//! # Algorithm
//!
//! Keys must be processed in **sorted order**. For each key:
//!
//! 1. **Fold**: If current key diverges from previous key, fold back to
//!    common prefix depth, computing hashes along the way.
//!
//! 2. **Unfold**: Expand grid to target depth by loading from DB or
//!    deriving from parent cell.
//!
//! 3. **Update**: Apply the value change at the leaf position.
//!
//! After all keys are processed, fold back to root to get final hash.
//!
//! # Performance
//!
//! Compared to recursive trie:
//! - O(1) stack depth instead of O(d) recursive calls
//! - Better cache locality (grid vs scattered tree nodes)
//! - Memoized hashing skips unchanged subtrees
//! - 16-way parallelism via rayon (ConcurrentPatriciaGrid)
//!
//! Expected speedup: 5-10x for large state changes (1K+ updates).
//!
//! # Usage
//!
//! ```ignore
//! use ethrex_trie::grid::HexPatriciaGrid;
//!
//! let mut grid = HexPatriciaGrid::new(db);
//!
//! // IMPORTANT: Updates must be sorted by key!
//! let mut updates: Vec<(H256, Vec<u8>)> = /* ... */;
//! updates.sort_by_key(|(k, _)| *k);
//!
//! let root_hash = grid.apply_sorted_updates(updates.into_iter())?;
//! ```

mod bitmap;
mod cell;
mod hex_patricia_grid;

pub use bitmap::{AfterMap, TouchMap};
pub use cell::{Cell, ACCOUNT_KEY_NIBBLES, MAX_DEPTH, NIBBLE_COUNT};
pub use hex_patricia_grid::HexPatriciaGrid;

#[cfg(feature = "grid-trie")]
mod concurrent;

#[cfg(feature = "grid-trie")]
pub use concurrent::ConcurrentPatriciaGrid;

#[cfg(test)]
mod equivalence_tests;
