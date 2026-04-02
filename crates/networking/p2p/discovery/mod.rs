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

use std::time::Duration;

/// Smooth easing curve for discovery lookup intervals based on peer completion progress.
///
/// Shared by discv4, discv5, and RLPx initiator.
pub fn lookup_interval_function(progress: f64, lower_limit: f64, upper_limit: f64) -> Duration {
    // Smooth progression curve
    // See https://easings.net/#easeInOutCubic
    let ease_in_out_cubic = if progress < 0.5 {
        4.0 * progress.powf(3.0)
    } else {
        1.0 - ((-2.0 * progress + 2.0).powf(3.0)) / 2.0
    };
    Duration::from_micros(
        // Use `progress` here instead of `ease_in_out_cubic` for a linear function.
        (1000f64 * (ease_in_out_cubic * (upper_limit - lower_limit) + lower_limit)).round() as u64,
    )
}
