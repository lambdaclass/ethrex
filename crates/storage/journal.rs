//! # State-history journal
//!
//! Per-block reverse-diff entries persisted to RocksDB so reorgs deeper than the
//! in-memory `TrieLayerCache` become possible up to the finalized boundary.
//!
//! Each entry captures the previous on-disk values (or absence markers) for every
//! account-trie node, storage-trie node, account flat-key-value, and storage
//! flat-key-value path that a single layer commit overwrites. Codes are
//! content-addressed and not journaled.
//!
//! Entries are keyed by `block_number.to_be_bytes()` in the
//! [`STATE_HISTORY`](crate::api::tables::STATE_HISTORY) column family.
//!
//! ## Codec
//!
//! Entries use a hand-rolled compact format (see design.md §D15): a version
//! byte at offset 0, then `block_hash` (32 bytes), `parent_state_root`
//! (32 bytes), then four varint-prefixed reverse-diff sections in order:
//! account-trie, storage-trie, account flat-KV, storage flat-KV. RLP, bincode,
//! and postcard are deliberately avoided — RLP has shown to be slow and clunky
//! for nested optional payloads of this shape elsewhere in the codebase, and
//! the access pattern (write-once, read-on-reorg, large volume) makes
//! encode/decode cost matter.

use ethrex_common::H256;

/// Current version of the journal entry codec.
pub const JOURNAL_VERSION: u8 = 1;

/// A single reverse-diff entry: `(on_disk_key, previous_value_or_none)`.
///
/// `on_disk_key` is the exact key written to its column family — for storage
/// CFs this includes the nibble-encoded account-hash prefix. `Some(prev)`
/// means the key existed on disk with `prev` before the commit; `None` means
/// the key did not exist on disk (i.e., the commit added it, and a rollback
/// should remove it).
pub type ReverseDiffEntry = (Vec<u8>, Option<Vec<u8>>);

/// A flat list of reverse-diff entries.
pub type FlatDiff = Vec<ReverseDiffEntry>;

/// A single reverse-diff entry covering one block's commit.
///
/// All four diff sections are flat lists of `(on_disk_key, prev_value)` tuples.
/// On rollback, each entry can be applied directly to its column family
/// without further interpretation: a `Some(prev)` becomes a `put`, a `None`
/// becomes a `delete`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    /// Hash of the block whose commit this entry reverses.
    pub block_hash: H256,
    /// Post-state root of the parent block (the state we'd return to on rollback).
    pub parent_state_root: H256,
    /// Reverse diff for `ACCOUNT_TRIE_NODES`.
    pub account_trie_diff: FlatDiff,
    /// Reverse diff for `STORAGE_TRIE_NODES`. Keys carry the nibble-encoded
    /// account-hash prefix as written on disk; no separate grouping is needed.
    pub storage_trie_diff: FlatDiff,
    /// Reverse diff for `ACCOUNT_FLATKEYVALUE`.
    pub account_flat_diff: FlatDiff,
    /// Reverse diff for `STORAGE_FLATKEYVALUE`. Keys carry the nibble-encoded
    /// account-hash prefix as written on disk.
    pub storage_flat_diff: FlatDiff,
}

/// Errors that can occur when decoding a journal entry from disk.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum JournalDecodeError {
    #[error("journal entry truncated: expected {expected} more bytes at offset {offset}")]
    Truncated { offset: usize, expected: usize },
    #[error("journal entry version mismatch: expected {expected}, found {found}")]
    VersionMismatch { expected: u8, found: u8 },
    #[error("journal entry varint overflow at offset {offset}")]
    VarintOverflow { offset: usize },
    #[error("journal entry presence byte invalid: expected 0 or 1, found {found} at offset {offset}")]
    InvalidPresenceByte { offset: usize, found: u8 },
}

impl JournalEntry {
    /// Encode this entry into its on-disk byte representation.
    pub fn encode(&self) -> Vec<u8> {
        // Heuristic: ~70 bytes overhead + ~50 bytes per typical small entry.
        let approx = 1 + 32 + 32
            + diff_byte_estimate(&self.account_trie_diff)
            + diff_byte_estimate(&self.storage_trie_diff)
            + diff_byte_estimate(&self.account_flat_diff)
            + diff_byte_estimate(&self.storage_flat_diff);
        let mut out = Vec::with_capacity(approx);

        out.push(JOURNAL_VERSION);
        out.extend_from_slice(self.block_hash.as_bytes());
        out.extend_from_slice(self.parent_state_root.as_bytes());

        encode_flat_diff(&mut out, &self.account_trie_diff);
        encode_flat_diff(&mut out, &self.storage_trie_diff);
        encode_flat_diff(&mut out, &self.account_flat_diff);
        encode_flat_diff(&mut out, &self.storage_flat_diff);

        out
    }

