//! # State-history journal
//!
//! Per-block reverse-diff entries persisted to disk so reorgs deeper than the
//! in-memory `TrieLayerCache` become possible up to the finalized boundary.
//!
//! Each entry captures the previous on-disk values (or absence markers) for every
//! account-trie node, storage-trie node, account flat-key-value, and storage
//! flat-key-value path that a single layer commit overwrites. Codes are
//! content-addressed and not journaled.
//!
//! Entries are keyed by `block_number.to_be_bytes()` in the
//! [`STATE_HISTORY`](crate::api::tables::STATE_HISTORY) column family. Big-endian
//! ensures lexicographic order matches numeric order, which lets finality
//! pruning use a single `delete_range`.
//!
//! ## Pruning model
//!
//! When a `forkchoice_update` advances the finalized block, `forkchoice_update_inner`
//! calls `delete_range(STATE_HISTORY, 0, finalized_number + 1)`, removing all journal
//! entries at or below the new finality boundary. The surviving entries cover
//! `[finalized_number+1, cache_edge_D]`, which is exactly the window a future
//! deep reorg could need. After pruning, `Store::lowest_state_history_block_number`
//! reflects the new floor.
//!
//! ## Batch mode (full sync)
//!
//! When `batch_mode == true` (full sync), the commit path skips journaling
//! entirely. A full-sync import writes one layer per ~1024 blocks and does
//! not produce the per-block reverse-diffs needed for deep reorgs. Reorg
//! support is only active after the node transitions to normal block-by-block
//! execution.
//!
//! ## Codec
//!
//! Entries use a hand-rolled compact format: a version byte at offset 0, then
//! `block_hash` (32 bytes), `parent_state_root` (32 bytes), then four
//! varint-prefixed reverse-diff sections in order: account-trie, storage-trie,
//! account flat-KV, storage flat-KV. RLP/bincode/postcard are skipped — the
//! access pattern (write-once, read-on-reorg, large volume) makes encode/decode
//! cost matter.
//!
//! ## Version strategy
//!
//! [`JOURNAL_VERSION`] is a single byte at offset 0 of every entry. The decoder
//! rejects any version other than the current one with
//! [`JournalDecodeError::VersionMismatch`]. On a codec bump, the journal should
//! be drained at a finality boundary so the new binary starts with an empty
//! journal and never encounters old-format entries. A future bump that needs to
//! keep history across the upgrade should introduce per-version `decode_vN` arms
//! rather than re-encoding existing entries.

use ethrex_common::H256;

/// Current version of the journal entry codec.
///
/// Bumping this constant changes the wire format. The decoder rejects any
/// other version with [`JournalDecodeError::VersionMismatch`]: a v(N) binary
/// will refuse to interpret v(N+1) entries (forward safety) and will also
/// refuse to read v(N-1) entries written by a previous binary (no implicit
/// fallback). The plan for the rollback consumer (PR 2/3/4) is to drain
/// the journal at a finality boundary on upgrade, so the v(N) journal
/// starts empty after the bump; a future bump that needs to keep history
/// across the upgrade should introduce per-version `decode_vN` arms here
/// rather than re-encoding existing entries.
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
/// without further interpretation: `Some(prev)` becomes a `put`, `None`
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
    #[error(
        "journal entry presence byte invalid: expected 0 or 1, found {found} at offset {offset}"
    )]
    InvalidPresenceByte { offset: usize, found: u8 },
    #[error("journal entry has {trailing} trailing bytes after offset {offset}")]
    TrailingBytes { offset: usize, trailing: usize },
    #[error(
        "journal entry length prefix {claimed} at offset {offset} exceeds remaining bytes {remaining}"
    )]
    LengthExceedsRemaining {
        offset: usize,
        claimed: u64,
        remaining: usize,
    },
}

