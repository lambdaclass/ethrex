//! # ethrex_db
//!
//! A Paprika-inspired Ethereum state storage engine for ethrex.
//!
//! ## Architecture
//!
//! The library is split into two major components:
//!
//! 1. **Blockchain** - Handles "hot" blocks (latest/safe, not yet finalized)
//! 2. **PagedDb** - Handles finalized blocks (cold storage)
//!
//! ## Modules
//!
//! - `data` - Core data structures (NibblePath, SlottedArray)
//! - `store` - Page-based persistent storage
//! - `chain` - Block management for unfinalized state
//! - `merkle` - State root hash computation

pub mod chain;
pub mod data;
pub mod merkle;
pub mod store;
