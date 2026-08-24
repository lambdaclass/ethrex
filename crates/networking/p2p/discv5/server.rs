use crate::discovery::lookup::IterativeLookup;
use crate::discv5::messages::Message;
use crate::{
    discv5::messages::Packet,
    types::{Node, NodeRecord},
};
use bytes::Bytes;
use ethrex_common::H256;
use lru::LruCache;
use rand::RngCore;
use rustc_hash::{FxHashMap, FxHashSet};
use std::{
    net::{IpAddr, SocketAddr},
    num::NonZero,
    time::{Duration, Instant},
};
use tracing::trace;

/// Maximum number of entries in the per-IP WHOAREYOU rate limit cache.
pub const MAX_WHOAREYOU_RATE_LIMIT_ENTRIES: usize = 10_000;
/// Time window for collecting IP votes from PONG recipient_addr.
const IP_VOTE_WINDOW: Duration = Duration::from_secs(300);
/// Minimum number of agreeing votes required to update external IP.
const IP_VOTE_THRESHOLD: usize = 3;
/// Timeout for pending messages awaiting WhoAreYou response.
const MESSAGE_CACHE_TIMEOUT: Duration = Duration::from_secs(2);
/// Max age of a `session_ips` entry before it is evicted. Bounds the map: it is inserted
/// per discv5 handshake and (absent this) was never removed for nodes we don't keep as peers.
const SESSION_TTL: Duration = Duration::from_secs(3600);
/// How long an outstanding FINDNODE stays eligible to be answered. Generous
/// enough for a multi-packet NODES response over a slow link, short enough that
/// a request id cannot be replayed against us much later.
const PENDING_FINDNODE_TIMEOUT: Duration = Duration::from_secs(10);

/// Source IP a discv5 session was established from, paired with when it was recorded so stale
/// entries can be evicted (see `SESSION_TTL`).
#[derive(Debug, Clone)]
pub struct SessionSource {
    pub ip: IpAddr,
    pub established_at: Instant,
}

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
    /// Tracks the source IP that each session was established from, with the insertion time
    /// so stale entries can be evicted (see `SESSION_TTL`).
    pub session_ips: FxHashMap<H256, SessionSource>,
    /// Collects recipient_addr IPs from PONGs for external IP detection via majority voting.
    pub ip_votes: FxHashMap<IpAddr, FxHashSet<H256>>,
    /// When the current IP voting period started. None if no votes received yet.
    pub ip_vote_period_start: Option<Instant>,
    /// Whether the first (fast) voting round has completed.
    pub first_ip_vote_round_completed: bool,
    /// Currently active iterative lookups.
    pub active_lookups: Vec<IterativeLookup>,
    /// FINDNODE requests we sent and have not yet timed out, keyed by
    /// (peer node id, request id). A NODES response is only accepted if it
    /// matches one of these: without the check, any peer with a session can
    /// push arbitrary ENRs into the peer table by sending an unsolicited
    /// NODES, and we then serve them back from our own FINDNODE responses.
    /// Entries are not removed on first use, because a single response may be
    /// split across up to `total` packets sharing one request id; they expire
    /// via `PENDING_FINDNODE_TIMEOUT` instead.
    pub pending_findnodes: FxHashMap<(H256, Bytes), Instant>,
}

