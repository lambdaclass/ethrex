//! # State-history journal
//!
//! Per-block reverse-diff entries persisted to disk so reorgs deeper than the
//! in-memory `TrieLayerCache` become possible up to the finalized boundary.
//!
//! Each entry captures the previous on-disk values (or absence markers) for every
//! account-trie node, storage-trie node, account flat-key-value, storage
//! flat-key-value and EIP-8297 binary-trie path that a single layer commit
//! overwrites. Codes are content-addressed and not journaled.
//!
//! ## Why the binary trie is journaled too
//!
//! The binary trie is path-keyed and single-version, exactly like the MPT's node
//! tables, so a commit that advances it destroys the previous version. Diff
//! layers cover every reorg *inside* the layer window — those nodes are never
//! written — but a reorg deeper than the cache edge has to put the on-disk trie
//! back, and only a reverse diff can do that. Without it the overlay rebuilds an
//! MPT that a post-activation header does not address, and re-executing the new
//! chain fails with a state-root mismatch it can never recover from.
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
//! `block_hash` (32 bytes), `parent_state_root` (32 bytes),
//! `parent_binary_root` (32 bytes), then six varint-prefixed reverse-diff
//! sections in order: account-trie, storage-trie, account flat-KV, storage
//! flat-KV, binary-trie, binary flat-KV. RLP/bincode/postcard are skipped — the
//! access pattern (write-once, read-on-reorg, large volume) makes encode/decode
//! cost matter.
//!
//! The binary sections are flat lists of their own rather than something folded
//! into the MPT four, because the reader cannot tell the column families apart
//! by key length: a `BitPath` key is `4 + ceil(bits/8)` bytes, which overlaps
//! the whole MPT range that `classify_trie_key` dispatches on. The section
//! boundary is the only thing that says which table a key belongs to.
//!
//! The two binary sections must stay apart from *each other* for a sharper
//! version of that reason. A `BitPath` DB key at bit-depth 240 is 34 bytes and
//! at bit-depth 496 is 66 bytes — byte-for-byte the lengths of account-zone and
//! storage-zone tree keys. Binary nodes and binary flat entries are not
//! distinguishable by length from each other either, so the only thing that
//! routes a key to `BINARY_TRIE_NODES` rather than `BINARY_FLATKEYVALUE` is
//! which section it arrived in.
//!
//! ## v2 was redefined in place; a datadir predating that must be wiped
//!
//! The sixth section (binary flat-KV) was added to v2 rather than by bumping to
//! v3. The format is unreleased and has no external consumers, so a bump would
//! buy a compatibility guarantee nobody needs and add a migration path with no
//! users. `STATE_HISTORY` is a rebuildable cache of recent history, not durable
//! state: losing it costs reorg *reach* until finality advances and the layer
//! cache refills, the same bounded consequence [`drain_stale_journal_entries`]
//! already has.
//!
//! Three things follow, and each is load-bearing:
//!
//! - **[`drain_stale_journal_entries`] will not fire.** It compares the version
//!   *byte*, and the byte is unchanged, so its fast path returns `Ok(None)` and
//!   old five-section entries survive to reach [`JournalEntry::decode`].
//! - **What happens then is a loud, deterministic
//!   [`JournalDecodeError::Truncated`].** [`JournalEntry::encode`] is
//!   exact-length, so a five-section record leaves the cursor at
//!   `offset == bytes.len()` after the fifth section; the sixth
//!   `decode_flat_diff` then reads its count varint at `offset >= len` and
//!   errors. Never a silent misread. The downgrade direction is equally loud:
//!   an old binary reading a new entry decodes five sections and fails the
//!   trailing-bytes check with [`JournalDecodeError::TrailingBytes`].
//! - **The section is appended *last*, and what that buys is narrower than it
//!   looks — checked against the codec rather than assumed.** The tempting claim
//!   is that the loud failure *depends* on the position: insert the section in
//!   the middle and an old entry's next section's count varint would be read as
//!   the new section's count, and the record could parse into garbage. **That is
//!   not true of this codec**, and it was tested rather than argued
//!   (`a_five_section_v2_record_fails_loudly_as_truncated` was mutation-checked
//!   by relocating the section to position two, in both `encode` and `decode`,
//!   with a fixture carrying content in every section; the outcome was the same
//!   `Truncated`, at the same offset). Sections here are *self-delimiting* — a
//!   count varint, then that many self-delimiting entries — so reading six
//!   sections from a stream that is exactly five consumes the same five
//!   partitions in the same order and runs off the end on the sixth, whatever
//!   the new section is labelled. The failure is position-independent.
//!
//!   What appending last genuinely buys is that **the five existing sections
//!   keep their byte offsets**: a current record with an empty sixth section is
//!   the old five-section encoding plus one `0` byte, which
//!   `appending_last_leaves_the_five_existing_sections_byte_identical` pins.
//!   That is the property a future lenient decode would need — "a missing
//!   trailing section defaults to empty" is sound only for a trailing section;
//!   the same leniency around a middle section would silently re-route every
//!   key after it. It is also what keeps hand-rolled fixtures and any
//!   offset-aware tooling valid. Put any future section on the end for those
//!   reasons, not for the failure mode.
//!
//! Redefining a version byte in place is acceptable only while the format is
//! unreleased. Once it has consumers, [`JOURNAL_VERSION`]'s own doc comment
//! states the rule that applies instead: per-version `decode_vN` arms, not
//! re-encoding.
//!
//! ## Version strategy
//!
//! [`JOURNAL_VERSION`] is a single byte at offset 0 of every entry. The decoder
//! rejects any version other than the current one with
//! [`JournalDecodeError::VersionMismatch`]. On a codec bump, the journal is
//! drained on the next startup by [`drain_stale_journal_entries`], so the new
//! binary starts without old-format entries below its own and never encounters
//! one mid-reorg. A future bump that needs to keep history across the upgrade
//! should introduce per-version `decode_vN` arms rather than re-encoding
//! existing entries.