impl JournalEntry {
    /// Encode this entry into its on-disk byte representation.
    pub fn encode(&self) -> Vec<u8> {
        let approx = 1
            + 32
            + 32
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

        // Reject trailing bytes: a corrupt or mixed-version record that happens to
        // have a valid prefix must not be silently treated as valid.
        if cur.offset != bytes.len() {
            return Err(JournalDecodeError::TrailingBytes {
                offset: cur.offset,
                trailing: bytes.len() - cur.offset,
            });
        }

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

fn encode_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

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

/// Returns the encoded LEB128 length of `value`. 1 byte per 7 bits, with bytes
/// 1-9 having the continuation bit set.
fn varint_len(value: u64) -> usize {
    let bits = 64 - value.leading_zeros() as usize;
    bits.div_ceil(7).max(1)
}

fn diff_byte_estimate(diff: &[ReverseDiffEntry]) -> usize {
    // varint(path_len) + path + presence_byte + (value_section if Some).
    // value_section = varint(value_len) + value.
    diff.iter()
        .map(|(p, v)| {
            varint_len(p.len() as u64)
                + p.len()
                + 1
                + v.as_ref()
                    .map_or(0, |v| varint_len(v.len() as u64) + v.len())
        })
        .sum::<usize>()
        + varint_len(diff.len() as u64)
}

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
        // Saturating form: explicit even though `offset <= bytes.len()` is an
        // invariant maintained by `read_byte` / `read_slice` themselves.
        let remaining = self.bytes.len().saturating_sub(self.offset);
        if remaining < n {
            return Err(JournalDecodeError::Truncated {
                offset: self.offset,
                expected: n,
            });
        }
        let s = &self.bytes[self.offset..self.offset + n];
        self.offset += n;
        Ok(s)
    }

