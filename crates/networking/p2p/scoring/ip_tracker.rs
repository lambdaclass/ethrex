//! IP colocation tracking for sybil attack protection.
//!
//! Tracks peers sharing the same /24 IP prefix and applies penalties
//! to prevent sybil attacks where an attacker controls many peers
//! from the same network.

use std::collections::HashMap;
use std::net::IpAddr;

use ethrex_common::H256;

/// Default threshold before penalties apply.
const DEFAULT_COLOCATION_THRESHOLD: usize = 3;

/// Default penalty factor for colocation.
const DEFAULT_PENALTY_FACTOR: f64 = 1.5;

/// Represents a /24 (IPv4) or /48 (IPv6) network prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IpPrefix {
    /// First three octets for IPv4, first six bytes for IPv6
    bytes: [u8; 6],
    /// Whether this is an IPv6 prefix
    is_ipv6: bool,
}

impl IpPrefix {
    /// Creates a prefix from an IP address.
    ///
    /// For IPv4: uses /24 (first 3 octets)
    /// For IPv6: uses /48 (first 6 bytes)
    pub fn from_ip(ip: IpAddr) -> Self {
        match ip {
            IpAddr::V4(ipv4) => {
                let octets = ipv4.octets();
                Self {
                    bytes: [octets[0], octets[1], octets[2], 0, 0, 0],
                    is_ipv6: false,
                }
            }
            IpAddr::V6(ipv6) => {
                let octets = ipv6.octets();
                Self {
                    bytes: [octets[0], octets[1], octets[2], octets[3], octets[4], octets[5]],
                    is_ipv6: true,
                }
            }
        }
    }

    /// Returns true if this is an IPv6 prefix.
    pub fn is_ipv6(&self) -> bool {
        self.is_ipv6
    }

    /// Returns the prefix as a displayable string.
    pub fn display(&self) -> String {
        if self.is_ipv6 {
            format!(
                "{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}::/48",
                self.bytes[0],
                self.bytes[1],
                self.bytes[2],
                self.bytes[3],
                self.bytes[4],
                self.bytes[5]
            )
        } else {
            format!("{}.{}.{}.0/24", self.bytes[0], self.bytes[1], self.bytes[2])
        }
    }
}

impl std::fmt::Display for IpPrefix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

/// Tracks IP colocation for sybil protection.
///
/// Monitors how many peers share the same network prefix and calculates
/// penalty factors for peers in congested prefixes.
#[derive(Debug, Clone)]
pub struct IpColocationTracker {
    /// Maps IP prefix to set of peer IDs
    prefix_peers: HashMap<IpPrefix, Vec<H256>>,
    /// Maps peer ID to their IP prefix
    peer_prefix: HashMap<H256, IpPrefix>,
    /// Threshold before penalties apply
    threshold: usize,
    /// Penalty factor (multiplied by (count - threshold)^2)
    penalty_factor: f64,
}

impl Default for IpColocationTracker {
    fn default() -> Self {
        Self::new(DEFAULT_COLOCATION_THRESHOLD, DEFAULT_PENALTY_FACTOR)
    }
}

impl IpColocationTracker {
    /// Creates a new tracker with custom threshold and penalty factor.
    ///
    /// # Arguments
    ///
    /// * `threshold` - Number of peers allowed before penalties apply
    /// * `penalty_factor` - Multiplier for the quadratic penalty
    pub fn new(threshold: usize, penalty_factor: f64) -> Self {
        Self {
            prefix_peers: HashMap::new(),
            peer_prefix: HashMap::new(),
            threshold,
            penalty_factor,
        }
    }

    /// Registers a peer's IP address.
    pub fn add_peer(&mut self, peer_id: H256, ip: IpAddr) {
        let prefix = IpPrefix::from_ip(ip);

        // Remove from old prefix if exists
        if let Some(old_prefix) = self.peer_prefix.remove(&peer_id) {
            if let Some(peers) = self.prefix_peers.get_mut(&old_prefix) {
                peers.retain(|p| *p != peer_id);
                if peers.is_empty() {
                    self.prefix_peers.remove(&old_prefix);
                }
            }
        }

        // Add to new prefix
        self.peer_prefix.insert(peer_id, prefix);
        self.prefix_peers
            .entry(prefix)
            .or_insert_with(Vec::new)
            .push(peer_id);
    }

