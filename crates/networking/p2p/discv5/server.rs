use crate::discovery::lookup::{IterativeLookup, LookupId};
use crate::discovery::query::NodeQuery;
use crate::discv5::messages::Message;
use crate::{
    discv5::messages::Packet,
    types::{Node, NodeRecord},
};
use bytes::Bytes;
use ethrex_common::H256;
use lru::LruCache;
use rand::RngCore;
use rustc_hash::FxHashMap;
use std::{
    net::{IpAddr, SocketAddr},
    num::NonZero,
    time::{Duration, Instant},
};
use tracing::trace;

/// Maximum number of entries in the per-IP WHOAREYOU rate limit cache.
pub const MAX_WHOAREYOU_RATE_LIMIT_ENTRIES: usize = 10_000;
/// Timeout for pending messages awaiting WhoAreYou response.
const MESSAGE_CACHE_TIMEOUT: Duration = Duration::from_secs(2);
/// Max time a discv5 session may sit unused before it is evicted. Bounds both halves of
/// a session: the symmetric keys in the contact table and the `session_ips` entry that
/// guards them. Both are inserted per handshake, by a remote peer's schedule, so without
/// this neither is ever removed for a node we do not keep as a peer.
///
/// Idle rather than absolute, so a peer we are still talking to is never made to
/// re-handshake mid-conversation: that costs a WHOAREYOU round trip and drops the message
/// that triggered it if the exchange outruns `MESSAGE_CACHE_TIMEOUT`. The
/// handshake-once-and-vanish population this exists to bound is unaffected, since nothing
/// refreshes those.
///
/// Shared rather than duplicated: when the keys outlive their `session_ips` entry, the
/// IP-rebinding check in `discv5_handle_ordinary` silently stops applying to a session
/// that still decrypts. Both halves are refreshed together, for that same reason.
pub const SESSION_TTL: Duration = Duration::from_secs(3600);
/// How long an outstanding FINDNODE stays eligible to be answered. Generous
/// enough for a multi-packet NODES response over a slow link, short enough that
/// a request id cannot be replayed against us much later.
const PENDING_FINDNODE_TIMEOUT: Duration = Duration::from_secs(10);
/// The most NODES packets one FINDNODE's answer is taken from: the first
/// packet's `total` is believed only up to here, and a packet past the answer
/// is dropped as unsolicited. geth's limit (`totalNodesResponseLimit`).
///
/// Without one, a peer could answer a single FINDNODE with packet after packet
/// until `PENDING_FINDNODE_TIMEOUT`, each full of records made up to match what
/// a predicate query looks for, and the query would spend its slots asking
/// those records' nodes. ethrex's own answers fit, see
/// `discv5_handle_find_node`.
pub(crate) const MAX_NODES_RESPONSES: u64 = 5;

/// Source IP a discv5 session was established from, paired with when the session was last
/// used so idle entries can be evicted (see `SESSION_TTL`).
///
/// The IP itself is never refreshed, only the timestamp: rebinding is still detected for
/// the whole life of the session.
#[derive(Debug, Clone)]
pub struct SessionSource {
    pub ip: IpAddr,
    pub last_used: Instant,
}

/// A FINDNODE we sent whose NODES answer is not complete yet.
#[derive(Debug, Clone, Copy)]
pub struct PendingFindNode {
    /// When it was sent, or last re-sent after a WHOAREYOU.
    pub sent_at: Instant,
    /// NODES packets still accepted: unknown until the first one arrives, then
    /// that packet's `total`, capped at [`MAX_NODES_RESPONSES`], counting down.
    packets_due: Option<u64>,
}

