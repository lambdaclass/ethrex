/// Minimal standalone Merkle Patricia Trie hash computation.
///
/// This module provides `compute_hash_from_unsorted_iter`, which is the only
/// MPT operation needed by `ethrex-common` types (block roots, receipts root,
/// withdrawals root, account storage root, genesis state root).
///
/// The implementation is self-contained: it does not depend on `ethrex-trie`.
/// It uses a recursive encoding algorithm over sorted nibble-key/value pairs
/// and produces the same Ethereum-compatible root hashes as the full MPT.
use ethereum_types::H256;
use ethrex_crypto::{Crypto, NativeCrypto};
use ethrex_rlp::{
    constants::RLP_NULL,
    encode::{RLPEncode, encode_length},
    structs::Encoder,
};

/// RLP-encoded path (keccak hash of key, or RLP index)
pub type PathRLP = Vec<u8>;
/// RLP-encoded value
pub type ValueRLP = Vec<u8>;

/// Builds an in-memory MPT from the given key-value iterator and returns its root hash.
///
/// This is identical in output to `ethrex_trie::Trie::compute_hash_from_unsorted_iter`.
/// Keys and values must already be RLP-encoded.
pub fn compute_hash_from_unsorted_iter(
    iter: impl Iterator<Item = (PathRLP, ValueRLP)>,
    crypto: &dyn Crypto,
) -> H256 {
    // Convert byte keys to nibble sequences and sort.
    let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = iter.map(|(k, v)| (to_nibbles(&k), v)).collect();
    pairs.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

    if pairs.is_empty() {
        return H256(crypto.keccak256(&[RLP_NULL]));
    }

    let encoded = encode_node(&pairs, 0, crypto);
    // The root is always hashed (even when encoding is < 32 bytes).
    H256(crypto.keccak256(&encoded))
}

/// Convenience wrapper that uses `NativeCrypto`.
pub fn compute_hash_from_unsorted_iter_native(
    iter: impl Iterator<Item = (PathRLP, ValueRLP)>,
) -> H256 {
    compute_hash_from_unsorted_iter(iter, &NativeCrypto)
}

/// Expands each byte in `key` into two nibbles (high nibble first).
fn to_nibbles(key: &[u8]) -> Vec<u8> {
    let mut nibbles = Vec::with_capacity(key.len() * 2);
    for byte in key {
        nibbles.push(byte >> 4);
        nibbles.push(byte & 0x0f);
    }
    nibbles
}

/// Compact-encodes a nibble path with an `is_leaf` flag (Ethereum yellow paper §C).
fn compact_encode(nibbles: &[u8], is_leaf: bool) -> Vec<u8> {
    let flag = if is_leaf { 2u8 } else { 0u8 };
    let odd = nibbles.len() % 2 == 1;
    let mut out = Vec::with_capacity(1 + nibbles.len() / 2);
    if odd {
        out.push((flag | 1) << 4 | nibbles[0]);
        for pair in nibbles[1..].chunks(2) {
            out.push(pair[0] << 4 | pair[1]);
        }
    } else {
        out.push(flag << 4);
        for pair in nibbles.chunks(2) {
            out.push(pair[0] << 4 | pair[1]);
        }
    }
    out
}

