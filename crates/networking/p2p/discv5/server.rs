use crate::discv5::messages::Message;
use crate::{
    discv5::messages::Packet,
    types::{Node, NodeRecord},
};
use ethrex_common::H256;
use lru::LruCache;
use rand::RngCore;
use rustc_hash::{FxHashMap, FxHashSet};
use std::{
    net::{IpAddr, SocketAddr},
    num::NonZero,
    time::{Duration, Instant},
};

/// Maximum number of entries in the per-IP WHOAREYOU rate limit cache.
pub const MAX_WHOAREYOU_RATE_LIMIT_ENTRIES: usize = 10_000;
/// Time window for collecting IP votes from PONG recipient_addr.
const IP_VOTE_WINDOW: Duration = Duration::from_secs(300);
/// Minimum number of agreeing votes required to update external IP.
const IP_VOTE_THRESHOLD: usize = 3;
/// Timeout for pending messages awaiting WhoAreYou response.
const MESSAGE_CACHE_TIMEOUT: Duration = Duration::from_secs(2);

/// Discv5-specific state held within the unified DiscoveryServer.
#[derive(Debug)]
pub struct Discv5State {
    /// Outgoing message count, used for nonce generation as per the spec.
    pub counter: u32,
    /// Pending outgoing messages awaiting WhoAreYou response, keyed by nonce.
    pub pending_by_nonce: FxHashMap<[u8; 12], (Node, Message, Instant)>,
    /// Pending WhoAreYou challenges awaiting Handshake response, keyed by src_id.
    /// Tuple: (challenge_data, timestamp, encoded_packet_bytes).
    pub pending_challenges: FxHashMap<H256, (Vec<u8>, Instant, Vec<u8>)>,
    /// Tracks last WHOAREYOU send time per (source IP, node ID) to prevent amplification attacks.
    pub whoareyou_rate_limit: LruCache<(IpAddr, H256), Instant>,
    /// Global WHOAREYOU rate limit: count of packets sent in the current second.
    pub whoareyou_global_count: u32,
    /// Start of the current global rate limit window.
    pub whoareyou_global_window_start: Instant,
    /// Tracks the source IP that each session was established from.
    pub session_ips: FxHashMap<H256, IpAddr>,
    /// Collects recipient_addr IPs from PONGs for external IP detection via majority voting.
    pub ip_votes: FxHashMap<IpAddr, FxHashSet<H256>>,
    /// When the current IP voting period started. None if no votes received yet.
    pub ip_vote_period_start: Option<Instant>,
    /// Whether the first (fast) voting round has completed.
    pub first_ip_vote_round_completed: bool,
}

impl Default for Discv5State {
    fn default() -> Self {
        Self {
            counter: 0,
            pending_by_nonce: Default::default(),
            pending_challenges: Default::default(),
            whoareyou_rate_limit: LruCache::new(
                NonZero::new(MAX_WHOAREYOU_RATE_LIMIT_ENTRIES)
                    .expect("MAX_WHOAREYOU_RATE_LIMIT_ENTRIES must be non-zero"),
            ),
            whoareyou_global_count: 0,
            whoareyou_global_window_start: Instant::now(),
            session_ips: Default::default(),
            ip_votes: Default::default(),
            ip_vote_period_start: None,
            first_ip_vote_round_completed: false,
        }
    }
}