    /// Removes a peer from tracking.
    pub fn remove_peer(&mut self, peer_id: &H256) {
        if let Some(prefix) = self.peer_prefix.remove(peer_id) {
            if let Some(peers) = self.prefix_peers.get_mut(&prefix) {
                peers.retain(|p| p != peer_id);
                if peers.is_empty() {
                    self.prefix_peers.remove(&prefix);
                }
            }
        }
    }

    /// Calculates the colocation penalty for a peer.
    ///
    /// Returns a value >= 0.0 where:
    /// - 0.0 means no penalty
    /// - Higher values indicate more penalty
    ///
    /// The penalty is quadratic: `factor * (count - threshold)^2`
    pub fn colocation_penalty(&self, peer_id: &H256) -> f64 {
        let Some(prefix) = self.peer_prefix.get(peer_id) else {
            return 0.0;
        };

        let count = self
            .prefix_peers
            .get(prefix)
            .map(|peers| peers.len())
            .unwrap_or(0);

        if count <= self.threshold {
            return 0.0;
        }

        let excess = (count - self.threshold) as f64;
        self.penalty_factor * excess * excess
    }

    /// Returns the number of peers in the same prefix as the given peer.
    pub fn peers_in_same_prefix(&self, peer_id: &H256) -> usize {
        let Some(prefix) = self.peer_prefix.get(peer_id) else {
            return 0;
        };

        self.prefix_peers
            .get(prefix)
            .map(|peers| peers.len())
            .unwrap_or(0)
    }

    /// Returns the prefix for a peer, if known.
    pub fn prefix_for_peer(&self, peer_id: &H256) -> Option<IpPrefix> {
        self.peer_prefix.get(peer_id).copied()
    }

    /// Returns all prefixes that exceed the threshold.
    pub fn congested_prefixes(&self) -> Vec<(IpPrefix, usize)> {
        self.prefix_peers
            .iter()
            .filter(|(_, peers)| peers.len() > self.threshold)
            .map(|(prefix, peers)| (*prefix, peers.len()))
            .collect()
    }

    /// Returns the total number of tracked peers.
    pub fn peer_count(&self) -> usize {
        self.peer_prefix.len()
    }

    /// Returns the number of unique prefixes.
    pub fn prefix_count(&self) -> usize {
        self.prefix_peers.len()
    }

    /// Clears all tracking data.
    pub fn clear(&mut self) {
        self.prefix_peers.clear();
        self.peer_prefix.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn make_peer_id(n: u8) -> H256 {
        H256([n; 32])
    }

    #[test]
    fn test_ip_prefix_ipv4() {
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
        let prefix = IpPrefix::from_ip(ip);

        assert!(!prefix.is_ipv6());
        assert_eq!(prefix.display(), "192.168.1.0/24");

        // Same /24 should produce same prefix
        let ip2 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 200));
        let prefix2 = IpPrefix::from_ip(ip2);
        assert_eq!(prefix, prefix2);

