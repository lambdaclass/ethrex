//! On-Demand Ethereum L1 Block Builder
//!
//! This crate provides an on-demand block builder that creates blocks
//! immediately when transactions arrive, without using a mempool.
//!
//! ## Features
//!
//! - **On-demand mode** (default): Builds a block immediately for each transaction
//! - **Interval mode**: Collects transactions and builds blocks at specified intervals
//! - **RPC logging**: All RPC calls are logged to console
//! - **Async response**: Returns transaction hash immediately (Ethereum behavior)
//!
//! ## Architecture
//!
//! The block builder is implemented as a GenServer using the `spawned` library.
//! It receives transactions via cast messages and builds blocks using the
//! existing `ethrex-blockchain` infrastructure.

pub mod banner;
pub mod builder;
pub mod error;

pub use banner::display_banner;
pub use builder::{BlockBuilder, BlockBuilderConfig, CallMsg, CastMsg, OutMsg};
pub use error::BlockBuilderError;