use ethrex_common::{H256, types::BlockNumber};
use tracing::info;

use crate::{
    api::{StorageBackend, tables::STATE_HISTORY},
    error::StoreError,
};

/// Current version of the journal entry codec.
///
/// Bumping this constant changes the wire format. The decoder rejects any
/// other version with [`JournalDecodeError::VersionMismatch`]: a v(N) binary
/// will refuse to interpret v(N+1) entries (forward safety) and will also
/// refuse to read v(N-1) entries written by a previous binary (no implicit
/// fallback). The rollback consumer (the deep-reorg overlay) is kept away from
/// stale entries by [`drain_stale_journal_entries`], which deletes them at
/// startup before a `Store` exists; a future bump that needs to keep history
/// across the upgrade should introduce per-version `decode_vN` arms here
/// rather than re-encoding existing entries.
///
/// v2 added `parent_binary_root` and the binary-trie reverse-diff section, so a
/// deep reorg can unwind the EIP-8297 trie as well as the MPT. v1 entries are
/// rejected outright rather than read with an empty binary section: on a
/// scheduled chain that would silently claim the binary trie needs no unwinding,
/// which is precisely the bug the version exists to prevent.
///
/// v2 was later **redefined in place** to add a sixth section, the binary
/// flat-KV reverse diff, rather than bumped to v3. See the "v2 was redefined in
/// place" section of this module's docs for why, and for the consequence: a
/// datadir predating that change must be wiped, because the drain will not fire
/// and its entries fail to decode.
pub const JOURNAL_VERSION: u8 = 2;

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
/// All six diff sections are flat lists of `(on_disk_key, prev_value)` tuples.
/// On rollback, each entry can be applied directly to its column family
/// without further interpretation: `Some(prev)` becomes a `put`, `None`
/// becomes a `delete`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    /// Hash of the block whose commit this entry reverses.
    pub block_hash: H256,
    /// Post-state root of the parent block (the state we'd return to on rollback).
    pub parent_state_root: H256,
    /// The parent block's EIP-8297 binary-trie root — the binary counterpart of
    /// `parent_state_root`, and the root a rollback returns the binary trie to.
    ///
    /// It has to be recorded rather than derived. Through the whole
    /// pre-activation window a header carries an MPT root, so `parent_state_root`
    /// says nothing about the binary trie; and a reader unwinding to the pivot
    /// holds no other handle on which binary root the reconstructed nodes make
    /// up. [`Overlay::serves_binary_root`] gates the binary read cascade on
    /// exactly this value.
    ///
    /// `H256::zero()` on a chain that does not schedule the commitment, where
    /// `binary_trie_diff` is empty too. Zero is never a real root, so the gate
    /// can treat it as "this overlay carries no binary state".
    ///
    /// [`Overlay::serves_binary_root`]: crate::layering::Overlay::serves_binary_root
    pub parent_binary_root: H256,
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
    /// Reverse diff for `BINARY_TRIE_NODES`, keyed by `BitPath::to_db_key()`.
    ///
    /// A `None` pre-image means the commit created the node, so a rollback
    /// deletes the key — which is also how the binary trie spells a tombstone,
    /// so the rollback and the trie's own convention agree by construction.
    ///
    /// Empty on every chain that does not schedule `binaryTreeTime`, which is
    /// the whole cost an unscheduled chain pays: one extra `0` count byte per
    /// entry.
    pub binary_trie_diff: FlatDiff,
    /// Reverse diff for `BINARY_FLATKEYVALUE`, keyed by the embedding's own tree
    /// key (34 or 66 bytes).
    ///
    /// **Appended last, and that is a format guarantee rather than an
    /// accident** — see the "v2 was redefined in place" section of this module's
    /// docs. A section inserted mid-record would let a five-section entry parse
    /// into garbage instead of failing; appended last it fails as
    /// [`JournalDecodeError::Truncated`].
    ///
    /// Its own section rather than a share of `binary_trie_diff`: the two key
    /// spaces have identical length ranges and only the section boundary says
    /// which of the two column families a key belongs to.
    ///
    /// A `None` pre-image means the commit created the row, so a rollback
    /// deletes it — which is also how the mirror spells a removed leaf, so the
    /// rollback and the mirror's own convention agree by construction. The
    /// mirror is derived state, but it is journaled all the same, including
    /// content-addressed code-zone leaves: a leftover chunk is inert to a point
    /// read and *visible to an ordered iterator*, which is the whole reason the
    /// table exists.
    ///
    /// Empty on every chain that does not schedule `binaryTreeTime`: one extra
    /// `0` count byte per entry.
    pub binary_flat_diff: FlatDiff,
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
            + 32
            + diff_byte_estimate(&self.account_trie_diff)
            + diff_byte_estimate(&self.storage_trie_diff)
            + diff_byte_estimate(&self.account_flat_diff)
            + diff_byte_estimate(&self.storage_flat_diff)
            + diff_byte_estimate(&self.binary_trie_diff)
            + diff_byte_estimate(&self.binary_flat_diff);
        let mut out = Vec::with_capacity(approx);

        out.push(JOURNAL_VERSION);
        out.extend_from_slice(self.block_hash.as_bytes());
        out.extend_from_slice(self.parent_state_root.as_bytes());
        out.extend_from_slice(self.parent_binary_root.as_bytes());

        encode_flat_diff(&mut out, &self.account_trie_diff);
        encode_flat_diff(&mut out, &self.storage_trie_diff);
        encode_flat_diff(&mut out, &self.account_flat_diff);
        encode_flat_diff(&mut out, &self.storage_flat_diff);
        encode_flat_diff(&mut out, &self.binary_trie_diff);
        // Last, always. The module docs explain why the position is a format
        // guarantee: it is what makes a stale five-section record fail as
        // `Truncated` instead of parsing into garbage.
        encode_flat_diff(&mut out, &self.binary_flat_diff);

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
        let parent_binary_root = cur.read_h256()?;

        let account_trie_diff = decode_flat_diff(&mut cur)?;
        let storage_trie_diff = decode_flat_diff(&mut cur)?;
        let account_flat_diff = decode_flat_diff(&mut cur)?;
        let storage_flat_diff = decode_flat_diff(&mut cur)?;
        let binary_trie_diff = decode_flat_diff(&mut cur)?;
        // The sixth and last section. On a five-section record written before
        // v2 was redefined in place, the cursor is already at `bytes.len()`
        // here (`encode` is exact-length), so this read fails as `Truncated` —
        // loud, deterministic, and only because the section is last.
        let binary_flat_diff = decode_flat_diff(&mut cur)?;

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
            parent_binary_root,
            account_trie_diff,
            storage_trie_diff,
            account_flat_diff,
            storage_flat_diff,
            binary_trie_diff,
            binary_flat_diff,
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

