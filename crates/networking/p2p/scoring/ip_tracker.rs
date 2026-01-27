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
                    bytes: [
                        octets[0], octets[1], octets[2], octets[3], octets[4], octets[5],
                    ],
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
        if let Some(old_prefix) = self.peer_prefix.remove(&peer_id)
            && let Some(peers) = self.prefix_peers.get_mut(&old_prefix)
        {
            peers.retain(|p| *p != peer_id);
            if peers.is_empty() {
                self.prefix_peers.remove(&old_prefix);
            }
        }

        // Add to new prefix
        self.peer_prefix.insert(peer_id, prefix);
        self.prefix_peers.entry(prefix).or_default().push(peer_id);
    }

    /// Removes a peer from tracking.
    pub fn remove_peer(&mut self, peer_id: &H256) {
        if let Some(prefix) = self.peer_prefix.remove(peer_id)
            && let Some(peers) = self.prefix_peers.get_mut(&prefix)
        {
            peers.retain(|p| p != peer_id);
            if peers.is_empty() {
                self.prefix_peers.remove(&prefix);
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
