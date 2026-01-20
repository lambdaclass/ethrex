//! # ethrex L2
//!
//! Layer 2 rollup implementation for the ethrex Ethereum client.
//!
//! ## Overview
//!
//! This crate implements ethrex's L2 rollup, a zkEVM-based optimistic/validity rollup
//! that settles on Ethereum L1. It includes the sequencer, prover coordination,
//! based sequencing support, and a terminal-based monitor.
//!
//! ## Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`sequencer`] | L2 sequencer services (BlockProducer, L1Committer, etc.) |
//! | [`based`] | Based sequencing support with BlockFetcher |
//! | [`monitor`] | TUI dashboard for monitoring rollup state |
//! | [`utils`] | State reconstruction and helper utilities |
//! | [`errors`] | Error types for L2 operations |
//!
//! ## Quick Start
//!
//! ```ignore
//! use ethrex_l2::{start_l2, SequencerConfig};
//!
//! let (committer, producer, driver) = start_l2(
//!     store,
//!     rollup_store,
//!     blockchain,
//!     config,
//!     cancellation_token,
//!     l2_url,
//!     genesis,
//!     checkpoints_dir,
//! ).await?;
//! ```
//!
//! ## Operating Modes
//!
//! - **Standard Sequencing**: Collects transactions, builds blocks, commits to L1
//! - **Based Sequencing**: Fetches sequenced transactions from L1
//! - **Syncing Mode**: Recovers state from a specific batch number
//!
//! ## Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `l2` | Enable L2 functionality (default) |
//! | `sp1` | SP1 prover backend |
//! | `risc0` | RISC0 prover backend |
//! | `metrics` | Prometheus metrics |

pub mod based;
pub mod errors;
pub mod monitor;
pub mod sequencer;
pub mod utils;

pub use based::block_fetcher::BlockFetcher;
pub use sequencer::configs::{
    BasedConfig, BlockFetcherConfig, BlockProducerConfig, CommitterConfig, EthConfig,
    L1WatcherConfig, ProofCoordinatorConfig, SequencerConfig, StateUpdaterConfig,
};
pub use sequencer::start_l2;
