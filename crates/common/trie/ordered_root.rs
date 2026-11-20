//! Root-only builder for dense RLP-indexed lists. No mutable trie is materialized.
//!
//! Integer keys are prefix-free RLP strings. Lexicographic order is 1..=127,
//! then 0 (encoded as 0x80), then 128 onward, omitting indices outside the list.
//! Their maximum nibble length is 18, bounding recursion independently of list
//! length. Each value is encoded once; parent frames retain only child hashes.
use crate::{EMPTY_TRIE_HASH, NodeHash};
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use ethereum_types::H256;
use ethrex_crypto::Crypto;

#[derive(Clone, Copy)]
struct Key {
    nibbles: [u8; 18],
    len: usize,
    index: usize,
}
impl Key {
    fn at(position: usize, count: usize) -> Self {
        let before_zero = count.saturating_sub(1).min(127);
        let index = if position < before_zero {
            position + 1
        } else if position == before_zero {
            0
        } else {
            position
        };
        let mut encoded = [0u8; 9];
        let len = if index == 0 {
            encoded[0] = 0x80;
            1
        } else if index < 128 {
            encoded[0] = index as u8;
            1
        } else {
            let bytes = (index as u64).to_be_bytes();
            let skip = bytes.iter().position(|b| *b != 0).unwrap();
            let n = 8 - skip;
            encoded[0] = 0x80 + n as u8;
            encoded[1..1 + n].copy_from_slice(&bytes[skip..]);
            n + 1
        };
        let mut nibbles = [0; 18];
        for (i, byte) in encoded[..len].iter().enumerate() {
            nibbles[2 * i] = byte >> 4;
            nibbles[2 * i + 1] = byte & 15;
        }
        Self {
            nibbles,
            len: len * 2,
            index,
        }
    }
}
fn prefix(out: &mut Vec<u8>, len: usize, short: u8, long: u8) {
    if len < 56 {
        out.push(short + len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let skip = bytes.iter().position(|b| *b != 0).unwrap();
        out.push(long + (bytes.len() - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
}
fn string(out: &mut Vec<u8>, bytes: &[u8]) {
    if bytes.len() == 1 && bytes[0] < 128 {
        out.push(bytes[0]);
    } else {
        prefix(out, bytes.len(), 0x80, 0xb7);
        out.extend_from_slice(bytes);
    }
}
fn reference(out: &mut Vec<u8>, hash: NodeHash) {
    match hash {
        NodeHash::Inline((b, n)) => out.extend_from_slice(&b[..n as usize]),
        NodeHash::Hashed(h) => {
            out.push(0xa0);
            out.extend_from_slice(h.as_bytes());
        }
    }
}
fn compact(out: &mut Vec<u8>, path: &[u8], leaf: bool) {
    let mut bytes = [0u8; 10];
    let odd = path.len() % 2;
    bytes[0] = (if leaf { 2 } else { 0 } + odd as u8) << 4;
    if odd != 0 {
        bytes[0] |= path[0];
    }
    for (i, pair) in path[odd..].chunks_exact(2).enumerate() {
        bytes[i + 1] = pair[0] * 16 + pair[1];
    }
    string(out, &bytes[..1 + (path.len() - odd) / 2]);
}
fn finish(buf: &mut [u8], crypto: &dyn Crypto) -> NodeHash {
    // Nine bytes reserved for the largest possible RLP list prefix. Move only
    // the prefix, not the payload, by hashing the occupied suffix.
    let payload = buf.len() - 9;
    let start = if payload < 56 {
        buf[8] = 0xc0 + payload as u8;
        8
    } else {
        let bytes = payload.to_be_bytes();
        let skip = bytes.iter().position(|b| *b != 0).unwrap();
        let n = bytes.len() - skip;
        let start = 8 - n;
        buf[start] = 0xf7 + n as u8;
        buf[start + 1..9].copy_from_slice(&bytes[skip..]);
        start
    };
    NodeHash::from_encoded(&buf[start..], crypto)
}
fn subtree<F: FnMut(usize) -> Vec<u8>>(
    start: usize,
    end: usize,
    depth: usize,
    count: usize,
    value: &mut F,
    buf: &mut Vec<u8>,
    crypto: &dyn Crypto,
) -> NodeHash {
    let first = Key::at(start, count);
    if end == start + 1 {
        let bytes = value(first.index);
        buf.clear();
        buf.resize(9, 0);
        compact(buf, &first.nibbles[depth..first.len], true);
        string(buf, &bytes);
        return finish(buf, crypto);
    }
    let last = Key::at(end - 1, count);
    let mut common = depth;
    while common < first.len.min(last.len) && first.nibbles[common] == last.nibbles[common] {
        common += 1;
    }
    if common > depth {
        let child = subtree(start, end, common, count, value, buf, crypto);
        buf.clear();
        buf.resize(9, 0);
        compact(buf, &first.nibbles[depth..common], false);
        reference(buf, child);
        return finish(buf, crypto);
    }
    let mut children = [NodeHash::Inline(([0; 31], 0)); 16];
    let mut pos = start;
    while pos < end {
        let nib = Key::at(pos, count).nibbles[depth] as usize;
        let begin = pos;
        pos += 1;
        while pos < end && Key::at(pos, count).nibbles[depth] as usize == nib {
            pos += 1;
        }
        children[nib] = subtree(begin, pos, depth + 1, count, value, buf, crypto);
    }
    buf.clear();
    buf.resize(9, 0);
    for child in children {
        if child.is_valid() {
            reference(buf, child);
        } else {
            buf.push(0x80);
        }
    }
    buf.push(0x80);
    finish(buf, crypto)
}
/// Compute the Ethereum trie root for a dense list keyed by `RLP(index)`.
///
/// `value` returns the already-encoded trie value for an index in `0..count`.
/// It is called exactly once per index, in lexicographic RLP-key order rather
/// than numeric order. Empty lists return `EMPTY_TRIE_HASH` without calling it.
/// Uses bounded recursion and one shared node-encoding buffer, without building
/// or retaining a mutable trie.
pub fn ordered_root(
    count: usize,
    mut value: impl FnMut(usize) -> Vec<u8>,
    crypto: &dyn Crypto,
) -> H256 {
    if count == 0 {
        return EMPTY_TRIE_HASH;
    }
    subtree(
        0,
        count,
        0,
        count,
        &mut value,
        &mut Vec::with_capacity(1024),
        crypto,
    )
    .finalize(crypto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_crypto::NativeCrypto;
    use ethrex_rlp::encode::RLPEncode;
    #[test]
    fn visits_each_index_once_in_rlp_key_order() {
        for count in [0, 1, 127, 128, 129, 256, 257, 65537] {
            let mut visited = Vec::new();
            ordered_root(
                count,
                |index| {
                    visited.push(index);
                    vec![42]
                },
                &NativeCrypto,
            );
            let mut expected: Vec<usize> = (0..count).collect();
            expected.sort_by_key(|index| index.encode_to_vec());
            assert_eq!(visited, expected, "count={count}");
        }
    }
    #[test]
    fn matches_mutable_trie_at_boundaries() {
        for count in [
            0, 1, 2, 15, 16, 17, 127, 128, 129, 255, 256, 257, 4096, 65537,
        ] {
            let value =
                |i: usize| vec![(i % 251) as u8; [0, 1, 2, 27, 28, 31, 32, 55, 56, 128][i % 10]];
            let expected = crate::Trie::compute_hash_from_unsorted_iter(
                (0..count).map(|i| (i.encode_to_vec(), value(i))),
                &NativeCrypto,
            );
            assert_eq!(
                ordered_root(count, value, &NativeCrypto),
                expected,
                "count={count}"
            );
        }
    }
}
