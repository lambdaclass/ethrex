//! # ethrex P2P Networking
//!
//! Peer-to-peer networking layer for the ethrex Ethereum client.
//!
//! ## Overview
//!
//! This crate implements the complete Ethereum P2P networking stack:
//! - **Discovery**: Node discovery using discv4 (stable) and discv5 (experimental)
//! - **RLPx**: ECIES-encrypted transport protocol for peer communication
//! - **eth Protocol**: Block and transaction propagation (eth/68, eth/69)
//! - **snap Protocol**: Fast state synchronization (snap/1)
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      Network Layer                           │
//! │  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────┐ │
//! │  │    Discovery    │  │     RLPx        │  │ Peer Handler│ │
//! │  │  (discv4/v5)    │  │   (Transport)   │  │  (Messages) │ │
//! │  └─────────────────┘  └─────────────────┘  └─────────────┘ │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!          ┌───────────────────┼───────────────────┐
//!          ▼                   ▼                   ▼
//! ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐
//! │   Sync Manager  │ │ TX Broadcaster  │ │ Snap/Full Sync  │
//! └─────────────────┘ └─────────────────┘ └─────────────────┘
//! ```
//!
//! ## Modules
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`network`] | Network initialization, P2P context, and peer management |
//! | [`peer_handler`] | Message handling for connected peers |
//! | [`sync_manager`] | High-level sync coordination and FCU handling |
//! | [`sync`] | Full and snap sync implementations with healing |
//! | [`tx_broadcaster`] | Transaction pool broadcasting to peers |
//! | [`discv4`] | Node discovery protocol v4 (Kademlia-based) |
//! | [`rlpx`] | RLPx encrypted transport (ECIES + AES-CTR) |
//! | [`types`] | P2P-specific types (Node, endpoint info) |
//!
//! ## Quick Start
//!
//! ```ignore
//! use ethrex_p2p::{start_network, SyncManager};
//!
//! // Start the P2P network
//! let (sync_manager, peer_handler) = start_network(
//!     udp_addr,
//!     tcp_addr,
//!     bootnodes,
//!     signer,
//!     storage,
//!     blockchain,
//! ).await?;
//!
//! // Trigger sync to a specific head
//! sync_manager.sync_to_head(block_hash);
//!
//! // Check current sync mode
//! let mode = sync_manager.sync_mode();  // Full or Snap
//! ```
//!
//! ## Wire Protocols
//!
//! | Protocol | Version | Description |
//! |----------|---------|-------------|
//! | eth | 68, 69 | Block and transaction exchange |
//! | snap | 1 | State snapshot synchronization |
//!
//! ## Sync Modes
//!
//! - **Full Sync**: Download and execute all blocks sequentially
//! - **Snap Sync**: Download state snapshots, then full sync recent blocks (~10,000)
//!
//! ## Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `c-kzg` | KZG commitment support (EIP-4844) - default |
//! | `sync-test` | Testing utilities for sync operations |
//! | `l2` | L2 rollup support with additional protocols |
//! | `metrics` | Prometheus metrics collection |
//! | `experimental-discv5` | Enable discv5 node discovery |

pub mod discv4;
#[cfg(feature = "experimental-discv5")]
pub mod discv5;
pub(crate) mod metrics;
pub mod network;
pub mod peer_handler;
pub mod rlpx;
pub(crate) mod snap;
pub mod sync;
pub mod sync_manager;
pub mod tx_broadcaster;
pub mod types;
pub mod utils;

pub use network::periodically_show_peer_stats;
pub use network::start_network;

#[cfg(not(feature = "experimental-discv5"))]
pub use discv4::peer_table;
#[cfg(not(feature = "experimental-discv5"))]
pub use discv4::server as discovery_server;
#[cfg(feature = "experimental-discv5")]
pub use discv5::peer_table;
#[cfg(feature = "experimental-discv5")]
pub use discv5::server as discovery_server;