// ===========================================================================
// Startup drain
// ===========================================================================

/// What a startup drain removed. Returned so callers (and tests) can assert on
/// the outcome; the drain logs the same facts at `info!` for operators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalDrainReport {
    /// Number of entries deleted.
    pub drained: u64,
    /// `(lowest, highest)` block number of the drained range. `None` only if a
    /// key was not the usual 8-byte big-endian block number, which no writer
    /// produces; the entries are drained either way rather than left behind on
    /// the strength of a key we cannot read.
    pub range: Option<(BlockNumber, BlockNumber)>,
    /// Distinct version bytes seen among the drained entries, in first-seen
    /// order. An entry too short to carry a version byte contributes nothing
    /// here but is still counted and still drained.
    pub versions: Vec<u8>,
}

/// Deletes the contiguous *bottom* run of `STATE_HISTORY` entries whose version
/// byte is not [`JOURNAL_VERSION`], keeping every current-version entry above
/// them. Returns `None` when nothing was drained.
///
/// ## Why this has to happen, and why at startup
///
/// The decoder refuses both directions (see [`JOURNAL_VERSION`]), so after a
/// restart onto a binary with a bumped codec the on-disk journal still holds
/// old-format entries covering `[finalized+1, cache_edge]` while new blocks
/// write the new format. A deep reorg whose unwind reaches into the old portion
/// gets [`JournalDecodeError::VersionMismatch`], the overlay cannot be built,
/// and the forkchoice update fails with `StateNotReachable` — the node halts and
/// only a resync recovers it.
///
/// That window closes on its own once finality advances, since every forkchoice
/// update that moves finality prunes at or below the new finalized block. But
/// the exposure is *correlated* with the risk rather than independent of it: a
/// reorg deeper than the layer cache essentially requires finality not to be
/// advancing, which is exactly the condition that keeps the old entries around.
/// So the two are not two unlikely events multiplying, and the window is worth
/// closing deliberately.
///
/// Draining is what makes the refusal honest. `compute_reorg_ceiling` derives
/// its journal reach from `Store::lowest_state_history_block_number`; if stale
/// entries stay, that floor advertises reach the node cannot deliver and the
/// reorg fails mid-flight. Once drained, the reach collapses to layer-cache
/// retention and a too-deep forkchoice update is refused cleanly with
/// `-38006 TooDeepReorg`, which the consensus layer understands. Same practical
/// capability during the window; correct error semantics. This is the same
/// shape as the batch-import limitation in `docs/known_issues.md` ("Deep reorgs
/// into a full-synced range are refused").
///
/// ## Why the bottom run only, and not the whole table
///
/// A node restarting a second time mid-window has already written current-version
/// entries above the stale ones. Wiping unconditionally would throw those away and
/// give up reorg depth the node genuinely has. Stopping at the first current-version
/// entry keeps them. On the first boot after an upgrade there are no such entries
/// yet, so the run covers the whole table and the drain is total.
///
/// ## Cost
///
/// O(1) in the steady state: one `first_key` and one `get`, and if that bottom
/// entry is current-version we stop without touching the rest of the table. Only
/// when it is stale do we walk upward, reading entries until the first
/// current-version one. That walk is bounded by the journal window
/// `[finalized+1, cache_edge]` — a few hundred entries whenever finality is
/// moving — and it happens once, on the first startup after a codec bump.
pub fn drain_stale_journal_entries(
    backend: &dyn StorageBackend,
) -> Result<Option<JournalDrainReport>, StoreError> {
    let read = backend.begin_read()?;

    let Some(first_key) = read.first_key(STATE_HISTORY)? else {
        // Empty journal: a fresh datadir, or everything pruned by finality.
        return Ok(None);
    };

    // Steady-state fast path. The stale entries, if any, are the ones the
    // previous binary wrote, and they sit at the bottom — so if the bottom entry
    // is already current-version there is no stale run and no reason to read the
    // rest of the table. This is the branch every startup after the upgrade
    // window takes.
    if entry_version(read.get(STATE_HISTORY, &first_key)?.as_deref()) == Some(JOURNAL_VERSION) {
        return Ok(None);
    }

    // The bottom is stale. Walk upward to find where the current-version portion
    // begins; that key is the exclusive upper bound of the delete.
    let mut end_key: Option<Vec<u8>> = None;
    let mut last_stale_key = first_key.clone();
    let mut drained = 0u64;
    let mut versions: Vec<u8> = Vec::new();
    for item in read.prefix_iterator(STATE_HISTORY, &[])? {
        let (key, value) = item?;
        match entry_version(Some(value.as_ref())) {
            Some(v) if v == JOURNAL_VERSION => {
                end_key = Some(key.into_vec());
                break;
            }
            other => {
                if let Some(v) = other
                    && !versions.contains(&v)
                {
                    versions.push(v);
                }
                drained += 1;
                last_stale_key = key.into_vec();
            }
        }
    }
    // The iterator is borrowed from the read view; release both before writing.
    drop(read);

    let end_key = end_key.unwrap_or_else(|| {
        // No current-version entry at all: the run is the whole table, so the
        // exclusive bound has to sit just past the highest key. Appending a byte
        // gives the immediate lexicographic successor of `last_stale_key`, which
        // is above it and below nothing else that exists.
        let mut past_the_end = last_stale_key.clone();
        past_the_end.push(0);
        past_the_end
    });

    let mut tx = backend.begin_write()?;
    tx.delete_range(STATE_HISTORY, &first_key, &end_key)?;
    tx.commit()?;

    let range = key_to_block_number(&first_key).zip(key_to_block_number(&last_stale_key));
    let report = JournalDrainReport {
        drained,
        range,
        versions,
    };

    // An operator who sees a reorg refused with `-38006 TooDeepReorg` shortly
    // after an upgrade needs this line to explain why the node's journal reach
    // collapsed. Without it the refusal looks like a regression.
    match report.range {
        Some((low, high)) => info!(
            drained = report.drained,
            from_block = low,
            to_block = high,
            stale_versions = ?report.versions,
            current_version = JOURNAL_VERSION,
            "drained stale-version state-history entries at startup; deep-reorg reach is \
             reduced to the layer cache until finality advances past the drained range"
        ),
        None => info!(
            drained = report.drained,
            stale_versions = ?report.versions,
            current_version = JOURNAL_VERSION,
            "drained stale-version state-history entries at startup (keys were not \
             block numbers); deep-reorg reach is reduced to the layer cache"
        ),
    }

    Ok(Some(report))
}