impl Discv5State {
    /// Generates a 96-bit AES-GCM nonce.
    /// Encodes the current outgoing message count into the first 32 bits
    /// and fills the remaining 64 bits with random data.
    pub fn next_nonce<R: RngCore>(&mut self, rng: &mut R) -> [u8; 12] {
        let counter = self.counter;
        self.counter = self.counter.wrapping_add(1);

        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&counter.to_be_bytes());
        rng.fill_bytes(&mut nonce[4..]);
        nonce
    }

    /// Remove stale entries from caches.
    /// Returns `Some(ip)` if a timed-out IP voting round produced a winning IP to apply.
    pub fn cleanup_stale_entries(&mut self) -> Option<IpAddr> {
        let now = Instant::now();

        self.pending_by_nonce
            .retain(|_nonce, (_node, _message, timestamp)| {
                now.duration_since(*timestamp) < MESSAGE_CACHE_TIMEOUT
            });

        self.pending_challenges
            .retain(|_src_id, (_challenge_data, timestamp, _raw)| {
                now.duration_since(*timestamp) < MESSAGE_CACHE_TIMEOUT
            });

        if let Some(start) = self.ip_vote_period_start
            && now.duration_since(start) >= IP_VOTE_WINDOW
        {
            return self.finalize_ip_vote_round();
        }
        None
    }

    /// Records an IP vote from a PONG recipient_addr.
    /// Returns `Some(ip)` if the voting round ended with a winning IP to apply.
    pub fn record_ip_vote(&mut self, reported_ip: IpAddr, voter_id: H256) -> Option<IpAddr> {
        if Self::is_private_ip(reported_ip) {
            return None;
        }

        let now = Instant::now();

        if self.ip_vote_period_start.is_none() {
            self.ip_vote_period_start = Some(now);
        }

        self.ip_votes
            .entry(reported_ip)
            .or_default()
            .insert(voter_id);

        let total_votes: usize = self.ip_votes.values().map(|v| v.len()).sum();
        let round_ended = if !self.first_ip_vote_round_completed {
            total_votes >= IP_VOTE_THRESHOLD
        } else {
            self.ip_vote_period_start
                .is_some_and(|start| now.duration_since(start) >= IP_VOTE_WINDOW)
        };

        if round_ended {
            return self.finalize_ip_vote_round();
        }
        None
    }

    /// Finalizes the current voting round.
    /// Returns `Some(winning_ip)` if a winner reached the threshold and should be applied.
    fn finalize_ip_vote_round(&mut self) -> Option<IpAddr> {
        let winner = self
            .ip_votes
            .iter()
            .map(|(ip, voters)| (*ip, voters.len()))
            .max_by_key(|(_, count)| *count);

        let result = winner.and_then(|(winning_ip, vote_count)| {
            (vote_count >= IP_VOTE_THRESHOLD).then_some(winning_ip)
        });

        self.ip_votes.clear();
        self.ip_vote_period_start = Some(Instant::now());
        self.first_ip_vote_round_completed = true;

        result
    }

    /// Returns true if the IP is private/local (not useful for external connectivity).
    pub fn is_private_ip(ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
            IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    // unique local (fc00::/7)
                    || (v6.segments()[0] & 0xfe00) == 0xfc00
                    // link-local (fe80::/10)
                    || (v6.segments()[0] & 0xffc0) == 0xfe80
            }
        }
    }
}

/// Updates local node IP and re-signs the ENR with incremented seq.
pub(crate) fn update_local_ip(
    local_node: &mut Node,
    local_node_record: &mut NodeRecord,
    signer: &secp256k1::SecretKey,
    new_ip: IpAddr,
) {
    let mut updated_node = local_node.clone();
    updated_node.ip = new_ip;
    let new_seq = local_node_record.seq + 1;
    let Ok(mut new_record) = NodeRecord::from_node(&updated_node, new_seq, signer) else {
        tracing::error!(%new_ip, "Failed to create new ENR for IP update");
        return;
    };
    if let Some(fork_id) = local_node_record.get_fork_id().cloned()
        && new_record.set_fork_id(fork_id, signer).is_err()
    {
        tracing::error!(%new_ip, "Failed to set fork_id in new ENR, aborting IP update");
        return;
    }
    local_node.ip = new_ip;
    *local_node_record = new_record;
}

#[derive(Debug, Clone)]
pub struct Discv5Message {
    pub(crate) from: SocketAddr,
    pub(crate) packet: Packet,
}