    /// Returns the number of unread bytes in the cursor.
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
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
            // Maximum 10 bytes for u64 LEB128 (10 * 7 = 70 > 64). Reject the
            // 11th byte unconditionally.
            if shift >= 64 {
                return Err(JournalDecodeError::VarintOverflow {
                    offset: self.offset - 1,
                });
            }
            // At shift==63 only bit 0 of the final byte fits into a u64; bits 1-6
            // would shift past position 63 and be silently dropped. A continuation
            // bit at this point is also invalid: a u64 LEB128 is at most 10 bytes.
            if shift == 63 && (b & 0x7e != 0 || b & 0x80 != 0) {
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

/// Smallest possible per-entry overhead in bytes: `varint(path_len=0)` + 0 path
/// bytes + 1 presence byte. Used to bound `Vec::with_capacity(count)` against the
/// actual payload size so a corrupt count prefix can't trigger OOM.
const MIN_ENTRY_BYTES: usize = 2;

fn decode_flat_diff(cur: &mut Cursor<'_>) -> Result<FlatDiff, JournalDecodeError> {
    let count_offset = cur.offset;
    let count_u64 = cur.read_varint()?;
    let remaining = cur.remaining();
    // Each entry needs at least MIN_ENTRY_BYTES of payload. Reject a count that
    // can't possibly fit in the remaining buffer ; otherwise `Vec::with_capacity`
    // could request near-`usize::MAX` and panic with OOM.
    let max_possible = remaining / MIN_ENTRY_BYTES;
    if count_u64 as usize > max_possible {
        return Err(JournalDecodeError::LengthExceedsRemaining {
            offset: count_offset,
            claimed: count_u64,
            remaining,
        });
    }
    let count = count_u64 as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let path_len_offset = cur.offset;
        let path_len_u64 = cur.read_varint()?;
        // Guard the path-length allocation against malformed input the same way.
        let remaining = cur.remaining();
        if path_len_u64 > remaining as u64 {
            return Err(JournalDecodeError::LengthExceedsRemaining {
                offset: path_len_offset,
                claimed: path_len_u64,
                remaining,
            });
        }
        let path_len = path_len_u64 as usize;
        let path = cur.read_slice(path_len)?.to_vec();
        let presence_offset = cur.offset;
        let presence = cur.read_byte()?;
        let value = match presence {
            0 => None,
            1 => {
                let value_len_offset = cur.offset;
                let value_len_u64 = cur.read_varint()?;
                let remaining = cur.remaining();
                if value_len_u64 > remaining as u64 {
                    return Err(JournalDecodeError::LengthExceedsRemaining {
                        offset: value_len_offset,
                        claimed: value_len_u64,
                        remaining,
                    });
                }
                let value_len = value_len_u64 as usize;
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
        H256::repeat_byte(b)
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
        // 1 (version) + 32 + 32 + 1 (count=0) * 4 = 69 bytes.
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
            storage_trie_diff: vec![(vec![0x0a; 67], Some(vec![0xff])), (vec![0x0b; 68], None)],
            account_flat_diff: vec![(vec![0xaa; 65], Some(vec![0x01, 0x02, 0x03]))],
            storage_flat_diff: vec![(vec![0xbb; 131], None)],
        };
        round_trip(&entry);
    }

    #[test]
    fn entry_with_only_absences_round_trips() {
        let entry = JournalEntry {
            block_hash: h(0x55),
            parent_state_root: h(0x66),
            account_trie_diff: vec![(vec![0x00], None), (vec![0x01], None), (vec![0x02], None)],
            storage_trie_diff: vec![],
            account_flat_diff: vec![(vec![0xaa; 32], None)],
            storage_flat_diff: vec![],
        };
        round_trip(&entry);
    }

    #[test]
    fn large_entry_round_trips() {
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
        let mut bytes = vec![0xff];
        bytes.extend_from_slice(&[0; 32]);
        bytes.extend_from_slice(&[0; 32]);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
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
        let err = JournalEntry::decode(&bytes[..bytes.len() - 1]).unwrap_err();
        assert!(matches!(err, JournalDecodeError::Truncated { .. }));
    }

    #[test]
    fn rejects_invalid_presence_byte() {
        let mut bytes = Vec::new();
        bytes.push(JOURNAL_VERSION);
        bytes.extend_from_slice(&[0; 32]);
        bytes.extend_from_slice(&[0; 32]);
        bytes.push(1); // account_trie_diff count = 1
        bytes.push(1); // path_len = 1
        bytes.push(0xab); // path
        bytes.push(2); // presence = 2 (invalid)
        let err = JournalEntry::decode(&bytes).unwrap_err();
        assert!(matches!(
            err,
            JournalDecodeError::InvalidPresenceByte { found: 2, .. }
        ));
    }

    #[test]
    fn varint_round_trip() {
        for &v in &[0u64, 1, 127, 128, 16_383, 16_384, u32::MAX as u64, u64::MAX] {
            let mut buf = Vec::new();
            encode_varint(&mut buf, v);
            let mut cur = Cursor::new(&buf);
            assert_eq!(cur.read_varint().unwrap(), v);
        }
    }

    /// At shift==63 only bit 0 of the 10th byte fits in a u64. A 10-byte LEB128
    /// where the 10th byte has bits 1-6 set encodes a value > u64::MAX; the
    /// decoder must reject it rather than silently truncate the extra bits.
    #[test]
    fn rejects_varint_with_truncating_10th_byte() {
        // 9 continuation bytes carrying zero data + 10th byte 0x7e (bits 1-5 set,
        // no continuation). Without the guard, this decodes the same as 0x00 at
        // shift==63 because the shifted bits fall outside the u64.
        let mut buf = vec![0x80; 9];
        buf.push(0x7e);
        let mut cur = Cursor::new(&buf);
        let err = cur.read_varint().unwrap_err();
        assert!(matches!(err, JournalDecodeError::VarintOverflow { .. }));
    }

    /// An 11th byte must be rejected regardless of its bits.
    #[test]
    fn rejects_varint_with_11th_byte() {
        let mut buf = vec![0x80; 10];
        buf.push(0x00);
        let mut cur = Cursor::new(&buf);
        let err = cur.read_varint().unwrap_err();
        assert!(matches!(err, JournalDecodeError::VarintOverflow { .. }));
    }

    /// A 10th byte with the continuation bit set claims an 11th byte and must
    /// be rejected even if its data bits are valid.
    #[test]
    fn rejects_varint_with_continuation_at_byte_10() {
        let mut buf = vec![0x80; 9];
        buf.push(0x81); // bit 0 set + continuation
        let mut cur = Cursor::new(&buf);
        let err = cur.read_varint().unwrap_err();
        assert!(matches!(err, JournalDecodeError::VarintOverflow { .. }));
    }

    /// u64::MAX must still round-trip after the tightened decoder (its 10-byte
    /// LEB128 has 10th byte 0x01, which has zero in bits 1-6 and no continuation).
    #[test]
    fn u64_max_round_trips_after_tighter_decoder() {
        let mut buf = Vec::new();
        encode_varint(&mut buf, u64::MAX);
        assert_eq!(buf.len(), 10);
        let mut cur = Cursor::new(&buf);
        assert_eq!(cur.read_varint().unwrap(), u64::MAX);
    }

    /// A corrupt count prefix (e.g. u64::MAX) must NOT cause a near-`usize::MAX`
    /// allocation. We expect `LengthExceedsRemaining` before any vec is allocated.
    #[test]
    fn rejects_oom_via_malformed_count() {
        // Manually craft a payload: version + 32B block_hash + 32B parent_state_root
        // + account_trie_diff count = u64::MAX (10-byte LEB128). The remaining
        // payload is too small to hold that many entries.
        let mut bytes = vec![JOURNAL_VERSION];
        bytes.extend_from_slice(&[0; 32]);
        bytes.extend_from_slice(&[0; 32]);
        encode_varint(&mut bytes, u64::MAX);
        let err = JournalEntry::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, JournalDecodeError::LengthExceedsRemaining { .. }),
            "expected LengthExceedsRemaining, got {err:?}"
        );
    }