/// Recursively encode a sub-trie for `pairs[..]`, where all keys share the first
/// `prefix_len` nibbles (already consumed by the ancestor node).
///
/// Returns the RLP encoding of the node. If encoding is ≥ 32 bytes the caller
/// stores a keccak reference; if < 32 bytes it is inlined per the MPT spec.
fn encode_node(pairs: &[(Vec<u8>, Vec<u8>)], prefix_len: usize, crypto: &dyn Crypto) -> Vec<u8> {
    debug_assert!(!pairs.is_empty());

    if pairs.len() == 1 {
        // Leaf node: encode remaining nibbles + value.
        let remaining = &pairs[0].0[prefix_len..];
        let encoded_path = compact_encode(remaining, true);
        return rlp_pair(&encoded_path, &pairs[0].1);
    }

    // Find how much additional prefix is shared among ALL keys in this slice.
    let shared = shared_prefix_len(pairs, prefix_len);

    if shared > prefix_len {
        // Extension node: consume the shared prefix and recurse.
        let prefix_nibbles = &pairs[0].0[prefix_len..shared];
        let encoded_path = compact_encode(prefix_nibbles, false);
        let child_enc = encode_node(pairs, shared, crypto);
        let child_ref = node_ref(&child_enc, crypto);
        return rlp_pair(&encoded_path, &child_ref);
    }

    // Branch node: split pairs by the next nibble (at position prefix_len).
    let mut branch_children: [Option<Vec<u8>>; 16] = Default::default();
    let mut branch_value: Vec<u8> = Vec::new();

    let mut i = 0;
    while i < pairs.len() {
        let key = &pairs[i].0;
        if key.len() == prefix_len {
            // This key terminates at this branch — it becomes the branch value.
            branch_value = pairs[i].1.clone();
            i += 1;
            continue;
        }
        let nibble = key[prefix_len] as usize;
        // Collect all consecutive pairs sharing this nibble at prefix_len.
        let start = i;
        while i < pairs.len()
            && pairs[i].0.len() > prefix_len
            && pairs[i].0[prefix_len] == nibble as u8
        {
            i += 1;
        }
        let sub_enc = encode_node(&pairs[start..i], prefix_len + 1, crypto);
        branch_children[nibble] = Some(sub_enc);
    }

    encode_branch(&branch_children, &branch_value, crypto)
}

/// Returns the length of the common prefix among all `pairs` starting at position `from`.
/// Since pairs are sorted, the common prefix of first and last covers all elements.
fn shared_prefix_len(pairs: &[(Vec<u8>, Vec<u8>)], from: usize) -> usize {
    let first = &pairs[0].0;
    let last = &pairs[pairs.len() - 1].0;
    let max_len = first.len().min(last.len());
    let mut len = from;
    while len < max_len && first[len] == last[len] {
        len += 1;
    }
    len
}

/// Returns the reference encoding for a child node in a branch or extension:
/// - encoding < 32 bytes → inline (raw bytes of the child's RLP)
/// - encoding ≥ 32 bytes → 33-byte RLP byte string holding keccak256(encoding)
fn node_ref(encoded: &[u8], crypto: &dyn Crypto) -> Vec<u8> {
    if encoded.len() < 32 {
        encoded.to_vec()
    } else {
        let hash = crypto.keccak256(encoded);
        let mut out = Vec::with_capacity(33);
        // Encode the 32-byte hash as an RLP byte string (0xa0 prefix + 32 bytes).
        hash.as_ref().encode(&mut out);
        out
    }
}

/// RLP-encode a two-item list `[path, value]` — used for leaf and extension nodes.
fn rlp_pair(path: &[u8], value: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    Encoder::new(&mut out)
        .encode_bytes(path)
        .encode_bytes(value)
        .finish();
    out
}

/// Returns the byte length of a node reference as it appears inside a branch node.
fn node_ref_len(encoded: &[u8]) -> usize {
    if encoded.len() < 32 {
        encoded.len()
    } else {
        33 // 1-byte RLP prefix (0xa0) + 32 bytes hash
    }
}

/// RLP-encode a branch node: 16 child references followed by an optional value.
fn encode_branch(children: &[Option<Vec<u8>>; 16], value: &[u8], crypto: &dyn Crypto) -> Vec<u8> {
    // Compute the payload length for the RLP list header.
    let mut payload_len = <[u8] as RLPEncode>::length(value);
    for child in children.iter() {
        payload_len += match child {
            None => 1, // 0x80 = RLP_NULL
            Some(enc) => node_ref_len(enc),
        };
    }

    let mut out = Vec::with_capacity(payload_len + 3);
    encode_length(payload_len, &mut out);

    for child in children.iter() {
        match child {
            None => out.push(RLP_NULL),
            Some(enc) => out.extend_from_slice(&node_ref(enc, crypto)),
        }
    }
    <[u8] as RLPEncode>::encode(value, &mut out);
    out
}