impl Discv5Message {
    pub fn from(packet: Packet, from: SocketAddr) -> Self {
        Self { from, packet }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{SeedableRng, rngs::StdRng};

    fn make_test_state() -> Discv5State {
        Discv5State::default()
    }

    #[tokio::test]
    async fn test_next_nonce_counter() {
        let mut rng = StdRng::seed_from_u64(7);
        let mut state = make_test_state();

        let n1 = state.next_nonce(&mut rng);
        let n2 = state.next_nonce(&mut rng);

        assert_eq!(&n1[..4], &[0, 0, 0, 0]);
        assert_eq!(&n2[..4], &[0, 0, 0, 1]);
        assert_ne!(&n1[4..], &n2[4..]);
    }

    #[tokio::test]
    async fn test_ip_voting_returns_winning_ip() {
        let mut state = make_test_state();

        let new_ip: IpAddr = "203.0.113.50".parse().unwrap();
        let voter1 = H256::from_low_u64_be(1);
        let voter2 = H256::from_low_u64_be(2);
        let voter3 = H256::from_low_u64_be(3);

        assert_eq!(state.record_ip_vote(new_ip, voter1), None);
        assert_eq!(state.record_ip_vote(new_ip, voter2), None);
        // Third vote triggers round end, returns the winning IP
        assert_eq!(state.record_ip_vote(new_ip, voter3), Some(new_ip));
        assert!(state.ip_votes.is_empty());
    }

    #[tokio::test]
    async fn test_ip_voting_same_peer_votes_once() {
        let mut state = make_test_state();

        let new_ip: IpAddr = "203.0.113.50".parse().unwrap();
        let same_voter = H256::from_low_u64_be(1);

        state.record_ip_vote(new_ip, same_voter);
        state.record_ip_vote(new_ip, same_voter);
        state.record_ip_vote(new_ip, same_voter);

        assert_eq!(state.ip_votes.get(&new_ip).map(|v| v.len()), Some(1));
    }

    #[tokio::test]
    async fn test_ip_voting_ignores_private_ips() {
        let mut state = make_test_state();

        let voter1 = H256::from_low_u64_be(1);

        let private_ip: IpAddr = "192.168.1.100".parse().unwrap();
        state.record_ip_vote(private_ip, voter1);
        assert!(state.ip_votes.is_empty());

        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        state.record_ip_vote(loopback, voter1);
        assert!(state.ip_votes.is_empty());

        let public_ip: IpAddr = "203.0.113.50".parse().unwrap();
        state.record_ip_vote(public_ip, voter1);
        assert_eq!(state.ip_votes.get(&public_ip).map(|v| v.len()), Some(1));
    }

    #[tokio::test]
    async fn test_ip_voting_split_votes_no_winner() {
        let mut state = make_test_state();

        let ip1: IpAddr = "203.0.113.50".parse().unwrap();
        let ip2: IpAddr = "203.0.113.51".parse().unwrap();
        let voter1 = H256::from_low_u64_be(1);
        let voter2 = H256::from_low_u64_be(2);
        let voter3 = H256::from_low_u64_be(3);

        state.record_ip_vote(ip1, voter1);
        state.record_ip_vote(ip2, voter2);
        // ip1 has 2 votes, ip2 has 1 — ip1 wins but only has 2 < threshold 3
        assert_eq!(state.record_ip_vote(ip1, voter3), None);
        assert!(state.ip_votes.is_empty());
        assert!(state.first_ip_vote_round_completed);
    }

    #[tokio::test]
    async fn test_ip_vote_cleanup() {
        let mut state = make_test_state();

        let ip: IpAddr = "203.0.113.50".parse().unwrap();
        let voter1 = H256::from_low_u64_be(1);

        let mut voters = FxHashSet::default();
        voters.insert(voter1);
        state.ip_votes.insert(ip, voters);
        state.ip_vote_period_start = Some(Instant::now());
        assert_eq!(state.ip_votes.len(), 1);

        // Cleanup should retain votes (round hasn't timed out yet)
        assert_eq!(state.cleanup_stale_entries(), None);
        assert_eq!(state.ip_votes.len(), 1);
        assert!(!state.first_ip_vote_round_completed);
    }
}
