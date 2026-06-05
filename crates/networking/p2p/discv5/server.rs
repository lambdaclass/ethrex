use crate::discovery::lookup::IterativeLookup;
use crate::discv5::messages::Message;
use crate::{
    discv5::messages::Packet,
    types::{Node, NodeRecord},
};
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
    /// Currently active iterative lookups.
    pub active_lookups: Vec<IterativeLookup>,
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

    /// Remove stale entries from pending caches.
    pub fn cleanup_stale_entries(&mut self) {
        let now = Instant::now();

        let before_messages = self.pending_by_nonce.len();
        self.pending_by_nonce
            .retain(|_nonce, (_node, _message, timestamp)| {
                now.duration_since(*timestamp) < MESSAGE_CACHE_TIMEOUT
            });
        let removed_messages = before_messages - self.pending_by_nonce.len();

        let before_challenges = self.pending_challenges.len();
        self.pending_challenges
            .retain(|_src_id, (_challenge_data, timestamp, _raw)| {
                now.duration_since(*timestamp) < MESSAGE_CACHE_TIMEOUT
            });
        let removed_challenges = before_challenges - self.pending_challenges.len();

        let total_removed = removed_messages + removed_challenges;
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
