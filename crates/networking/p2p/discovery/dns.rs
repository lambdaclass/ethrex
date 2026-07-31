//! EIP-1459 DNS-based node discovery.
//!
//! Discovery bootstrapping normally relies on a small hardcoded bootnode list.
//! When those hosts stop answering, a node starting with an empty peer table has
//! no way into the DHT at all and stays at zero peers indefinitely. A DNS node
//! list is the independent bootstrap path the other clients use for exactly that
//! case: a signed, DNS-hosted Merkle tree of ENRs that can be refreshed without
//! shipping a new binary.
//!
//! The tree is described by an `enrtree://<base32-pubkey>@<domain>` URL. Its root
//! lives in a TXT record at `<domain>` and is signed by the tree's key, so a
//! hijacked resolver cannot inject nodes. Interior nodes are TXT records at
//! `<base32-hash>.<domain>`, where the label is
//! `base32(keccak256(child_record_text)[..16])` — verified on every fetch, which
//! makes the whole tree tamper-evident under one signature.
//!
//! Note that a bundled list of discv5 ENRs is *not* an equivalent fallback, even
//! though other clients ship one: theirs hold only consensus-layer records (an
//! `eth2`/`attnets` key, no `eth` key, and a TCP port of 0 or a beacon port), so
//! none of those nodes can ever become an execution-layer peer. A DNS node list
//! is the only bootstrap source that is both independent of the bootnodes and
//! actually made of execution-layer nodes.
//!
//! See <https://eips.ethereum.org/EIPS/eip-1459>.

use std::{collections::HashSet, future::Future, str::FromStr, time::Duration};

use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::decode::RLPDecode;
use rand::seq::SliceRandom;
use secp256k1::{PublicKey, ecdsa::Signature};
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::{
    peer_table::{DiscoveryProtocol, PeerTable, PeerTableServerProtocol as _},
    types::{Node, NodeRecord},
};

/// Prefix of a root record, which also fixes the version we accept.
const ROOT_PREFIX: &str = "enrtree-root:v1";
/// Prefix of an interior (branch) record.
const BRANCH_PREFIX: &str = "enrtree-branch:";
/// Prefix of a leaf record holding a base64url-encoded ENR.
const ENR_PREFIX: &str = "enr:";
/// Prefix of a record (or URL) linking to another tree.
const LINK_PREFIX: &str = "enrtree://";

/// Bytes of the record hash that the DNS label encodes.
const HASH_ABBREV_BYTES: usize = 16;

/// How many nodes to accept from a single sync. Bounds both memory and the
/// number of DNS queries a malicious tree can make us perform.
pub const DEFAULT_MAX_NODES: usize = 256;
/// Hard ceiling on TXT lookups per sync, independent of tree shape. A tree that
/// keeps handing back branches can otherwise walk us forever.
pub const DEFAULT_MAX_REQUESTS: usize = 512;
/// How deep we follow `enrtree://` links into other trees.
const MAX_LINK_DEPTH: usize = 1;

/// Interval between DNS re-syncs once a sync has succeeded. The published trees
/// change on the order of minutes, and this path only needs to keep a
/// *bootstrap* supply of contacts available, so a slow poll is enough.
pub const DNS_SYNC_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// First retry delay after a sync that produced no nodes. Doubles up to
/// [`DNS_SYNC_INTERVAL`]; see [`next_sync_delay`].
pub const DNS_RETRY_MIN_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Error)]
pub enum DnsDiscoveryError {
    #[error("DNS lookup for {name} failed: {source}")]
    Lookup {
        name: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("no TXT record at {0}")]
    NoRecord(String),
    #[error("malformed enrtree URL: {0}")]
    InvalidUrl(String),
    #[error("malformed {kind} record: {reason}")]
    InvalidRecord { kind: &'static str, reason: String },
    #[error("root signature verification failed for {0}")]
    InvalidRootSignature(String),
}

/// Resolves TXT records. Abstracted so the tree walk can be tested without DNS.
pub trait TxtResolver: Send + Sync + 'static {
    /// Returns the TXT record at `name` with all character-strings concatenated.
    fn lookup_txt(
        &self,
        name: String,
    ) -> impl Future<Output = Result<String, DnsDiscoveryError>> + Send;
}