impl Default for Discv5State {
    fn default() -> Self {
        Self {
            counter: 0,
            pending_by_nonce: Default::default(),
            pending_challenges: Default::default(),
            pending_findnodes: Default::default(),
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
            active_lookups: Vec::new(),
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

        let before_messages = self.pending_by_nonce.len();
        self.pending_by_nonce
            .retain(|_nonce, (_node, _message, timestamp)| {
                now.duration_since(*timestamp) < MESSAGE_CACHE_TIMEOUT
            });
        let removed_messages = before_messages - self.pending_by_nonce.len();

        self.pending_findnodes
            .retain(|_key, timestamp| now.duration_since(*timestamp) < PENDING_FINDNODE_TIMEOUT);

        let before_challenges = self.pending_challenges.len();
        self.pending_challenges
            .retain(|_src_id, (_challenge_data, timestamp, _raw)| {
                now.duration_since(*timestamp) < MESSAGE_CACHE_TIMEOUT
            });
        let removed_challenges = before_challenges - self.pending_challenges.len();

        let before_sessions = self.session_ips.len();
        self.session_ips
            .retain(|_node_id, source| now.duration_since(source.established_at) < SESSION_TTL);
        let removed_sessions = before_sessions - self.session_ips.len();

        let total_removed = removed_messages + removed_challenges + removed_sessions;
        if total_removed > 0 {
            trace!(
                protocol = "discv5",
                "Cleaned up {} stale entries ({} messages, {} challenges)",
                total_removed,
                removed_messages,
                removed_challenges,
            );
        }

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

/// Points `local_node` and its ENR at `new_ip`, bumping `seq` once and re-signing.
///
/// Sets whichever of the record's `ip`/`ip6` entries matches `new_ip` and clears
/// the other. ENRs may carry both, but a [`Node`] holds a single [`IpAddr`], so
/// the record ethrex owns describes one address and a leftover entry from the
/// family we just moved away from would send peers to an address we dropped.
/// Every other entry is left as it is.
///
/// [`NodeRecord::edit`] commits only once the new signature is in hand, so a
/// failed re-sign leaves both the node and the record untouched.
pub(crate) fn update_local_ip(
    local_node: &mut Node,
    local_node_record: &mut NodeRecord,
    signer: &secp256k1::SecretKey,
    new_ip: IpAddr,
) {
    let edited = local_node_record.edit(signer, |pairs| match new_ip.to_canonical() {
        IpAddr::V4(ip) => {
            pairs.ip = Some(ip);
            pairs.ip6 = None;
        }
        IpAddr::V6(ip) => {
            pairs.ip6 = Some(ip);
            pairs.ip = None;
        }
    });
    if let Err(err) = edited {
        tracing::error!(%new_ip, %err, "Failed to re-sign ENR for IP update");
        return;
    }

    local_node.ip = new_ip;
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
    use crate::utils::public_key_from_signing_key;
    use ethrex_common::types::ForkId;
    use rand::{SeedableRng, rngs::StdRng};
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn make_test_state() -> Discv5State {
        Discv5State::default()
    }

    const NEW_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7));
    const QUIC_PORT: u16 = 9001;

    /// A local identity whose record advertises `quic`, an entry
    /// [`NodeRecord::from_node`] cannot derive and an IP update must not destroy.
    fn local_identity(eth: Option<ForkId>) -> (Node, NodeRecord, secp256k1::SecretKey) {
        let signer = secp256k1::SecretKey::from_slice(&[
            16, 125, 177, 238, 167, 212, 168, 215, 239, 165, 77, 224, 199, 143, 55, 205, 9, 194,
            87, 139, 92, 46, 30, 191, 74, 37, 68, 242, 38, 225, 104, 246,
        ])
        .unwrap();
        let node = Node::new(
            IpAddr::from(Ipv4Addr::LOCALHOST),
            30303,
            30303,
            public_key_from_signing_key(&signer),
        );
        let mut record = NodeRecord::from_node(&node, 1, &signer).unwrap();
        record
            .edit(&signer, |pairs| {
                pairs.eth = eth;
                assert!(pairs.set_extra_int(b"quic", QUIC_PORT.into()));
            })
            .unwrap();
        (node, record, signer)
    }

    #[test]
    fn update_local_ip_preserves_entries_it_does_not_touch() {
        // Rebuilding the record from the `Node` carries only what `from_node`
        // knows how to derive, so a `quic` entry the caller added separately was
        // silently dropped the first time discv5's IP voting fired.
        let (mut node, mut record, signer) = local_identity(None);

        update_local_ip(&mut node, &mut record, &signer, NEW_IP);

        assert_eq!(node.ip, NEW_IP);
        assert_eq!(record.pairs().ip, Some(Ipv4Addr::new(203, 0, 113, 7)));
        assert_eq!(
            record.pairs().extra_int::<u16>(b"quic"),
            Some(QUIC_PORT),
            "an entry the update never mentions must survive it"
        );
        assert!(
            record.verify_signature(),
            "must be re-signed after the edit"
        );
    }

    #[test]
    fn update_local_ip_bumps_seq_exactly_once() {
        // With a fork id present the rebuild path bumped twice: once for the
        // record built at `seq + 1`, then again inside `set_fork_id`. Every extra
        // bump is a wasted signature and a spurious re-gossip.
        let fork_id = ForkId {
            fork_hash: ethrex_common::H32::from_low_u64_be(0xdead_beef),
            fork_next: 42,
        };
        let (mut node, mut record, signer) = local_identity(Some(fork_id.clone()));
        let seq_before = record.seq;

        update_local_ip(&mut node, &mut record, &signer, NEW_IP);

        assert_eq!(record.seq, seq_before + 1);
        assert_eq!(record.get_fork_id(), Some(&fork_id));
    }

    #[test]
    fn update_local_ip_clears_the_address_family_it_moves_away_from() {
        // The only case where clearing the other entry is load-bearing: both
        // tests above start from an `ip`-only record, so `pairs.ip6 = None` runs
        // but can never be seen to change anything. Left behind, the stale `ip6`
        // would keep pointing peers at an address we no longer answer on.
        let (mut node, mut record, signer) = local_identity(None);
        let old_ipv6 = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);
        record
            .edit(&signer, |pairs| {
                pairs.ip = None;
                pairs.ip6 = Some(old_ipv6);
            })
            .unwrap();
        assert_eq!(record.pairs().ip6, Some(old_ipv6));

        update_local_ip(&mut node, &mut record, &signer, NEW_IP);

        assert_eq!(record.pairs().ip, Some(Ipv4Addr::new(203, 0, 113, 7)));
        assert_eq!(record.pairs().ip6, None, "the stale entry must be cleared");
        assert!(record.verify_signature());
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