/// A NODES packet that answers a FINDNODE of ours, as
/// [`Discv5State::accept_nodes_packet`] counted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AcceptedNodes {
    /// The lookup that sent the FINDNODE, if any.
    pub(crate) owner: Option<LookupId>,
    /// Whether it completes the answer. The FINDNODE is then no longer
    /// pending, so a later packet for it is dropped.
    pub(crate) last: bool,
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
    /// Tracks the source IP that each session was established from, with the time the
    /// session was last used so idle entries can be evicted (see `SESSION_TTL`).
    pub session_ips: FxHashMap<H256, SessionSource>,
    /// The background crawl's active iterative lookups. Timer-driven: each
    /// tick sends the next alpha queries.
    pub active_lookups: Vec<(LookupId, IterativeLookup)>,
    /// Running predicate queries. Response-driven, and kept apart from the
    /// crawl so the crawl's timer never advances them.
    pub(crate) active_queries: Vec<NodeQuery>,
    /// Which lookup sent each FINDNODE, keyed like `pending_findnodes`, so a
    /// NODES response reaches the lookup that asked and no other. A FINDNODE
    /// sent outside any lookup, such as an ENR refresh, has no entry. Leaves
    /// with its `pending_findnodes` entry.
    pub(crate) findnode_owners: FxHashMap<(H256, Bytes), LookupId>,
    /// Whether a query tick is scheduled, so a second one is never armed.
    pub(crate) query_tick_armed: bool,
    next_lookup_id: u64,
    /// FINDNODE requests we sent whose answer is not complete yet, keyed by
    /// (peer node id, request id). A NODES response is only accepted if it
    /// matches one of these: without the check, any peer with a session can
    /// push arbitrary ENRs into the peer table by sending an unsolicited
    /// NODES, and we then serve them back from our own FINDNODE responses.
    /// An answer may span several packets sharing one request id, so an entry
    /// stays until the packets its answer is allowed are in (see
    /// `accept_nodes_packet`), or until `PENDING_FINDNODE_TIMEOUT`.
    pub pending_findnodes: FxHashMap<(H256, Bytes), PendingFindNode>,
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
            active_lookups: Vec::new(),
            active_queries: Vec::new(),
            findnode_owners: Default::default(),
            query_tick_armed: false,
            next_lookup_id: 0,
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

    /// A fresh id for a lookup about to start.
    pub(crate) fn next_lookup_id(&mut self) -> LookupId {
        let id = LookupId(self.next_lookup_id);
        self.next_lookup_id = self.next_lookup_id.wrapping_add(1);
        id
    }

    /// Note a FINDNODE sent to `node_id`, so its NODES answer is accepted.
    ///
    /// Every send path calls this, so none can issue a FINDNODE unregistered.
    /// A resend of the same request after a WHOAREYOU restarts its timeout but
    /// keeps the packets already counted against it.
    pub(crate) fn record_findnode_sent(&mut self, node_id: H256, req_id: Bytes, now: Instant) {
        self.pending_findnodes
            .entry((node_id, req_id))
            .and_modify(|pending| pending.sent_at = now)
            .or_insert(PendingFindNode {
                sent_at: now,
                packets_due: None,
            });
    }

    /// Count one NODES packet from `sender` answering our FINDNODE `req_id`,
    /// one of `total`.
    ///
    /// `None` means drop it: it answers no FINDNODE of ours, or arrives after
    /// the answer was complete. The first packet's `total` is the one believed,
    /// as geth does: read as at least one, the packet itself, and at most
    /// [`MAX_NODES_RESPONSES`]. The packet that completes the answer takes the
    /// FINDNODE out of `pending_findnodes` and `findnode_owners`.
    pub(crate) fn accept_nodes_packet(
        &mut self,
        sender: H256,
        req_id: &Bytes,
        total: u64,
    ) -> Option<AcceptedNodes> {
        let key = (sender, req_id.clone());
        let pending = self.pending_findnodes.get_mut(&key)?;
        let due = pending
            .packets_due
            .get_or_insert(total.clamp(1, MAX_NODES_RESPONSES));
        *due -= 1;
        let last = *due == 0;
        let owner = if last {
            self.pending_findnodes.remove(&key);
            self.findnode_owners.remove(&key)
        } else {
            self.findnode_owners.get(&key).copied()
        };
        Some(AcceptedNodes { owner, last })
    }

    /// Remove stale entries from pending caches.
    pub fn cleanup_stale_entries(&mut self) {
        let now = Instant::now();

        let before_messages = self.pending_by_nonce.len();
        self.pending_by_nonce
            .retain(|_nonce, (_node, _message, timestamp)| {
                now.duration_since(*timestamp) < MESSAGE_CACHE_TIMEOUT
            });
        let removed_messages = before_messages - self.pending_by_nonce.len();

        self.pending_findnodes
            .retain(|_key, pending| now.duration_since(pending.sent_at) < PENDING_FINDNODE_TIMEOUT);
        self.findnode_owners
            .retain(|key, _| self.pending_findnodes.contains_key(key));

        let before_challenges = self.pending_challenges.len();
        self.pending_challenges
            .retain(|_src_id, (_challenge_data, timestamp, _raw)| {
                now.duration_since(*timestamp) < MESSAGE_CACHE_TIMEOUT
            });
        let removed_challenges = before_challenges - self.pending_challenges.len();

        let before_sessions = self.session_ips.len();
        self.session_ips
            .retain(|_node_id, source| now.duration_since(source.last_used) < SESSION_TTL);
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

    const PEER: H256 = H256::repeat_byte(0xab);

    /// What [`Discv5State::accept_nodes_packet`] makes of each of `totals`, one
    /// packet each, answering one FINDNODE: `Some(last)` for a packet taken.
    fn answer(state: &mut Discv5State, totals: &[u64]) -> Vec<Option<bool>> {
        let req_id = Bytes::from_static(&[1]);
        totals
            .iter()
            .map(|total| {
                state
                    .accept_nodes_packet(PEER, &req_id, *total)
                    .map(|accepted| accepted.last)
            })
            .collect()
    }

    fn with_findnode_sent() -> Discv5State {
        let mut state = Discv5State::default();
        state.record_findnode_sent(PEER, Bytes::from_static(&[1]), Instant::now());
        state
    }

    #[test]
    fn a_nodes_packet_nobody_asked_for_is_dropped() {
        assert_eq!(answer(&mut Discv5State::default(), &[1]), vec![None]);
    }

    #[test]
    fn an_answer_ends_at_the_first_packets_total() {
        // Later packets cannot stretch the answer by claiming a larger total.
        let mut state = with_findnode_sent();
        assert_eq!(
            answer(&mut state, &[2, 5, 5]),
            vec![Some(false), Some(true), None]
        );
        assert!(state.pending_findnodes.is_empty());
    }

    #[test]
    fn an_answer_is_cut_off_at_geths_packet_limit() {
        // A peer claiming more packets than the limit, so as to keep feeding
        // records into a query, gets only the limit's worth taken.
        let mut state = with_findnode_sent();
        let taken = answer(&mut state, &[200; 7]);
        let mut expected = vec![Some(false); MAX_NODES_RESPONSES as usize - 1];
        expected.extend([Some(true), None, None]);
        assert_eq!(taken, expected);
    }

    #[test]
    fn a_total_of_zero_still_counts_its_own_packet() {
        let mut state = with_findnode_sent();
        assert_eq!(answer(&mut state, &[0, 0]), vec![Some(true), None]);
    }

    #[test]
    fn a_findnode_resent_after_whoareyou_keeps_its_count() {
        let mut state = with_findnode_sent();
        assert_eq!(answer(&mut state, &[2]), vec![Some(false)]);
        state.record_findnode_sent(PEER, Bytes::from_static(&[1]), Instant::now());
        assert_eq!(answer(&mut state, &[2, 2]), vec![Some(true), None]);
    }

    #[test]
    fn the_packet_that_completes_an_answer_releases_its_owner() {
        let mut state = with_findnode_sent();
        let owner = state.next_lookup_id();
        let key = (PEER, Bytes::from_static(&[1]));
        state.findnode_owners.insert(key.clone(), owner);

        let req_id = &key.1;
        let first = state.accept_nodes_packet(PEER, req_id, 2);
        assert_eq!(
            first,
            Some(AcceptedNodes {
                owner: Some(owner),
                last: false
            })
        );
        let second = state.accept_nodes_packet(PEER, req_id, 2);
        assert_eq!(
            second,
            Some(AcceptedNodes {
                owner: Some(owner),
                last: true
            })
        );
        assert!(!state.findnode_owners.contains_key(&key));
    }

    #[tokio::test]
    async fn test_next_nonce_counter() {
        let mut rng = StdRng::seed_from_u64(7);
        let mut state = Discv5State::default();

        let n1 = state.next_nonce(&mut rng);
        let n2 = state.next_nonce(&mut rng);

        assert_eq!(&n1[..4], &[0, 0, 0, 0]);
        assert_eq!(&n2[..4], &[0, 0, 0, 1]);
        assert_ne!(&n1[4..], &n2[4..]);
    }
}
