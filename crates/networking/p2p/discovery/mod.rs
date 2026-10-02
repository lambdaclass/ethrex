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
pub mod ip_predictor;
pub mod lookup;
pub mod server;

pub use ip_predictor::IpPredictor;
pub use server::{DiscoveryServer, DiscoveryServerError, is_discv4_packet};

use crate::netrestrict::NetRestrict;
use std::time::Duration;

/// Configuration for which discovery protocols to enable.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    pub discv4_enabled: bool,
    pub discv5_enabled: bool,
    /// Set to true when `--nat extip:<addr>` was supplied; locks the IP predictor
    /// from overwriting the user-specified external address.
    pub nat_extip_set: bool,
    /// IP networks peers must fall in. Packets from outside are dropped before
    /// they are decoded, and bootnodes outside are not contacted.
    pub netrestrict: NetRestrict,
}

/// Lookup interval bounds for the RLPx initiator's connection attempts. The
/// discovery server has its own bounds, since each of its iterative lookups
/// emits far more traffic than a single connection attempt.
pub const INITIAL_LOOKUP_INTERVAL_MS: f64 = 100.0; // 10 per second
pub const LOOKUP_INTERVAL_MS: f64 = 600.0; // 100 per minute

/// Smooth easing curve for lookup intervals based on peer completion progress.
///
/// Shared by the discovery server and the RLPx initiator.
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

/// Interval before the *next* lookup starts, once the previous one has finished.
///
/// `empty_lookups_in_a_row` counts finished lookups that added nothing to the
/// peer table. Each one doubles the wait from `lower_limit`, up to
/// `upper_limit`: a network whose every node is already known stops being asked
/// the same question twice a second. The completion-based pacing still applies,
/// so the result is never shorter than [`lookup_interval_function`] alone.
pub fn next_lookup_interval(
    progress: f64,
    empty_lookups_in_a_row: u32,
    lower_limit: f64,
    upper_limit: f64,
) -> Duration {
    let paced = lookup_interval_function(progress, lower_limit, upper_limit);
    if empty_lookups_in_a_row == 0 {
        return paced;
    }
    // 2^16 already dwarfs any sane upper limit; the clamp only keeps `powi` finite.
    let backoff_ms =
        (lower_limit * 2f64.powi(empty_lookups_in_a_row.min(16) as i32)).min(upper_limit);
    paced.max(Duration::from_micros((1000f64 * backoff_ms).round() as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saturation_backoff_doubles_from_the_lower_limit_and_caps() {
        let at = |empty| next_lookup_interval(0.0, empty, 500.0, 10_000.0);
        assert_eq!(at(0), Duration::from_millis(500));
        assert_eq!(at(1), Duration::from_millis(1_000));
        assert_eq!(at(2), Duration::from_millis(2_000));
        assert_eq!(at(3), Duration::from_millis(4_000));
        assert_eq!(at(4), Duration::from_millis(8_000));
        assert_eq!(at(5), Duration::from_millis(10_000));
        assert_eq!(at(1_000), Duration::from_millis(10_000));
    }

    #[test]
    fn saturation_backoff_never_undercuts_completion_pacing() {
        let paced = lookup_interval_function(0.9, 500.0, 10_000.0);
        assert!(paced > Duration::from_millis(1_000));
        assert_eq!(next_lookup_interval(0.9, 1, 500.0, 10_000.0), paced);
    }
}