        // Different /24 should produce different prefix
        let ip3 = IpAddr::V4(Ipv4Addr::new(192, 168, 2, 100));
        let prefix3 = IpPrefix::from_ip(ip3);
        assert_ne!(prefix, prefix3);
    }

    #[test]
    fn test_ip_prefix_ipv6() {
        let ip = IpAddr::V6(Ipv6Addr::new(0x2001, 0x0db8, 0x85a3, 0x0000, 0x0000, 0x8a2e, 0x0370, 0x7334));
        let prefix = IpPrefix::from_ip(ip);

        assert!(prefix.is_ipv6());
    }

    #[test]
    fn test_no_penalty_under_threshold() {
        let mut tracker = IpColocationTracker::new(3, 1.5);

        // Add 3 peers from same subnet (at threshold)
        for i in 0..3 {
            tracker.add_peer(make_peer_id(i), IpAddr::V4(Ipv4Addr::new(192, 168, 1, i)));
        }

        // No penalty at threshold
        assert_eq!(tracker.colocation_penalty(&make_peer_id(0)), 0.0);
    }

    #[test]
    fn test_quadratic_penalty_above_threshold() {
        let mut tracker = IpColocationTracker::new(3, 1.0);

        // Add 5 peers from same subnet (2 over threshold)
        for i in 0..5 {
            tracker.add_peer(make_peer_id(i), IpAddr::V4(Ipv4Addr::new(192, 168, 1, i)));
        }

        // Penalty should be 1.0 * (5-3)^2 = 4.0
        let penalty = tracker.colocation_penalty(&make_peer_id(0));
        assert!((penalty - 4.0).abs() < 0.01, "Expected 4.0, got {}", penalty);

        // Add more peers
        for i in 5..8 {
            tracker.add_peer(make_peer_id(i), IpAddr::V4(Ipv4Addr::new(192, 168, 1, i)));
        }

        // Penalty should be 1.0 * (8-3)^2 = 25.0
        let penalty = tracker.colocation_penalty(&make_peer_id(0));
        assert!((penalty - 25.0).abs() < 0.01, "Expected 25.0, got {}", penalty);
    }

    #[test]
    fn test_remove_peer() {
        let mut tracker = IpColocationTracker::new(2, 1.0);

        // Add 4 peers from same subnet
        for i in 0..4 {
            tracker.add_peer(make_peer_id(i), IpAddr::V4(Ipv4Addr::new(192, 168, 1, i)));
        }

        assert_eq!(tracker.peers_in_same_prefix(&make_peer_id(0)), 4);

        // Remove one peer
        tracker.remove_peer(&make_peer_id(0));

        // Remaining peers should now only see 3
        assert_eq!(tracker.peers_in_same_prefix(&make_peer_id(1)), 3);

        // Removed peer should have no penalty
        assert_eq!(tracker.colocation_penalty(&make_peer_id(0)), 0.0);
    }

    #[test]
    fn test_different_subnets() {
        let mut tracker = IpColocationTracker::new(2, 1.0);

        // Add peers from different subnets
        tracker.add_peer(make_peer_id(0), IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
        tracker.add_peer(make_peer_id(1), IpAddr::V4(Ipv4Addr::new(192, 168, 2, 1)));
        tracker.add_peer(make_peer_id(2), IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));

        // Each peer is alone in their subnet
        assert_eq!(tracker.peers_in_same_prefix(&make_peer_id(0)), 1);
        assert_eq!(tracker.peers_in_same_prefix(&make_peer_id(1)), 1);
        assert_eq!(tracker.peers_in_same_prefix(&make_peer_id(2)), 1);

        // No penalties
        assert_eq!(tracker.colocation_penalty(&make_peer_id(0)), 0.0);
    }

    #[test]
    fn test_congested_prefixes() {
        let mut tracker = IpColocationTracker::new(2, 1.0);

        // Add peers to create one congested subnet
        for i in 0..5 {
            tracker.add_peer(make_peer_id(i), IpAddr::V4(Ipv4Addr::new(192, 168, 1, i)));
        }

        // Add peers to a normal subnet
        tracker.add_peer(make_peer_id(10), IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
        tracker.add_peer(make_peer_id(11), IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)));

        let congested = tracker.congested_prefixes();
        assert_eq!(congested.len(), 1);
        assert_eq!(congested[0].1, 5); // 5 peers in congested subnet
    }

    #[test]
    fn test_peer_ip_update() {
        let mut tracker = IpColocationTracker::new(2, 1.0);

        // Add peer to one subnet
        tracker.add_peer(make_peer_id(0), IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(tracker.peers_in_same_prefix(&make_peer_id(0)), 1);

        // Move peer to another subnet
        tracker.add_peer(make_peer_id(0), IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));

        // Old prefix should be empty, new should have peer
        let prefix192 = IpPrefix::from_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
        assert!(tracker.prefix_peers.get(&prefix192).is_none() ||
                tracker.prefix_peers.get(&prefix192).unwrap().is_empty());

        assert_eq!(tracker.peers_in_same_prefix(&make_peer_id(0)), 1);
    }
}