/// A parsed `enrtree://<base32-pubkey>@<domain>` URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnrTreeLink {
    pub public_key: PublicKey,
    pub domain: String,
}

impl FromStr for EnrTreeLink {
    type Err = DnsDiscoveryError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s
            .strip_prefix(LINK_PREFIX)
            .ok_or_else(|| DnsDiscoveryError::InvalidUrl(s.to_string()))?;
        let (key, domain) = rest
            .split_once('@')
            .ok_or_else(|| DnsDiscoveryError::InvalidUrl(s.to_string()))?;
        if domain.is_empty() {
            return Err(DnsDiscoveryError::InvalidUrl(s.to_string()));
        }
        let key_bytes =
            base32_decode(key).ok_or_else(|| DnsDiscoveryError::InvalidUrl(s.to_string()))?;
        let public_key = PublicKey::from_slice(&key_bytes)
            .map_err(|_| DnsDiscoveryError::InvalidUrl(s.to_string()))?;
        Ok(Self {
            public_key,
            domain: domain.to_string(),
        })
    }
}

/// The signed root of a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RootEntry {
    /// Label of the subtree holding ENRs.
    enr_root: String,
    /// Label of the subtree holding links to other trees.
    link_root: String,
    seq: u64,
    signature: Vec<u8>,
}

impl RootEntry {
    fn parse(record: &str) -> Result<Self, DnsDiscoveryError> {
        let invalid = |reason: &str| DnsDiscoveryError::InvalidRecord {
            kind: "enrtree-root",
            reason: reason.to_string(),
        };
        let body = record
            .strip_prefix(ROOT_PREFIX)
            .ok_or_else(|| invalid("unsupported version or prefix"))?;

        let (mut enr_root, mut link_root, mut seq, mut signature) = (None, None, None, None);
        for token in body.split_whitespace() {
            let Some((key, value)) = token.split_once('=') else {
                continue;
            };
            match key {
                "e" => enr_root = Some(value.to_string()),
                "l" => link_root = Some(value.to_string()),
                "seq" => {
                    seq = Some(
                        value
                            .parse::<u64>()
                            .map_err(|_| invalid("seq is not a number"))?,
                    )
                }
                // Signatures are base64url without padding, matching ENR encoding.
                "sig" => signature = Some(ethrex_common::base64::decode(value.as_bytes())),
                _ => {}
            }
        }

        Ok(Self {
            enr_root: enr_root.ok_or_else(|| invalid("missing e="))?,
            link_root: link_root.ok_or_else(|| invalid("missing l="))?,
            seq: seq.ok_or_else(|| invalid("missing seq="))?,
            signature: signature.ok_or_else(|| invalid("missing sig="))?,
        })
    }

    /// The exact string the tree owner signed: the record without its `sig=`
    /// field, with fields in canonical order.
    fn signed_content(&self) -> String {
        format!(
            "{ROOT_PREFIX} e={} l={} seq={}",
            self.enr_root, self.link_root, self.seq
        )
    }

    fn verify_signature(&self, public_key: &PublicKey) -> bool {
        // The signature is 65 bytes (r || s || recovery id); only r || s is verified.
        let Some(compact) = self.signature.get(..64) else {
            return false;
        };
        let Ok(signature) = Signature::from_compact(compact) else {
            return false;
        };
        let digest = keccak_hash(self.signed_content().as_bytes());
        let Ok(message) = secp256k1::Message::from_digest_slice(&digest) else {
            return false;
        };
        secp256k1::SECP256K1
            .verify_ecdsa(&message, &signature, public_key)
            .is_ok()
    }
}

/// A non-root record in the tree.
enum TreeEntry {
    Branch(Vec<String>),
    Enr(Box<NodeRecord>),
    Link(EnrTreeLink),
}