/// The version byte of an on-disk entry: its first byte. `None` for a missing or
/// empty record, which cannot be current-version and so counts as stale.
fn entry_version(value: Option<&[u8]>) -> Option<u8> {
    value?.first().copied()
}

/// Decodes a `STATE_HISTORY` key back to its block number. `None` if the key is
/// not the 8-byte big-endian form every writer uses.
fn key_to_block_number(key: &[u8]) -> Option<BlockNumber> {
    <[u8; 8]>::try_from(key)
        .ok()
        .map(BlockNumber::from_be_bytes)
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
            parent_binary_root: h(0xcc),
            account_trie_diff: vec![],
            storage_trie_diff: vec![],
            account_flat_diff: vec![],
            storage_flat_diff: vec![],
            binary_trie_diff: vec![],
            binary_flat_diff: vec![],
        };
        round_trip(&entry);
        // 1 (version) + 32 + 32 + 32 + 1 (count=0) * 6 = 103 bytes. It was 102
        // with five sections; the sixth costs one `0` count byte per entry,
        // which is the whole price an unscheduled chain pays for the mirror.
        assert_eq!(entry.encode().len(), 103);
    }

    #[test]
    fn typical_entry_round_trips() {
        let entry = JournalEntry {
            block_hash: h(0x11),
            parent_state_root: h(0x22),
            parent_binary_root: h(0x23),
            account_trie_diff: vec![
                (vec![0x00, 0x01], Some(vec![0xde, 0xad, 0xbe, 0xef])),
                (vec![0x02], None),
            ],
            storage_trie_diff: vec![(vec![0x0a; 67], Some(vec![0xff])), (vec![0x0b; 68], None)],
            account_flat_diff: vec![(vec![0xaa; 65], Some(vec![0x01, 0x02, 0x03]))],
            storage_flat_diff: vec![(vec![0xbb; 131], None)],
            // 34-byte `BitPath::to_db_key()` shape, plus a tombstoned key whose
            // pre-image is `None` (the commit created it).
            binary_trie_diff: vec![
                (vec![0x0c; 34], Some(vec![0xcc, 0xdd])),
                (vec![0x0d; 5], None),
            ],
            // A 34-byte account-zone tree key and a 66-byte storage-zone one:
            // the same lengths the binary *node* section above uses, which is
            // the point.
            binary_flat_diff: vec![
                (vec![0x1c; 34], Some(vec![0xee; 32])),
                (vec![0x1d; 66], None),
            ],
        };
        round_trip(&entry);
    }

    #[test]
    fn entry_with_only_absences_round_trips() {
        let entry = JournalEntry {
            block_hash: h(0x55),
            parent_state_root: h(0x66),
            parent_binary_root: h(0x67),
            account_trie_diff: vec![(vec![0x00], None), (vec![0x01], None), (vec![0x02], None)],
            storage_trie_diff: vec![],
            account_flat_diff: vec![(vec![0xaa; 32], None)],
            storage_flat_diff: vec![],
            binary_trie_diff: vec![(vec![0x0c; 34], None)],
            binary_flat_diff: vec![],
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
            parent_binary_root: h(0xfe),
            account_trie_diff,
            storage_trie_diff: vec![],
            account_flat_diff: vec![],
            storage_flat_diff: vec![],
            binary_trie_diff: vec![],
            binary_flat_diff: vec![],
        };
        round_trip(&entry);
    }

    #[test]
    fn rejects_unknown_version() {
        let mut bytes = vec![0xff];
        bytes.extend_from_slice(&[0; 32]);
        bytes.extend_from_slice(&[0; 32]);
        bytes.extend_from_slice(&[0; 32]);
        bytes.extend_from_slice(&[0, 0, 0, 0, 0]);
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
            parent_binary_root: h(0x89),
            account_trie_diff: vec![(vec![0x00], Some(vec![0xff]))],
            storage_trie_diff: vec![],
            account_flat_diff: vec![],
            storage_flat_diff: vec![],
            binary_trie_diff: vec![],
            binary_flat_diff: vec![],
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
        // + 32B parent_binary_root + account_trie_diff count = u64::MAX (10-byte
        // LEB128). The remaining payload is too small to hold that many entries.
        let mut bytes = vec![JOURNAL_VERSION];
        bytes.extend_from_slice(&[0; 32]);
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
            parent_binary_root: h(0xcc),
            account_trie_diff: vec![],
            storage_trie_diff: vec![],
            account_flat_diff: vec![],
            storage_flat_diff: vec![],
            binary_trie_diff: vec![],
            binary_flat_diff: vec![],
        };
        let mut bytes = entry.encode();
        bytes.push(0xff); // unexpected trailing byte
        let err = JournalEntry::decode(&bytes).unwrap_err();
        match err {
            JournalDecodeError::TrailingBytes { trailing, .. } => assert_eq!(trailing, 1),
            other => panic!("expected TrailingBytes, got {other:?}"),
        }
    }

    /// Round-trip property: any structurally-valid `JournalEntry` must decode
    /// back to itself after `encode`. Complements the hand-written cases above
    /// by covering shapes the author did not think to enumerate (empty paths,
    /// large counts, mixed presence patterns, paths/values that straddle the
    /// varint width boundary, etc.).
    #[test]
    fn proptest_encode_decode_round_trip() {
        use proptest::collection::vec;
        use proptest::prelude::*;
        let entry_strategy = (
            any::<[u8; 32]>(),
            any::<[u8; 32]>(),
            any::<[u8; 32]>(),
            vec(flat_diff_entry(), 0..16),
            vec(flat_diff_entry(), 0..16),
            vec(flat_diff_entry(), 0..16),
            vec(flat_diff_entry(), 0..16),
            vec(flat_diff_entry(), 0..16),
            vec(flat_diff_entry(), 0..16),
        )
            .prop_map(|(bh, psr, pbr, a, b, c, d, e, f)| JournalEntry {
                block_hash: H256::from(bh),
                parent_state_root: H256::from(psr),
                parent_binary_root: H256::from(pbr),
                account_trie_diff: a,
                storage_trie_diff: b,
                account_flat_diff: c,
                storage_flat_diff: d,
                binary_trie_diff: e,
                binary_flat_diff: f,
            });
        proptest!(|(entry in entry_strategy)| {
            let bytes = entry.encode();
            let decoded = JournalEntry::decode(&bytes).expect("encoded entry must decode");
            prop_assert_eq!(decoded, entry);
        });
    }

    /// Safety property: the decoder must never panic, abort, or hang on
    /// arbitrary input. Only `Ok` or `Err` outcomes are permitted. This is the
    /// minimum bar for code that reads possibly-corrupted bytes off disk and
    /// motivated the OOM / varint-truncation / trailing-bytes hardening above.
    #[test]
    fn proptest_decoder_never_panics_on_arbitrary_bytes() {
        use proptest::collection::vec;
        use proptest::prelude::*;
        proptest!(|(bytes in vec(any::<u8>(), 0..1024))| {
            let _ = JournalEntry::decode(&bytes);
        });
    }

    /// Mutation property: flipping a byte in a valid encoding must either
    /// produce `Err` or decode to a *different* entry. A silent
    /// "corrupted-but-still-valid" result would be a hole in the decoder's
    /// validation.
    #[test]
    fn proptest_single_byte_mutation_never_silently_accepted() {
        use proptest::prelude::*;
        let entry = JournalEntry {
            block_hash: h(0x42),
            parent_state_root: h(0x43),
            parent_binary_root: h(0x44),
            account_trie_diff: vec![(vec![0x01, 0x02], Some(vec![0xaa, 0xbb]))],
            storage_trie_diff: vec![(vec![0x0a; 67], None)],
            account_flat_diff: vec![(vec![0xcc; 65], Some(vec![0xdd]))],
            storage_flat_diff: vec![],
            binary_trie_diff: vec![(vec![0x0e; 34], Some(vec![0xef]))],
            binary_flat_diff: vec![],
        };
        let baseline = entry.encode();
        proptest!(|(idx in 0..baseline.len(), bit in 0u8..8)| {
            let mut mutated = baseline.clone();
            mutated[idx] ^= 1 << bit;
            match JournalEntry::decode(&mutated) {
                Err(_) => {}
                Ok(decoded) => prop_assert_ne!(decoded, entry.clone()),
            }
        });
    }

    /// The binary section must be addressed by its position in the record, not
    /// by key length. A `BitPath` key is `4 + ceil(bits/8)` bytes, which lands
    /// squarely inside every range `classify_trie_key` uses, so a key that is
    /// byte-identical to an account-trie path must still come back in the binary
    /// section and only there.
    ///
    /// This is the property that makes the fifth section necessary rather than
    /// merely tidy: folding binary pre-images into `account_trie_diff` would put
    /// them in `ACCOUNT_TRIE_NODES` on rollback and corrupt the MPT.
    #[test]
    fn binary_and_account_sections_do_not_bleed_at_a_shared_key() {
        let shared_key = vec![0x07; 34];
        let entry = JournalEntry {
            block_hash: h(0x01),
            parent_state_root: h(0x02),
            parent_binary_root: h(0x03),
            account_trie_diff: vec![(shared_key.clone(), Some(vec![0xa1]))],
            storage_trie_diff: vec![],
            account_flat_diff: vec![],
            storage_flat_diff: vec![],
            binary_trie_diff: vec![(shared_key.clone(), Some(vec![0xb2]))],
            binary_flat_diff: vec![],
        };
        let decoded = JournalEntry::decode(&entry.encode()).unwrap();
        assert_eq!(
            decoded.account_trie_diff,
            vec![(shared_key.clone(), Some(vec![0xa1]))],
            "the account-trie pre-image must survive the shared key"
        );
        assert_eq!(
            decoded.binary_trie_diff,
            vec![(shared_key, Some(vec![0xb2]))],
            "the binary pre-image must survive it too, with its own value"
        );
    }

    /// The Decision 6 collision at its sharpest: the two *binary* sections.
    ///
    /// A `BitPath` DB key at bit-depth 240 is 34 bytes and an account-zone tree
    /// key is 34 bytes. Nothing in the bytes distinguishes them, and there are
    /// no section tags — sections are positional — so the section boundary is
    /// the only thing that routes a key to `BINARY_TRIE_NODES` rather than
    /// `BINARY_FLATKEYVALUE`. Folding the two together would, on rollback,
    /// restore a leaf value into the node table and a node encoding into the
    /// mirror, at the same key, in the same commit.
    ///
    /// The direct analogue of
    /// [`binary_and_account_sections_do_not_bleed_at_a_shared_key`], and the one
    /// existing tests cannot cover because the collision is new.
    #[test]
    fn binary_flat_and_binary_node_sections_do_not_bleed_at_a_shared_key() {
        let shared_key = vec![0x07; 34];
        let entry = JournalEntry {
            block_hash: h(0x01),
            parent_state_root: h(0x02),
            parent_binary_root: h(0x03),
            account_trie_diff: vec![],
            storage_trie_diff: vec![],
            account_flat_diff: vec![],
            storage_flat_diff: vec![],
            binary_trie_diff: vec![(shared_key.clone(), Some(vec![0xb2]))],
            binary_flat_diff: vec![(shared_key.clone(), Some(vec![0xf3; 32]))],
        };
        let decoded = JournalEntry::decode(&entry.encode()).unwrap();
        assert_eq!(
            decoded.binary_trie_diff,
            vec![(shared_key.clone(), Some(vec![0xb2]))],
            "the node pre-image must survive the shared key with its own value"
        );
        assert_eq!(
            decoded.binary_flat_diff,
            vec![(shared_key.clone(), Some(vec![0xf3; 32]))],
            "the mirror pre-image must survive it too, and must not be the node's"
        );

        // The absence direction as well: a `None` in one section must not be
        // read as a `None` in the other, because a rollback turns it into a
        // delete against a different column family.
        let asymmetric = JournalEntry {
            binary_trie_diff: vec![(shared_key.clone(), None)],
            binary_flat_diff: vec![(shared_key.clone(), Some(vec![0xf3; 32]))],
            ..entry
        };
        let decoded = JournalEntry::decode(&asymmetric.encode()).unwrap();
        assert_eq!(decoded.binary_trie_diff, vec![(shared_key.clone(), None)]);
        assert_eq!(
            decoded.binary_flat_diff,
            vec![(shared_key, Some(vec![0xf3; 32]))]
        );
    }

    /// v2 was redefined in place, so a datadir written before the sixth section
    /// existed still carries the version byte `2` and survives
    /// `drain_stale_journal_entries` untouched. This pins what happens when such
    /// a record reaches the decoder: a loud, deterministic `Truncated`, never a
    /// silent misread.
    ///
    /// **What this does not pin, and why the module docs say so.** It does *not*
    /// prove the failure depends on the section being last. Mutation-checked by
    /// relocating the section to position two in both `encode` and `decode`,
    /// with a fixture carrying content in every section: same `Truncated`, same
    /// offset. Sections are self-delimiting, so reading six of them from a
    /// five-section stream always consumes the same five partitions and runs off
    /// the end, wherever the sixth is declared. The positional guarantee is
    /// pinned separately, by
    /// [`appending_last_leaves_the_five_existing_sections_byte_identical`].
    ///
    /// Hand-rolled the way [`encode_v1_entry`] is: the five-section encoder no
    /// longer exists, and pinning this against a real predecessor shape rather
    /// than a truncated current record is the whole point.
    #[test]
    fn a_five_section_v2_record_fails_loudly_as_truncated() {
        let mut bytes = vec![JOURNAL_VERSION];
        bytes.extend_from_slice(h(0xaa).as_bytes());
        bytes.extend_from_slice(h(0xbb).as_bytes());
        bytes.extend_from_slice(h(0xcc).as_bytes());
        // account_trie_diff: one (path, None) pair, so the record is not
        // trivially empty and the failure cannot come from an empty buffer.
        encode_varint(&mut bytes, 1);
        encode_varint(&mut bytes, 1);
        bytes.push(0x42);
        bytes.push(0);
        // The other four v2 sections, all empty. Five in total — no sixth.
        for _ in 0..4 {
            encode_varint(&mut bytes, 0);
        }
        let len_before_sixth = bytes.len();

        assert_eq!(
            JournalEntry::decode(&bytes),
            Err(JournalDecodeError::Truncated {
                offset: len_before_sixth,
                expected: 1,
            }),
            "a five-section v2 record must fail at the start of the missing \
             sixth section, not decode into garbage"
        );

        // The claim that the failure is *positional* rather than incidental:
        // append the missing section and the very same bytes decode cleanly.
        encode_varint(&mut bytes, 0);
        let decoded = JournalEntry::decode(&bytes).expect("six sections decode");
        assert_eq!(decoded.account_trie_diff, vec![(vec![0x42], None)]);
        assert!(decoded.binary_flat_diff.is_empty());
    }

    /// The positional guarantee, stated as the thing that is actually true: the
    /// sixth section is *appended*, so the five that predate it keep their byte
    /// offsets and a current record with an empty sixth section is exactly the
    /// old five-section encoding plus one `0` count byte.
    ///
    /// This is what "appended last" buys, and it is not the loud failure — that
    /// happens wherever the section sits (see
    /// [`a_five_section_v2_record_fails_loudly_as_truncated`]). It is what makes
    /// a future lenient decode possible and sound: "a missing *trailing* section
    /// defaults to empty" can only be said of a trailing section, whereas the
    /// same leniency around a middle section would silently re-route every key
    /// after it into the wrong column family. Moving the section — even
    /// consistently across `encode` and `decode`, which nothing else in this
    /// file would notice — fails here.
    #[test]
    fn appending_last_leaves_the_five_existing_sections_byte_identical() {
        // A record with content in all five original sections and nothing in the
        // sixth, so the only difference from the pre-change encoding is the byte
        // the sixth section contributes.
        let entry = JournalEntry {
            block_hash: h(0xaa),
            parent_state_root: h(0xbb),
            parent_binary_root: h(0xcc),
            account_trie_diff: vec![(vec![0x01, 0x02], Some(vec![0xa1, 0xa2]))],
            storage_trie_diff: vec![(vec![0x03; 67], None)],
            account_flat_diff: vec![(vec![0x04; 65], Some(vec![0xa4; 3]))],
            storage_flat_diff: vec![(vec![0x05; 131], None)],
            binary_trie_diff: vec![(vec![0x06; 34], Some(vec![0xa6; 2]))],
            binary_flat_diff: vec![],
        };
        let encoded = entry.encode();

        // The five-section encoding, hand-rolled the way `encode_v1_entry` is:
        // the previous encoder is gone, and comparing against a truncated
        // current record would make the assertion vacuous.
        let mut five = vec![JOURNAL_VERSION];
        five.extend_from_slice(h(0xaa).as_bytes());
        five.extend_from_slice(h(0xbb).as_bytes());
        five.extend_from_slice(h(0xcc).as_bytes());
        encode_flat_diff(&mut five, &entry.account_trie_diff);
        encode_flat_diff(&mut five, &entry.storage_trie_diff);
        encode_flat_diff(&mut five, &entry.account_flat_diff);
        encode_flat_diff(&mut five, &entry.storage_flat_diff);
        encode_flat_diff(&mut five, &entry.binary_trie_diff);

        assert_eq!(
            &encoded[..five.len()],
            five.as_slice(),
            "the five pre-existing sections must sit at unchanged byte offsets"
        );
        assert_eq!(
            &encoded[five.len()..],
            &[0u8],
            "the only addition is the sixth section's empty count byte, on the end"
        );
    }

    /// The downgrade direction, and the other half of "loud in both directions":
    /// a binary that knows only five sections reads a six-section record,
    /// consumes five, and is left holding the sixth section's bytes. Simulated
    /// by decoding a six-section record truncated to nothing — the observable
    /// form is `TrailingBytes`, which the existing decoder already produces for
    /// any suffix it does not understand.
    #[test]
    fn a_five_section_reader_would_see_trailing_bytes_on_a_six_section_record() {
        let entry = JournalEntry {
            block_hash: h(0xaa),
            parent_state_root: h(0xbb),
            parent_binary_root: h(0xcc),
            account_trie_diff: vec![],
            storage_trie_diff: vec![],
            account_flat_diff: vec![],
            storage_flat_diff: vec![],
            binary_trie_diff: vec![],
            binary_flat_diff: vec![],
        };
        let mut bytes = entry.encode();
        // One trailing byte is what a six-section record looks like to a
        // five-section decoder: everything parsed, one section's worth left.
        bytes.push(0x00);
        assert!(
            matches!(
                JournalEntry::decode(&bytes),
                Err(JournalDecodeError::TrailingBytes { trailing: 1, .. })
            ),
            "an unrecognised trailing section must be rejected, not ignored"
        );
    }

    fn flat_diff_entry() -> impl proptest::strategy::Strategy<Value = ReverseDiffEntry> {
        use proptest::collection::vec;
        use proptest::prelude::*;
        (
            vec(any::<u8>(), 0..200),
            proptest::option::of(vec(any::<u8>(), 0..200)),
        )
    }

    // -----------------------------------------------------------------------
    // Startup drain
    // -----------------------------------------------------------------------

    /// A v1 entry as the previous binary wrote it: version byte 1, `block_hash`,
    /// `parent_state_root`, then *four* diff sections — no `parent_binary_root`
    /// and no binary section. Hand-rolled because the v1 encoder no longer
    /// exists, and the drain must be pinned against the real predecessor shape
    /// rather than a v2 record with its first byte overwritten.
    fn encode_v1_entry(block_number: u64) -> Vec<u8> {
        let mut bytes = vec![1u8];
        bytes.extend_from_slice(H256::repeat_byte(block_number as u8).as_bytes());
        bytes.extend_from_slice(H256::zero().as_bytes());
        // account_trie_diff: one (path, None) pair, so the record is not trivially empty.
        encode_varint(&mut bytes, 1);
        encode_varint(&mut bytes, 1);
        bytes.push(block_number as u8);
        bytes.push(0);
        // The remaining three v1 sections, all empty.
        for _ in 0..3 {
            encode_varint(&mut bytes, 0);
        }
        bytes
    }

    fn encode_current_entry(block_number: u64) -> Vec<u8> {
        JournalEntry {
            block_hash: H256::repeat_byte(block_number as u8),
            parent_state_root: H256::zero(),
            parent_binary_root: H256::zero(),
            account_trie_diff: vec![(vec![block_number as u8], None)],
            storage_trie_diff: vec![],
            account_flat_diff: vec![],
            storage_flat_diff: vec![],
            binary_trie_diff: vec![],
            binary_flat_diff: vec![],
        }
        .encode()
    }

    /// Seeds `STATE_HISTORY` with `(block_number, encoded_entry)` pairs.
    fn seed(backend: &dyn StorageBackend, entries: &[(u64, Vec<u8>)]) {
        let mut tx = backend.begin_write().unwrap();
        for (n, encoded) in entries {
            tx.put(STATE_HISTORY, &n.to_be_bytes(), encoded).unwrap();
        }
        tx.commit().unwrap();
    }

    /// The block numbers still present in `STATE_HISTORY`, ascending.
    fn present(backend: &dyn StorageBackend) -> Vec<u64> {
        let read = backend.begin_read().unwrap();
        let mut out: Vec<u64> = read
            .prefix_iterator(STATE_HISTORY, &[])
            .unwrap()
            .map(|item| {
                let (key, _) = item.unwrap();
                u64::from_be_bytes(<[u8; 8]>::try_from(key.as_ref()).unwrap())
            })
            .collect();
        out.sort_unstable();
        out
    }

    fn backend() -> crate::backend::in_memory::InMemoryBackend {
        crate::backend::in_memory::InMemoryBackend::open().unwrap()
    }

    /// First boot after the codec bump: every entry on disk is stale, so the
    /// whole journal goes. The floor `compute_reorg_ceiling` reads then reports
    /// no journal reach at all, which is the truth.
    #[test]
    fn drain_removes_an_all_stale_journal() {
        let backend = backend();
        seed(
            &backend,
            &[
                (10, encode_v1_entry(10)),
                (11, encode_v1_entry(11)),
                (12, encode_v1_entry(12)),
            ],
        );

        let report = drain_stale_journal_entries(&backend).unwrap().unwrap();
        assert_eq!(report.drained, 3);
        assert_eq!(report.range, Some((10, 12)));
        assert_eq!(report.versions, vec![1]);
        assert!(present(&backend).is_empty(), "the journal must be empty");
    }

    /// A node restarting again mid-window: the stale bottom goes, the entries it
    /// has since written in the current format stay, and the new floor is the
    /// lowest surviving entry. Wiping unconditionally would pass the "no stale
    /// entries" bar while throwing away reorg depth the node genuinely has.
    #[test]
    fn drain_keeps_the_current_version_portion_of_a_mixed_journal() {
        let backend = backend();
        seed(
            &backend,
            &[
                (10, encode_v1_entry(10)),
                (11, encode_v1_entry(11)),
                (12, encode_current_entry(12)),
                (13, encode_current_entry(13)),
            ],
        );

        let report = drain_stale_journal_entries(&backend).unwrap().unwrap();
        assert_eq!(report.drained, 2);
        assert_eq!(report.range, Some((10, 11)));
        assert_eq!(report.versions, vec![1]);
        assert_eq!(
            present(&backend),
            vec![12, 13],
            "the current-version portion must survive, and it sets the new floor"
        );
        // Every survivor must actually decode — the point of the drain is that
        // nothing left behind can fail mid-reorg.
        let read = backend.begin_read().unwrap();
        for n in [12u64, 13] {
            let bytes = read.get(STATE_HISTORY, &n.to_be_bytes()).unwrap().unwrap();
            JournalEntry::decode(&bytes).expect("surviving entry must decode");
        }
    }

    /// The steady state: nothing stale, nothing touched, and no write at all.
    #[test]
    fn drain_leaves_an_all_current_journal_untouched() {
        let backend = backend();
        seed(
            &backend,
            &[
                (10, encode_current_entry(10)),
                (11, encode_current_entry(11)),
            ],
        );

        assert_eq!(drain_stale_journal_entries(&backend).unwrap(), None);
        assert_eq!(present(&backend), vec![10, 11]);
    }

    /// A fresh datadir, or a journal fully pruned by finality. Nothing to do.
    #[test]
    fn drain_on_an_empty_journal_is_a_no_op() {
        let backend = backend();
        assert_eq!(drain_stale_journal_entries(&backend).unwrap(), None);
        assert!(present(&backend).is_empty());
    }

    /// The run is *contiguous from the bottom*, not "every stale entry". A stale
    /// entry sitting above a current-version one is left alone: the scan stops at
    /// the first current-version entry.
    ///
    /// No writer produces this interleaving, which is exactly why it is the test
    /// that separates the two candidate semantics — "delete the contiguous bottom
    /// run" and "delete everything stale" agree on every shape a real node can
    /// reach, and disagree only here.
    #[test]
    fn drain_stops_at_the_first_current_version_entry() {
        let backend = backend();
        seed(
            &backend,
            &[
                (10, encode_v1_entry(10)),
                (11, encode_current_entry(11)),
                (12, encode_v1_entry(12)),
            ],
        );

        let report = drain_stale_journal_entries(&backend).unwrap().unwrap();
        assert_eq!(report.drained, 1);
        assert_eq!(report.range, Some((10, 10)));
        assert_eq!(
            present(&backend),
            vec![11, 12],
            "the scan stops at 11; the stale entry above it is not the drain's business"
        );
    }

    /// An entry with no bytes at all cannot carry a version byte. It is not
    /// current-version, so it drains with the rest of the bottom run rather than
    /// stopping the scan and pinning the floor to a record nothing can decode.
    #[test]
    fn drain_treats_a_versionless_entry_as_stale() {
        let backend = backend();
        seed(
            &backend,
            &[
                (10, Vec::new()),
                (11, encode_v1_entry(11)),
                (12, encode_current_entry(12)),
            ],
        );

        let report = drain_stale_journal_entries(&backend).unwrap().unwrap();
        assert_eq!(report.drained, 2);
        assert_eq!(report.range, Some((10, 11)));
        assert_eq!(
            report.versions,
            vec![1],
            "the empty record has no version byte to report, but it is still drained"
        );
        assert_eq!(present(&backend), vec![12]);
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

    /// The same invariant one level up: `encode`'s `Vec::with_capacity(approx)`
    /// must be exact for the *whole entry*, not just one section.
    ///
    /// The section-level test above cannot see a missing term in `approx` —
    /// dropping one section's `diff_byte_estimate` leaves every section's own
    /// estimate correct and only makes the entry realloc on the hot path, which
    /// nothing observes. `with_capacity` allocates exactly, and the encoder only
    /// ever appends, so `capacity() == len()` holds if and only if every section
    /// is accounted for. Adding a seventh section without its term fails here.
    #[test]
    fn encode_capacity_is_exact_for_every_section() {
        let entry = JournalEntry {
            block_hash: h(0x11),
            parent_state_root: h(0x22),
            parent_binary_root: h(0x33),
            account_trie_diff: vec![(vec![0x01; 3], Some(vec![0xa1; 4]))],
            storage_trie_diff: vec![(vec![0x02; 67], None)],
            account_flat_diff: vec![(vec![0x03; 65], Some(vec![0xa3; 200]))],
            storage_flat_diff: vec![(vec![0x04; 131], Some(vec![0xa4; 5]))],
            binary_trie_diff: vec![(vec![0x05; 34], Some(vec![0xa5; 9]))],
            binary_flat_diff: vec![
                (vec![0x06; 34], Some(vec![0xa6; 32])),
                (vec![0x07; 66], None),
            ],
        };
        let encoded = entry.encode();
        assert_eq!(
            encoded.capacity(),
            encoded.len(),
            "encode()'s capacity hint must cover every section exactly; a \
             mismatch means a section's diff_byte_estimate term is missing"
        );
    }
}
