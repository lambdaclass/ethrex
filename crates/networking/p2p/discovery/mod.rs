//! Discovery multiplexer for running both discv4 and discv5 on a shared UDP port.
//!
//! This module provides packet discrimination between discv4 and discv5 protocols
//! and routes packets to the appropriate protocol handler.
//!
//! ## Packet Discrimination Strategy
//!
//! DiscV4 packets have a deterministic structure:
//! `hash (32 bytes) || signature (65 bytes) || type (1 byte) || data`
//! where `hash == keccak256(rest_of_packet)`.
//!
//! **Discrimination logic:**
//! 1. If packet length >= 98 bytes AND `packet[0..32] == keccak256(packet[32..])` → DiscV4
//! 2. Otherwise → DiscV5
//!
//! This is O(1) with a single keccak hash and has negligible false positive probability (2^-256).

pub mod codec;
mod multiplexer;

pub use multiplexer::{
    DiscoveryConfig, DiscoveryMultiplexer, DiscoveryMultiplexerError, is_discv4_packet,
};