    /// Decode an entry from its on-disk byte representation.
    ///
    /// Returns [`JournalDecodeError::VersionMismatch`] if the version byte is
    /// not [`JOURNAL_VERSION`]. The current binary deliberately refuses to
    /// interpret entries written by a future codec version rather than silently
    /// producing a malformed reverse-diff.
    pub fn decode(bytes: &[u8]) -> Result<Self, JournalDecodeError> {
        let mut cur = Cursor::new(bytes);

        let version = cur.read_byte()?;
        if version != JOURNAL_VERSION {
            return Err(JournalDecodeError::VersionMismatch {
                expected: JOURNAL_VERSION,
                found: version,
            });
        }

        let block_hash = cur.read_h256()?;
        let parent_state_root = cur.read_h256()?;

        let account_trie_diff = decode_flat_diff(&mut cur)?;
        let storage_trie_diff = decode_flat_diff(&mut cur)?;
        let account_flat_diff = decode_flat_diff(&mut cur)?;
        let storage_flat_diff = decode_flat_diff(&mut cur)?;

        Ok(Self {
            block_hash,
            parent_state_root,
            account_trie_diff,
            storage_trie_diff,
            account_flat_diff,
            storage_flat_diff,
        })
    }
}

// --- varint (LEB128 unsigned) ---------------------------------------------

fn encode_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

// --- diff section encoders -------------------------------------------------

fn encode_flat_diff(out: &mut Vec<u8>, diff: &[ReverseDiffEntry]) {
    encode_varint(out, diff.len() as u64);
    for (path, value) in diff {
        encode_varint(out, path.len() as u64);
        out.extend_from_slice(path);
        match value {
            None => out.push(0),
            Some(v) => {
                out.push(1);
                encode_varint(out, v.len() as u64);
                out.extend_from_slice(v);
            }
        }
    }
}

fn diff_byte_estimate(diff: &[ReverseDiffEntry]) -> usize {
    diff.iter()
        .map(|(p, v)| 2 + p.len() + 1 + v.as_ref().map_or(0, |v| 2 + v.len()))
        .sum::<usize>()
        + 2
}

// --- cursor / decoders -----------------------------------------------------

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn read_byte(&mut self) -> Result<u8, JournalDecodeError> {
        if self.offset >= self.bytes.len() {
            return Err(JournalDecodeError::Truncated {
                offset: self.offset,
                expected: 1,
            });
        }
        let b = self.bytes[self.offset];
        self.offset += 1;
        Ok(b)
    }

    fn read_slice(&mut self, n: usize) -> Result<&'a [u8], JournalDecodeError> {
        if self.bytes.len() - self.offset < n {
            return Err(JournalDecodeError::Truncated {
                offset: self.offset,
                expected: n,
            });
        }
        let s = &self.bytes[self.offset..self.offset + n];
        self.offset += n;
        Ok(s)
    }

    fn read_h256(&mut self) -> Result<H256, JournalDecodeError> {
        let s = self.read_slice(32)?;
        Ok(H256::from_slice(s))
    }

    fn read_varint(&mut self) -> Result<u64, JournalDecodeError> {
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            let b = self.read_byte()?;
            // Maximum 10 bytes for u64 LEB128 (10 * 7 = 70 > 64).
            if shift >= 64 {
                return Err(JournalDecodeError::VarintOverflow {
                    offset: self.offset - 1,
                });
            }
            result |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
    }
}