impl TreeEntry {
    fn parse(record: &str) -> Result<Self, DnsDiscoveryError> {
        if let Some(children) = record.strip_prefix(BRANCH_PREFIX) {
            // An empty branch is legal and simply terminates that path.
            let children = children
                .split(',')
                .filter(|hash| !hash.is_empty())
                .map(str::to_string)
                .collect();
            return Ok(Self::Branch(children));
        }
        if let Some(encoded) = record.strip_prefix(ENR_PREFIX) {
            let rlp = ethrex_common::base64::decode(encoded.as_bytes());
            let node_record =
                NodeRecord::decode(&rlp).map_err(|e| DnsDiscoveryError::InvalidRecord {
                    kind: "enr",
                    reason: e.to_string(),
                })?;
            return Ok(Self::Enr(Box::new(node_record)));
        }
        if record.starts_with(LINK_PREFIX) {
            return Ok(Self::Link(record.parse()?));
        }
        Err(DnsDiscoveryError::InvalidRecord {
            kind: "tree entry",
            reason: format!("unrecognized prefix in {:.32}", record),
        })
    }
}

/// Walks EIP-1459 trees and yields the nodes found in them.
pub struct DnsDiscovery<R: TxtResolver> {
    resolver: R,
    links: Vec<EnrTreeLink>,
    max_nodes: usize,
    max_requests: usize,
}

impl<R: TxtResolver> DnsDiscovery<R> {
    pub fn new(resolver: R, links: Vec<EnrTreeLink>) -> Self {
        Self {
            resolver,
            links,
            max_nodes: DEFAULT_MAX_NODES,
            max_requests: DEFAULT_MAX_REQUESTS,
        }
    }

    pub fn with_limits(mut self, max_nodes: usize, max_requests: usize) -> Self {
        self.max_nodes = max_nodes;
        self.max_requests = max_requests;
        self
    }

    /// Resolves every configured tree, returning the union of their nodes.
    ///
    /// A tree that fails to resolve is logged and skipped: one broken or
    /// unreachable tree must not take down the others.
    pub async fn sync(&self) -> Vec<Node> {
        let mut nodes = Vec::new();
        let mut seen_node_ids = HashSet::new();
        let mut requests = 0usize;

        // (link, depth); depth counts how many `enrtree://` hops we followed.
        let mut pending: Vec<(EnrTreeLink, usize)> =
            self.links.iter().cloned().map(|l| (l, 0)).collect();
        let mut visited_domains = HashSet::new();

        while let Some((link, depth)) = pending.pop() {
            if nodes.len() >= self.max_nodes || requests >= self.max_requests {
                break;
            }
            if !visited_domains.insert(link.domain.clone()) {
                continue;
            }
            match self
                .sync_tree(
                    &link,
                    depth,
                    &mut nodes,
                    &mut seen_node_ids,
                    &mut requests,
                    &mut pending,
                )
                .await
            {
                Ok(()) => {}
                Err(e) => warn!(domain = %link.domain, "DNS discovery tree sync failed: {e}"),
            }
        }

        nodes
    }

    async fn sync_tree(
        &self,
        link: &EnrTreeLink,
        depth: usize,
        nodes: &mut Vec<Node>,
        seen_node_ids: &mut HashSet<ethrex_common::H256>,
        requests: &mut usize,
        pending: &mut Vec<(EnrTreeLink, usize)>,
    ) -> Result<(), DnsDiscoveryError> {
        let root_record = self.lookup(&link.domain, requests).await?;
        let root = RootEntry::parse(&root_record)?;
        if !root.verify_signature(&link.public_key) {
            return Err(DnsDiscoveryError::InvalidRootSignature(link.domain.clone()));
        }
        debug!(
            domain = %link.domain,
            seq = root.seq,
            "DNS discovery: verified tree root"
        );

        // Only descend into the link subtree while we still have hops left.
        let mut stack = vec![root.enr_root.clone()];
        if depth < MAX_LINK_DEPTH {
            stack.push(root.link_root.clone());
        }
        let mut visited_labels = HashSet::new();

        while let Some(label) = stack.pop() {
            if nodes.len() >= self.max_nodes || *requests >= self.max_requests {
                break;
            }
            if label.is_empty() || !visited_labels.insert(label.clone()) {
                continue;
            }

            let name = format!("{label}.{}", link.domain);
            let record = match self.lookup(&name, requests).await {
                Ok(record) => record,
                // A single missing or broken subtree shouldn't abort the walk;
                // other branches may still hold usable nodes.
                Err(e) => {
                    debug!("DNS discovery: skipping {name}: {e}");
                    continue;
                }
            };

            if !label_matches_record(&label, &record) {
                warn!(%name, "DNS discovery: record hash does not match its label, skipping");
                continue;
            }

            match TreeEntry::parse(&record) {
                Ok(TreeEntry::Branch(mut children)) => {
                    // Randomize so restarts and different nodes don't all walk
                    // the tree in the same order and converge on one subset.
                    children.shuffle(&mut rand::thread_rng());
                    stack.extend(children);
                }
                Ok(TreeEntry::Enr(record)) => {
                    // The ENR carries its own signature; reject anything the
                    // tree owner (or a tampered subtree) got wrong.
                    if !record.verify_signature() {
                        warn!(%name, "DNS discovery: ENR signature invalid, skipping");
                        continue;
                    }
                    match Node::from_enr(&record) {
                        Ok(node) => {
                            if seen_node_ids.insert(node.node_id()) {
                                nodes.push(node);
                            }
                        }
                        Err(e) => debug!(%name, "DNS discovery: unusable ENR: {e}"),
                    }
                }
                Ok(TreeEntry::Link(link)) => pending.push((link, depth + 1)),
                Err(e) => debug!(%name, "DNS discovery: {e}"),
            }
        }

        Ok(())
    }

