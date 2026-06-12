//! Inverted log index (address -> blocks) for `eth_getLogs`.
//!
//! The per-block header `logs_bloom` is saturated on mainnet for ubiquitous
//! signatures (e.g. ERC-20 `Transfer`), so it barely narrows a query — most
//! blocks survive the bloom and their receipts get read anyway. This index
//! instead records, exactly, which blocks each log *address* appears in, so a
//! query for `address` visits only the blocks that truly contain its logs.
//!
//! Layout: blocks are grouped into fixed-size [`SECTION_SIZE`]-block sections.
//! For each `(address, section)` we store the sorted block offsets (within the
//! section) where that address emitted at least one log. Keys are
//! `address (20 bytes) || section (8 bytes, big-endian)`; values are the
//! offsets encoded as big-endian `u16`s (a section has <= 4096 blocks, so an
//! offset always fits in a `u16`).
//!
//! The index is exact (built from receipts, not blooms), so it has no false
//! positives; the caller still exact-filters topics on the returned blocks.
//!
//! It is populated by a background indexer over buried/finalized sections only
//! (never on the block-import path), so it is append-only and never needs reorg
//! invalidation.

use ethrex_common::{Address, types::BlockNumber};

/// Number of blocks per index section.
pub const SECTION_SIZE: u64 = 4096;

/// Section that a block number falls into.
pub fn section_of(block_number: BlockNumber) -> u64 {
    block_number / SECTION_SIZE
}

/// Offset of a block within its section (`0..SECTION_SIZE`).
pub fn offset_in_section(block_number: BlockNumber) -> u16 {
    (block_number % SECTION_SIZE) as u16
}

/// Storage key for an `(address, section)` entry.
pub fn index_key(address: &Address, section: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(20 + 8);
    key.extend_from_slice(address.as_bytes());
    key.extend_from_slice(&section.to_be_bytes());
    key
}

/// Encodes a sorted, deduplicated slice of in-section offsets as big-endian `u16`s.
pub fn encode_offsets(offsets: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(offsets.len() * 2);
    for off in offsets {
        out.extend_from_slice(&off.to_be_bytes());
    }
    out
}

/// Decodes a value written by [`encode_offsets`] back into offsets.
pub fn decode_offsets(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect()
}

/// Given, for one section, the in-section offsets each address logged in,
/// produces the `(key, value)` entries to store: one per address, with
/// sorted-unique offsets.
pub fn build_section_entries(
    section: u64,
    address_offsets: impl IntoIterator<Item = (Address, Vec<u16>)>,
) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut entries = Vec::new();
    for (address, mut offsets) in address_offsets {
        offsets.sort_unstable();
        offsets.dedup();
        entries.push((index_key(&address, section), encode_offsets(&offsets)));
    }
    entries
}

/// Maps stored offsets back to absolute block numbers within `[from, to]`.
pub fn offsets_to_blocks(
    section: u64,
    offsets: &[u16],
    from: BlockNumber,
    to: BlockNumber,
) -> impl Iterator<Item = BlockNumber> + '_ {
    let base = section * SECTION_SIZE;
    offsets
        .iter()
        .map(move |&off| base + off as u64)
        .filter(move |bn| (from..=to).contains(bn))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(n: u64) -> Address {
        Address::from_low_u64_be(n)
    }

    #[test]
    fn section_and_offset_roundtrip() {
        let bn = 5 * SECTION_SIZE + 123;
        assert_eq!(section_of(bn), 5);
        assert_eq!(offset_in_section(bn), 123);
        assert_eq!(
            section_of(bn) * SECTION_SIZE + offset_in_section(bn) as u64,
            bn
        );
    }

    #[test]
    fn key_is_address_then_section_be() {
        let key = index_key(&addr(1), 7);
        assert_eq!(key.len(), 28);
        assert_eq!(&key[..20], addr(1).as_bytes());
        assert_eq!(&key[20..], &7u64.to_be_bytes());
    }

    #[test]
    fn offsets_encode_decode_roundtrip() {
        let offsets = vec![0u16, 1, 42, 4095];
        assert_eq!(decode_offsets(&encode_offsets(&offsets)), offsets);
        assert!(decode_offsets(&[]).is_empty());
    }

    #[test]
    fn build_section_sorts_and_dedups() {
        let entries = build_section_entries(3, [(addr(9), vec![10, 1, 10, 5])]);
        assert_eq!(entries.len(), 1);
        let (key, val) = &entries[0];
        assert_eq!(key, &index_key(&addr(9), 3));
        assert_eq!(decode_offsets(val), vec![1, 5, 10]);
    }

    #[test]
    fn offsets_map_to_blocks_within_range() {
        // section 2 -> base 8192; offsets 0, 100, 4095 -> blocks 8192, 8292, 12287.
        let blocks: Vec<_> = offsets_to_blocks(2, &[0, 100, 4095], 8200, 12000).collect();
        // 8192 (<8200) and 12287 (>12000) excluded; only 8292 is in range.
        assert_eq!(blocks, vec![8292]);
    }
}