fn decode_flat_diff(cur: &mut Cursor<'_>) -> Result<FlatDiff, JournalDecodeError> {
    let count = cur.read_varint()? as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let path_len = cur.read_varint()? as usize;
        let path = cur.read_slice(path_len)?.to_vec();
        let presence_offset = cur.offset;
        let presence = cur.read_byte()?;
        let value = match presence {
            0 => None,
            1 => {
                let value_len = cur.read_varint()? as usize;
                Some(cur.read_slice(value_len)?.to_vec())
            }
            other => {
                return Err(JournalDecodeError::InvalidPresenceByte {
                    offset: presence_offset,
                    found: other,
                });
            }
        };
        out.push((path, value));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(b: u8) -> H256 {
        let mut x = [0u8; 32];
        x.fill(b);
        H256::from(x)
    }

    fn round_trip(entry: &JournalEntry) {
        let bytes = entry.encode();
        let decoded = JournalEntry::decode(&bytes).unwrap();
        assert_eq!(&decoded, entry);
    }

    #[test]
    fn empty_entry_round_trips() {
        let entry = JournalEntry {
            block_hash: h(0xaa),
            parent_state_root: h(0xbb),
            account_trie_diff: vec![],
            storage_trie_diff: vec![],
            account_flat_diff: vec![],
            storage_flat_diff: vec![],
        };
        round_trip(&entry);
        // Encoded shape: 1 (version) + 32 + 32 + 1 (count=0) × 4 = 69 bytes.
        assert_eq!(entry.encode().len(), 69);
    }

    #[test]
    fn typical_entry_round_trips() {
        let entry = JournalEntry {
            block_hash: h(0x11),
            parent_state_root: h(0x22),
            account_trie_diff: vec![
                (vec![0x00, 0x01], Some(vec![0xde, 0xad, 0xbe, 0xef])),
                (vec![0x02], None),
            ],
            storage_trie_diff: vec![
                (vec![0x0a; 67], Some(vec![0xff])),
                (vec![0x0b; 68], None),
            ],
            account_flat_diff: vec![(vec![0xaa; 65], Some(vec![0x01, 0x02, 0x03]))],
            storage_flat_diff: vec![(vec![0xbb; 131], None)],
        };
        round_trip(&entry);
    }

    #[test]
    fn entry_with_empty_sections_round_trips() {
        let entry = JournalEntry {
            block_hash: h(0x33),
            parent_state_root: h(0x44),
            account_trie_diff: vec![(vec![0x00], Some(vec![0xff]))],
            storage_trie_diff: vec![],
            account_flat_diff: vec![],
            storage_flat_diff: vec![(vec![0xbb; 67], None)],
        };
        round_trip(&entry);
    }

    #[test]
    fn entry_with_only_absences_round_trips() {
        // All values are None: the rollback would only delete keys, never restore them.
        let entry = JournalEntry {
            block_hash: h(0x55),
            parent_state_root: h(0x66),
            account_trie_diff: vec![
                (vec![0x00], None),
                (vec![0x01], None),
                (vec![0x02], None),
            ],
            storage_trie_diff: vec![],
            account_flat_diff: vec![(vec![0xaa; 32], None)],
            storage_flat_diff: vec![],
        };
        round_trip(&entry);
    }

    #[test]
    fn large_entry_round_trips() {
        // 10k entries to exercise allocations and varint widths.
        let mut account_trie_diff = Vec::with_capacity(10_000);
        for i in 0u32..10_000 {
            let path = i.to_be_bytes().to_vec();
            let value = if i % 7 == 0 {
                None
            } else {
                Some(vec![(i & 0xff) as u8; (i % 200) as usize])
            };
            account_trie_diff.push((path, value));
        }
        let entry = JournalEntry {
            block_hash: h(0xee),
            parent_state_root: h(0xff),
            account_trie_diff,
            storage_trie_diff: vec![],
            account_flat_diff: vec![],
            storage_flat_diff: vec![],
        };
        round_trip(&entry);
    }

    #[test]
    fn rejects_unknown_version() {
        let mut bytes = vec![0xff]; // bogus version
        bytes.extend_from_slice(&[0; 32]); // block_hash
        bytes.extend_from_slice(&[0; 32]); // parent_state_root
        bytes.extend_from_slice(&[0, 0, 0, 0]); // four empty diff sections
        let err = JournalEntry::decode(&bytes).unwrap_err();
        assert_eq!(
            err,
            JournalDecodeError::VersionMismatch {
                expected: JOURNAL_VERSION,
                found: 0xff,
            }
        );
    }

    #[test]
    fn rejects_truncated_input() {
        let entry = JournalEntry {
            block_hash: h(0x77),
            parent_state_root: h(0x88),
            account_trie_diff: vec![(vec![0x00], Some(vec![0xff]))],
            storage_trie_diff: vec![],
            account_flat_diff: vec![],
            storage_flat_diff: vec![],
        };
        let bytes = entry.encode();
        // Chop off the last byte — decode should report truncation.
        let err = JournalEntry::decode(&bytes[..bytes.len() - 1]).unwrap_err();
        assert!(matches!(err, JournalDecodeError::Truncated { .. }));
    }

    #[test]
    fn rejects_invalid_presence_byte() {
        // Manually craft a payload with an invalid presence marker (2).
        let mut bytes = Vec::new();
        bytes.push(JOURNAL_VERSION);
        bytes.extend_from_slice(&[0; 32]);
        bytes.extend_from_slice(&[0; 32]);
        bytes.push(1); // account_trie_diff count = 1
        bytes.push(1); // path_len = 1
        bytes.push(0xab); // path
        bytes.push(2); // presence = 2 (invalid)
        let err = JournalEntry::decode(&bytes).unwrap_err();
        assert!(matches!(err, JournalDecodeError::InvalidPresenceByte { found: 2, .. }));
    }

    #[test]
    fn varint_round_trip() {
        // Hit a few interesting widths.
        for &v in &[0u64, 1, 127, 128, 16_383, 16_384, u32::MAX as u64, u64::MAX] {
            let mut buf = Vec::new();
            encode_varint(&mut buf, v);
            let mut cur = Cursor::new(&buf);
            assert_eq!(cur.read_varint().unwrap(), v);
        }
    }
}
