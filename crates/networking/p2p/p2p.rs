//! # ethrex P2P Networking
//!
//! Peer-to-peer networking layer for the ethrex Ethereum client.
//!
//! ## Overview
//!
//! This crate implements the Ethereum P2P networking stack:
//! - **Discovery**: Node discovery using discv4 (and experimental discv5)
//! - **RLPx**: Encrypted transport protocol for peer communication
//! - **eth Protocol**: Block and transaction propagation
//! - **snap Protocol**: Fast state synchronization
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      Network Layer                           │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │
//! │  │   discv4    │  │    RLPx     │  │   Peer Handler      │ │
//! │  │ (Discovery) │  │ (Transport) │  │   (Messages)        │ │
//! │  └─────────────┘  └─────────────┘  └─────────────────────┘ │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!           ┌──────────────────┼──────────────────┐
//!           ▼                  ▼                  ▼
//! ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐
//! │   Sync Manager  │ │ TX Broadcaster  │ │  Snap Sync      │
//! └─────────────────┘ └─────────────────┘ └─────────────────┘
//! ```
//!
//! ## Key Components
//!
//! - [`network`]: Network initialization and peer management
//! - [`peer_handler`]: Message handling for connected peers
//! - [`sync_manager`]: Block synchronization coordination
//! - [`sync`]: Full and snap sync implementations
//! - [`tx_broadcaster`]: Transaction pool broadcasting
//! - [`discv4`]: Node discovery protocol v4
//! - [`rlpx`]: RLPx encrypted transport
//!
//! ## Usage
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
//! // Start synchronization
//! sync_manager.start_sync().await?;
//! ```
//!
//! ## Protocols
//!
//! - **eth/68**: Block and transaction exchange
//! - **snap/1**: State snapshot synchronization
//!
pub(crate) mod backend;
pub mod discovery;
pub mod discv4;
pub mod discv5;
pub(crate) mod metrics;
pub mod network;
pub mod peer_handler;
pub mod peer_table;
pub mod rlpx;
pub mod snap;
pub mod sync;
pub mod sync_manager;
pub mod tx_broadcaster;
pub mod types;
pub mod utils;

pub use discovery::DiscoveryConfig;
pub use network::periodically_show_peer_stats;
pub use network::start_network;