    async fn lookup(&self, name: &str, requests: &mut usize) -> Result<String, DnsDiscoveryError> {
        *requests += 1;
        self.resolver.lookup_txt(name.to_string()).await
    }
}

/// Checks that a record is the one its DNS label commits to, i.e. that the label
/// is `base32(keccak256(record)[..HASH_ABBREV_BYTES])`.
fn label_matches_record(label: &str, record: &str) -> bool {
    let digest = keccak_hash(record.as_bytes());
    let Some(abbrev) = digest.get(..HASH_ABBREV_BYTES) else {
        return false;
    };
    base32_encode(abbrev).eq_ignore_ascii_case(label)
}

/// Decodes unpadded RFC 4648 base32 (the alphabet EIP-1459 uses for labels and
/// tree public keys). Returns `None` on any character outside the alphabet.
fn base32_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 5 / 8);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for byte in input.bytes() {
        if byte == b'=' {
            break;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a',
            b'2'..=b'7' => byte - b'2' + 26,
            _ => return None,
        } as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Encodes bytes as unpadded RFC 4648 base32.
fn base32_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity(input.len().saturating_mul(8).div_ceil(5));
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in input {
        buffer = (buffer << 8) | byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
            buffer &= (1 << bits) - 1;
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

/// TXT resolver backed by the platform's configured DNS servers.
pub struct SystemResolver(hickory_resolver::TokioResolver);

impl SystemResolver {
    pub fn new() -> Result<Self, DnsDiscoveryError> {
        let resolver = hickory_resolver::Resolver::builder_tokio()
            .map_err(|e| DnsDiscoveryError::Lookup {
                name: "<resolver init>".to_string(),
                source: Box::new(e),
            })?
            .build()
            .map_err(|e| DnsDiscoveryError::Lookup {
                name: "<resolver init>".to_string(),
                source: Box::new(e),
            })?;
        Ok(Self(resolver))
    }
}

impl TxtResolver for SystemResolver {
    async fn lookup_txt(&self, name: String) -> Result<String, DnsDiscoveryError> {
        use hickory_resolver::proto::rr::{RData, RecordType};

        let lookup = self
            .0
            .lookup(name.clone(), RecordType::TXT)
            .await
            .map_err(|e| DnsDiscoveryError::Lookup {
                name: name.clone(),
                source: Box::new(e),
            })?;

        // A TXT record longer than 255 bytes is split into several
        // character-strings that have to be concatenated back together; root and
        // ENR records routinely exceed that.
        let record = lookup
            .answers()
            .iter()
            .find_map(|record| match &record.data {
                RData::TXT(txt) => Some(
                    txt.txt_data
                        .iter()
                        .flat_map(|chunk| chunk.iter().copied())
                        .collect::<Vec<u8>>(),
                ),
                _ => None,
            })
            .ok_or_else(|| DnsDiscoveryError::NoRecord(name.clone()))?;

        String::from_utf8(record).map_err(|e| DnsDiscoveryError::Lookup {
            name,
            source: Box::new(e),
        })
    }
}

/// Delay before the next sync, given how many syncs in a row produced nothing.
///
/// A sync that yields no nodes retries quickly and backs off toward the
/// steady-state interval. This matters because DNS is not merely a supplement in
/// practice: when the bootnodes are unreachable it is the *only* way in, so
/// waiting a full interval after a transient resolver failure would leave the
/// node peerless for that whole time.
fn next_sync_delay(consecutive_empty_syncs: u32) -> Duration {
    let Some(attempt) = consecutive_empty_syncs.checked_sub(1) else {
        return DNS_SYNC_INTERVAL;
    };
    // Shift is clamped well below u32's width, so the doubling cannot overflow.
    DNS_RETRY_MIN_INTERVAL
        .saturating_mul(1u32 << attempt.min(16))
        .min(DNS_SYNC_INTERVAL)
}

/// Periodically syncs the configured DNS node lists into the peer table.
///
/// Runs alongside discv4/discv5 rather than gating startup on it, so a slow or
/// unreachable resolver never delays the node coming up. Contacts are added
/// exactly the way bootnodes are, so they get pinged, validated and dialed
/// through the existing paths.
pub async fn run_dns_discovery(
    peer_table: PeerTable,
    links: Vec<EnrTreeLink>,
    protocols: Vec<DiscoveryProtocol>,
) {
    let resolver = match SystemResolver::new() {
        Ok(resolver) => resolver,
        Err(e) => {
            warn!("DNS discovery disabled, could not build resolver: {e}");
            return;
        }
    };
    let domains = links
        .iter()
        .map(|link| link.domain.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    info!(trees = %domains, "Starting DNS discovery");

    let discovery = DnsDiscovery::new(resolver, links);
    let mut consecutive_empty_syncs = 0u32;
    loop {
        let nodes = discovery.sync().await;
        if nodes.is_empty() {
            consecutive_empty_syncs = consecutive_empty_syncs.saturating_add(1);
            warn!(
                attempt = consecutive_empty_syncs,
                "DNS discovery sync returned no nodes"
            );
        } else {
            consecutive_empty_syncs = 0;
            info!(count = nodes.len(), "DNS discovery: adding contacts");
            for protocol in &protocols {
                if let Err(e) = peer_table.new_contacts(nodes.clone(), *protocol) {
                    debug!("DNS discovery could not add contacts: {e}");
                }
            }
        }
        let delay = next_sync_delay(consecutive_empty_syncs);
        debug!(?delay, "DNS discovery: sleeping until next sync");
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// The tree key EF DevOps publishes every `*.ethdisco.net` list under.
    const ETHDISCO_KEY: &str = "AKA3AM6LPBYEUDMVNU3BSVQJ5AD45Y7YPOHJLEF6W26QOE4VTUDPE";

    struct MockResolver(HashMap<String, String>);

    impl TxtResolver for MockResolver {
        async fn lookup_txt(&self, name: String) -> Result<String, DnsDiscoveryError> {
            self.0
                .get(&name)
                .cloned()
                .ok_or(DnsDiscoveryError::NoRecord(name))
        }
    }

    /// Builds a tree whose labels are computed the same way a real publisher
    /// would, so the walk exercises label verification for real.
    fn tree(domain: &str, records: Vec<String>) -> (HashMap<String, String>, Vec<String>) {
        let mut dns = HashMap::new();
        let mut labels = Vec::new();
        for record in records {
            let digest = keccak_hash(record.as_bytes());
            let label = base32_encode(&digest[..HASH_ABBREV_BYTES]);
            dns.insert(format!("{label}.{domain}"), record);
            labels.push(label);
        }
        (dns, labels)
    }

    #[test]
    fn retries_quickly_after_an_empty_sync_then_backs_off() {
        // A successful sync waits the full steady-state interval.
        assert_eq!(next_sync_delay(0), DNS_SYNC_INTERVAL);
        // The first empty sync must retry soon, not an interval later: with the
        // bootnodes unreachable this is the only path to a peer.
        assert_eq!(next_sync_delay(1), DNS_RETRY_MIN_INTERVAL);
        assert_eq!(next_sync_delay(2), DNS_RETRY_MIN_INTERVAL * 2);
        assert_eq!(next_sync_delay(3), DNS_RETRY_MIN_INTERVAL * 4);
        // Backoff is monotonic and never exceeds the steady-state interval.
        let mut previous = Duration::ZERO;
        for attempt in 0..64 {
            let delay = next_sync_delay(attempt + 1);
            assert!(delay <= DNS_SYNC_INTERVAL, "attempt {attempt} overshot");
            assert!(delay >= previous, "attempt {attempt} went backwards");
            previous = delay;
        }
        // Deep into the backoff it has saturated rather than overflowed.
        assert_eq!(next_sync_delay(u32::MAX), DNS_SYNC_INTERVAL);
    }

    #[test]
    fn base32_roundtrip() {
        for len in 1..=32 {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
            let encoded = base32_encode(&bytes);
            assert_eq!(base32_decode(&encoded).as_deref(), Some(&bytes[..]));
        }
    }

    #[test]
    fn base32_label_length_matches_eip1459() {
        // A 16-byte abbreviated hash must encode to the 26-char labels used in DNS.
        assert_eq!(base32_encode(&[0xab; HASH_ABBREV_BYTES]).len(), 26);
    }

    #[test]
    fn base32_decode_rejects_invalid_characters() {
        assert!(base32_decode("ABC1").is_none()); // '1' is not in the alphabet
        assert!(base32_decode("A B").is_none());
    }

    #[test]
    fn parses_enrtree_link() {
        let link: EnrTreeLink = format!("enrtree://{ETHDISCO_KEY}@all.sepolia.ethdisco.net")
            .parse()
            .expect("valid link");
        assert_eq!(link.domain, "all.sepolia.ethdisco.net");
    }

    #[test]
    fn rejects_malformed_links() {
        for bad in [
            "enrtree://nokey",
            "enrtree://@all.sepolia.ethdisco.net",
            "https://all.sepolia.ethdisco.net",
            &format!("enrtree://{ETHDISCO_KEY}@"),
        ] {
            assert!(
                bad.parse::<EnrTreeLink>().is_err(),
                "should have rejected {bad}"
            );
        }
    }

    /// Live root of `all.sepolia.ethdisco.net`, captured 2026-07-31. Pins the
    /// signed preimage: the signature covers the record *without* its `sig=`
    /// field, so a change in how `signed_content` is built breaks this test.
    const SEPOLIA_ROOT: &str = "enrtree-root:v1 e=HB5O4I23N7IDSXPDD2DVKYC4TY l=FDXN3SN67NA5DKA4J2GOK7BVQI seq=5211 sig=S3ReY3w2hrJRQ2pM2jE0ZguIYtYq8r_zrAwA7uy3CF9DhUgg-7EsDrQ3llPPgEsBLUSA1np4mxEYR5FD0XxEtQE";

    #[test]
    fn verifies_real_root_signature() {
        let link: EnrTreeLink = format!("enrtree://{ETHDISCO_KEY}@all.sepolia.ethdisco.net")
            .parse()
            .expect("valid link");
        let root = RootEntry::parse(SEPOLIA_ROOT).expect("valid root");
        assert_eq!(root.seq, 5211);
        assert_eq!(root.enr_root, "HB5O4I23N7IDSXPDD2DVKYC4TY");
        assert_eq!(
            root.signed_content(),
            "enrtree-root:v1 e=HB5O4I23N7IDSXPDD2DVKYC4TY l=FDXN3SN67NA5DKA4J2GOK7BVQI seq=5211"
        );
        assert!(root.verify_signature(&link.public_key));
    }

    #[test]
    fn rejects_root_signed_by_another_key() {
        // Same record, different tree key: must not verify.
        let other_key = PublicKey::from_secret_key(
            secp256k1::SECP256K1,
            &secp256k1::SecretKey::from_slice(&[7u8; 32]).expect("valid key"),
        );
        let root = RootEntry::parse(SEPOLIA_ROOT).expect("valid root");
        assert!(!root.verify_signature(&other_key));
    }

    #[test]
    fn rejects_tampered_root() {
        // Bump seq but keep the signature: verification must fail.
        let tampered = SEPOLIA_ROOT.replace("seq=5211", "seq=5212");
        let link: EnrTreeLink = format!("enrtree://{ETHDISCO_KEY}@all.sepolia.ethdisco.net")
            .parse()
            .expect("valid link");
        let root = RootEntry::parse(&tampered).expect("parses");
        assert!(!root.verify_signature(&link.public_key));
    }

    #[test]
    fn rejects_root_with_missing_fields() {
        assert!(RootEntry::parse("enrtree-root:v1 e=AAA l=BBB sig=CCC").is_err());
        assert!(RootEntry::parse("enrtree-root:v2 e=AAA l=BBB seq=1 sig=CCC").is_err());
    }

    #[test]
    fn label_verification_detects_substitution() {
        let record = "enrtree-branch:";
        let digest = keccak_hash(record.as_bytes());
        let label = base32_encode(&digest[..HASH_ABBREV_BYTES]);
        assert!(label_matches_record(&label, record));
        assert!(!label_matches_record(&label, "enrtree-branch:AAAA"));
    }

    // A locally-signed tree lets the walk be tested end to end without DNS.
    fn signed_root(secret: &secp256k1::SecretKey, enr_root: &str, link_root: &str) -> String {
        let content = format!("{ROOT_PREFIX} e={enr_root} l={link_root} seq=1");
        let digest = keccak_hash(content.as_bytes());
        let message = secp256k1::Message::from_digest_slice(&digest).expect("32 bytes");
        let (recovery_id, sig) = secp256k1::SECP256K1
            .sign_ecdsa_recoverable(&message, secret)
            .serialize_compact();
        let mut full = sig.to_vec();
        full.push(i32::from(recovery_id) as u8);
        let encoded = String::from_utf8(ethrex_common::base64::encode(&full)).expect("ascii");
        format!("{content} sig={}", encoded.trim_end_matches('='))
    }

    /// Real sepolia ENRs pulled from `all.sepolia.ethdisco.net` on 2026-07-31.
    /// Using genuine records means the test also covers ENR self-signature
    /// verification, not just the tree walk.
    const ENR_A: &str = "enr:-KO4QKgAqcMkgAQnYWG9dYUuRdH6D-2uSGw98ABfq_fAou49RmH-527_WvKj0VIgXpt_wa19KVWkUAE50mVVdJPqoNGGAZXSmWFXg2V0aMfGhCaJVraAgmlkgnY0gmlwhMApTdiJc2VjcDI1NmsxoQPIXgr6uOZRUK3f9hySAd4hlqBhCzXDq8bB5hARPbaBIIRzbmFwwIN0Y3CCdl-DdWRwgnZf";
    const ENR_B: &str = "enr:-J24QFSMwERNztHqJdAF86e2y_j6LqiB4Ma0yMFahPuolCopJE9san-5fFsjvtos3R3QCPo-q8wLP-fFO3SVVzSmAKKGAZ17xbL4g2V0aMfGhCaJVraAgmlkgnY0gmlwhFwFA2-Jc2VjcDI1NmsxoQJgJXtzDmONKBJiQEncZR_acBwzZ0m3ae1HqmkYFbHkToN0Y3CCdl-DdWRwgnZf";

    #[test]
    fn parses_and_verifies_real_enr_leaves() {
        for enr in [ENR_A, ENR_B] {
            match TreeEntry::parse(enr) {
                Ok(TreeEntry::Enr(record)) => {
                    assert!(record.verify_signature(), "ENR signature should verify");
                    assert!(Node::from_enr(&record).is_ok(), "ENR should yield a node");
                }
                _ => panic!("expected an ENR leaf"),
            }
        }
    }

    #[tokio::test]
    async fn walks_tree_and_collects_nodes() {
        let secret = secp256k1::SecretKey::from_slice(&[3u8; 32]).expect("valid key");
        let public_key = PublicKey::from_secret_key(secp256k1::SECP256K1, &secret);
        let domain = "nodes.test";

        // Leaves first, then a branch pointing at them, then the root.
        let (mut dns, leaf_labels) = tree(domain, vec![ENR_A.to_string(), ENR_B.to_string()]);
        let branch = format!("{BRANCH_PREFIX}{}", leaf_labels.join(","));
        let (branch_dns, branch_labels) = tree(domain, vec![branch]);
        dns.extend(branch_dns);
        let empty_links = BRANCH_PREFIX.to_string();
        let (links_dns, link_labels) = tree(domain, vec![empty_links]);
        dns.extend(links_dns);
        dns.insert(
            domain.to_string(),
            signed_root(&secret, &branch_labels[0], &link_labels[0]),
        );

        let discovery = DnsDiscovery::new(
            MockResolver(dns),
            vec![EnrTreeLink {
                public_key,
                domain: domain.to_string(),
            }],
        );
        let nodes = discovery.sync().await;
        assert_eq!(nodes.len(), 2, "should have found both ENR leaves");
    }

    #[tokio::test]
    async fn rejects_tree_whose_root_is_signed_by_another_key() {
        let signing_secret = secp256k1::SecretKey::from_slice(&[3u8; 32]).expect("valid key");
        let expected_key = PublicKey::from_secret_key(
            secp256k1::SECP256K1,
            &secp256k1::SecretKey::from_slice(&[4u8; 32]).expect("valid key"),
        );
        let domain = "nodes.test";

        let (mut dns, leaf_labels) = tree(domain, vec![ENR_A.to_string()]);
        let branch = format!("{BRANCH_PREFIX}{}", leaf_labels.join(","));
        let (branch_dns, branch_labels) = tree(domain, vec![branch]);
        dns.extend(branch_dns);
        dns.insert(
            domain.to_string(),
            signed_root(&signing_secret, &branch_labels[0], ""),
        );

        let discovery = DnsDiscovery::new(
            MockResolver(dns),
            vec![EnrTreeLink {
                public_key: expected_key,
                domain: domain.to_string(),
            }],
        );
        assert!(
            discovery.sync().await.is_empty(),
            "a root signed by the wrong key must yield no nodes"
        );
    }

    #[tokio::test]
    async fn stops_at_request_budget_on_a_cyclic_tree() {
        // A branch that lists its own label would loop forever without the
        // visited set; the budget is the backstop if a tree fans out instead.
        let secret = secp256k1::SecretKey::from_slice(&[3u8; 32]).expect("valid key");
        let public_key = PublicKey::from_secret_key(secp256k1::SECP256K1, &secret);
        let domain = "loop.test";

        let mut dns = HashMap::new();
        // Self-referential branch: label(record) is inside record's own child list.
        // Build it by fixpoint: a branch listing a label we then make resolve to itself.
        let placeholder = BRANCH_PREFIX.to_string();
        let digest = keccak_hash(placeholder.as_bytes());
        let self_label = base32_encode(&digest[..HASH_ABBREV_BYTES]);
        dns.insert(format!("{self_label}.{domain}"), placeholder);
        dns.insert(
            domain.to_string(),
            signed_root(&secret, &self_label, &self_label),
        );

        let discovery = DnsDiscovery::new(
            MockResolver(dns),
            vec![EnrTreeLink {
                public_key,
                domain: domain.to_string(),
            }],
        )
        .with_limits(8, 8);
        assert!(discovery.sync().await.is_empty());
    }

    #[tokio::test]
    async fn one_unreachable_tree_does_not_block_the_others() {
        let secret = secp256k1::SecretKey::from_slice(&[3u8; 32]).expect("valid key");
        let public_key = PublicKey::from_secret_key(secp256k1::SECP256K1, &secret);
        let domain = "nodes.test";

        let (mut dns, leaf_labels) = tree(domain, vec![ENR_A.to_string()]);
        let branch = format!("{BRANCH_PREFIX}{}", leaf_labels.join(","));
        let (branch_dns, branch_labels) = tree(domain, vec![branch]);
        dns.extend(branch_dns);
        dns.insert(
            domain.to_string(),
            signed_root(&secret, &branch_labels[0], ""),
        );

        let discovery = DnsDiscovery::new(
            MockResolver(dns),
            vec![
                EnrTreeLink {
                    public_key,
                    domain: "does-not-resolve.test".to_string(),
                },
                EnrTreeLink {
                    public_key,
                    domain: domain.to_string(),
                },
            ],
        );
        assert_eq!(discovery.sync().await.len(), 1);
    }
}