    /// A corrupt path-length must NOT cause a near-`usize::MAX` allocation.
    #[test]
    fn rejects_oom_via_malformed_path_len() {
        let mut bytes = vec![JOURNAL_VERSION];
        bytes.extend_from_slice(&[0; 32]);
        bytes.extend_from_slice(&[0; 32]);
        encode_varint(&mut bytes, 1); // count = 1
        encode_varint(&mut bytes, u64::MAX); // path_len = u64::MAX
        let err = JournalEntry::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, JournalDecodeError::LengthExceedsRemaining { .. }),
            "expected LengthExceedsRemaining, got {err:?}"
        );
    }

    /// A corrupt value-length must NOT cause a near-`usize::MAX` allocation.
    #[test]
    fn rejects_oom_via_malformed_value_len() {
        let mut bytes = vec![JOURNAL_VERSION];
        bytes.extend_from_slice(&[0; 32]);
        bytes.extend_from_slice(&[0; 32]);
        encode_varint(&mut bytes, 1); // count = 1
        encode_varint(&mut bytes, 1); // path_len = 1
        bytes.push(0xaa); // path
        bytes.push(1); // presence = 1
        encode_varint(&mut bytes, u64::MAX); // value_len = u64::MAX
        let err = JournalEntry::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, JournalDecodeError::LengthExceedsRemaining { .. }),
            "expected LengthExceedsRemaining, got {err:?}"
        );
    }

    /// Trailing bytes after a valid prefix must be rejected. A corrupt or
    /// mixed-version record could otherwise be silently treated as valid.
    #[test]
    fn rejects_trailing_bytes() {
        let entry = JournalEntry {
            block_hash: h(0xaa),
            parent_state_root: h(0xbb),
            account_trie_diff: vec![],
            storage_trie_diff: vec![],
            account_flat_diff: vec![],
            storage_flat_diff: vec![],
        };
        let mut bytes = entry.encode();
        bytes.push(0xff); // unexpected trailing byte
        let err = JournalEntry::decode(&bytes).unwrap_err();
        match err {
            JournalDecodeError::TrailingBytes { trailing, .. } => assert_eq!(trailing, 1),
            other => panic!("expected TrailingBytes, got {other:?}"),
        }
    }

    /// `diff_byte_estimate` must be a lower-bound that matches the actual encoded
    /// length when paths/values cross varint width boundaries (>= 128 bytes).
    #[test]
    fn diff_byte_estimate_handles_large_lengths() {
        let diff: Vec<ReverseDiffEntry> = vec![
            (vec![0xaa; 200], Some(vec![0xbb; 300])), // both > 128, 2-byte varints
            (vec![0xcc; 50], Some(vec![0xdd; 50])),   // < 128, 1-byte varints
        ];
        // The estimate is consumed via diff_byte_estimate() in encode()'s
        // Vec::with_capacity hint. Verify it matches the actual encoded length
        // for one diff section so reallocations don't fire on the hot path.
        let mut buf = Vec::new();
        encode_flat_diff(&mut buf, &diff);
        assert_eq!(
            diff_byte_estimate(&diff),
            buf.len(),
            "estimate must equal actual encoded length so encode() avoids realloc"
        );
    }
}
