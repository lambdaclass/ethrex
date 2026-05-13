//! Discovery protocol implementation for running both discv4 and discv5 on a shared UDP port.
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

pub mod codec;
mod discv4_handlers;
mod discv5_handlers;
pub mod server;

pub use server::{DiscoveryServer, DiscoveryServerError, is_discv4_packet};

use std::time::Duration;

/// Configuration for which discovery protocols to enable.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    pub discv4_enabled: bool,
    pub discv5_enabled: bool,
    pub initial_lookup_interval: f64,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            discv4_enabled: true,
            discv5_enabled: true,
            initial_lookup_interval: INITIAL_LOOKUP_INTERVAL_MS,
        }
    }
}

/// Lookup interval constants shared by discv4, discv5, and RLPx initiator.
pub const INITIAL_LOOKUP_INTERVAL_MS: f64 = 100.0; // 10 per second
pub const LOOKUP_INTERVAL_MS: f64 = 600.0; // 100 per minute

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
        (1000f64 * (ease_in_out_cubic * (upper_limit - lower_limit) + lower_limit)).round() as u64,
    )
}
