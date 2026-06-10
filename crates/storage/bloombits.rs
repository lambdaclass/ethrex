//! Bloombits log index (geth-style).
//!
//! `eth_getLogs` is expensive because it scans every block in the requested
//! range. Every block header already carries a 2048-bit `logs_bloom` over the
//! `(address, topic)` pairs logged in that block, so we can answer "which
//! blocks *might* match this filter?" without touching bodies or receipts.
//!
//! A linear walk over those header blooms is still O(range) (see #6813). The
//! bloombits index removes that ceiling by *transposing* the blooms: blocks are
//! grouped into fixed-size sections of [`SECTION_SIZE`] blocks, and for each of
//! the 2048 bloom bit positions we store one bit-vector across the section
//! (bit `j` set ⇔ the j-th block of the section has that bloom bit set).
//!
//! To find candidate blocks for an item (address or topic) we look at the three
//! bloom bits it maps to and AND the corresponding bit-vectors: a block is a
//! candidate only where all three bits are set. Items are combined the same way
//! the log filter combines them — OR within an address set or a topic position,
//! AND across positions. The result is a candidate bitmap that lets the query
//! skip straight to the handful of blocks worth reading receipts for.
//!
//! Bloom false positives are harmless: the exact filter still runs on every
//! candidate, so the index never drops a matching log, it only occasionally
//! nominates a block that turns out not to match.

use ethrex_common::Bloom;
use ethrex_common::utils::keccak;

/// Number of blocks grouped into a single bloombits section. Matches geth's
/// value; a section's transposed bit-vectors are this many bits wide.
pub const SECTION_SIZE: u64 = 4096;

/// Length of an Ethereum bloom filter in bits.
const BLOOM_BIT_LENGTH: usize = 2048;

/// Number of bloom bits each logged item (address/topic) sets.
const BITS_PER_ITEM: usize = 3;

/// Bytes needed to hold one section's worth of per-block bits.
pub const SECTION_BYTES: usize = SECTION_SIZE as usize / 8;

/// The three bloom bit positions a single item (address or topic) maps to.
///
/// Mirrors `ethbloom`'s `accrue`: the item is keccak256-hashed, then three
/// big-endian `u16`s are taken from the first six bytes of the hash, each masked
/// into the 2048-bit space. This is exactly how a block's `logs_bloom` is built,
/// so an item is present in a bloom iff all three of these bits are set.
pub fn bloom_bit_indices(item: &[u8]) -> [usize; BITS_PER_ITEM] {
    let hash = keccak(item);
    let bytes = hash.as_bytes();
    let mut indices = [0usize; BITS_PER_ITEM];
    for (i, index) in indices.iter_mut().enumerate() {
        let hi = bytes[2 * i] as usize;
        let lo = bytes[2 * i + 1] as usize;
        *index = ((hi << 8) | lo) & (BLOOM_BIT_LENGTH - 1);
    }
    indices
}

/// Returns the byte position within the transposed row where the bit for the
/// `block_offset`-th block of a section lives, plus the in-byte mask.
#[inline]
fn block_bit(block_offset: usize) -> (usize, u8) {
    (block_offset / 8, 1 << (block_offset % 8))
}

/// Transposes a section's block blooms into [`BLOOM_BIT_LENGTH`] bit-rows.
///
/// `blooms[j]` must be the `logs_bloom` of the j-th block of the section, in
/// ascending block order. The returned `rows[b]` is a [`SECTION_BYTES`]-long
/// bit-vector whose bit `j` is set iff `blooms[j]` has bloom bit `b` set.
///
/// Rows are allocated for every bit position; callers may drop all-zero rows
/// when persisting (their absence is read back as zero).
pub fn transpose_section(blooms: &[Bloom]) -> Vec<Vec<u8>> {
    debug_assert!(blooms.len() <= SECTION_SIZE as usize);
    let mut rows = vec![vec![0u8; SECTION_BYTES]; BLOOM_BIT_LENGTH];
    for (offset, bloom) in blooms.iter().enumerate() {
        let (byte, mask) = block_bit(offset);
        // Walk only the set bits of this bloom rather than all 2048 positions.
        for (bloom_byte_pos, &bloom_byte) in bloom.as_bytes().iter().enumerate() {
            if bloom_byte == 0 {
                continue;
            }
            for bit_in_byte in 0..8 {
                if (bloom_byte >> bit_in_byte) & 1 == 1 {
                    // Inverse of ethbloom's storage layout: bit `b` lives at byte
                    // `255 - b/8`, bit `b % 8`.
                    let bit = (255 - bloom_byte_pos) * 8 + bit_in_byte;
                    rows[bit][byte] |= mask;
                }
            }
        }
    }
    rows
}

/// A filter reduced to bloom-bit terms, ready to query a section.
///
/// Each term is the OR-set of one constrained filter position: the address set,
/// or one topic position. Within a term any item may match (OR); across terms
/// every term must match (AND) — exactly the log-filter semantics. Wildcard
/// positions contribute no term. A query with no terms constrains nothing.
#[derive(Debug, Clone, Default)]
pub struct BloomQuery {
    terms: Vec<Vec<[usize; BITS_PER_ITEM]>>,
}

impl BloomQuery {
    /// Builds a query from the filter's addresses and per-position topic sets.
    ///
    /// `addresses` is the address filter (empty ⇒ no address constraint).
    /// `topics` holds one entry per topic position; an empty entry is a wildcard
    /// for that position and contributes no constraint.
    pub fn new<'a>(
        addresses: impl IntoIterator<Item = &'a [u8]>,
        topics: impl IntoIterator<Item = &'a [&'a [u8]]>,
    ) -> Self {
        let mut terms = Vec::new();
        let address_term: Vec<_> = addresses.into_iter().map(bloom_bit_indices).collect();
        if !address_term.is_empty() {
            terms.push(address_term);
        }
        for position in topics {
            if position.is_empty() {
                continue;
            }
            terms.push(position.iter().copied().map(bloom_bit_indices).collect());
        }
        Self { terms }
    }

    /// Whether this query has any constraint at all. An unconstrained query
    /// can't narrow the candidate set, so callers should just scan the range.
    pub fn is_unconstrained(&self) -> bool {
        self.terms.is_empty()
    }

    /// Every distinct bloom bit position this query needs to read.
    pub fn required_bits(&self) -> impl Iterator<Item = usize> + '_ {
        self.terms
            .iter()
            .flatten()
            .flat_map(|item| item.iter().copied())
    }

    /// Runs the query against a single section, returning the candidate bitmap.
    ///
    /// `section_len` is the number of blocks actually present in the section
    /// (≤ [`SECTION_SIZE`]; the final section may be short). `row(bit)` returns
    /// the transposed bit-vector for a bloom bit position, as persisted by
    /// [`transpose_section`]; an all-zero row may be returned as an empty slice.
    ///
    /// The returned bitmap has bit `j` set iff the j-th block of the section is
    /// a candidate. Bits at or beyond `section_len` are always zero.
    pub fn run_section<F>(&self, section_len: usize, mut row: F) -> Vec<u8>
    where
        F: FnMut(usize) -> Vec<u8>,
    {
        // No constraints ⇒ every present block is a candidate.
        if self.terms.is_empty() {
            return full_bitmap(section_len);
        }

        let mut candidates = full_bitmap(section_len);
        for term in &self.terms {
            let mut term_bitmap = vec![0u8; SECTION_BYTES];
            for item in term {
                // A block has the item iff all three of its bloom bits are set:
                // AND the three rows together.
                let mut item_bitmap: Option<Vec<u8>> = None;
                for &bit in item {
                    let r = row(bit);
                    match &mut item_bitmap {
                        None => item_bitmap = Some(normalize_row(r)),
                        Some(acc) => and_into(acc, &r),
                    }
                }
                if let Some(item_bitmap) = item_bitmap {
                    or_into(&mut term_bitmap, &item_bitmap);
                }
            }
            and_into(&mut candidates, &term_bitmap);
        }
        candidates
    }
}

/// A bitmap with the low `len` bits set (the blocks present in a section).
fn full_bitmap(len: usize) -> Vec<u8> {
    let mut bitmap = vec![0u8; SECTION_BYTES];
    for offset in 0..len {
        let (byte, mask) = block_bit(offset);
        bitmap[byte] |= mask;
    }
    bitmap
}

/// Pads a persisted row (which may be empty or short for an all-zero tail) to
/// [`SECTION_BYTES`].
fn normalize_row(mut row: Vec<u8>) -> Vec<u8> {
    row.resize(SECTION_BYTES, 0);
    row
}

/// `acc &= other`, treating a short/empty `other` as zero-padded.
fn and_into(acc: &mut [u8], other: &[u8]) {
    for (i, byte) in acc.iter_mut().enumerate() {
        *byte &= other.get(i).copied().unwrap_or(0);
    }
}

/// `acc |= other`, treating a short/empty `other` as zero-padded.
fn or_into(acc: &mut [u8], other: &[u8]) {
    for (i, byte) in other.iter().enumerate() {
        if i < acc.len() {
            acc[i] |= byte;
        }
    }
}

/// Composite `LOG_BLOOM_BITS` key for one transposed row:
/// `section (u64 BE) || bloom_bit (u16 BE)`. Section-major so a whole section's
/// rows sort contiguously and can be range-scanned by [`section_prefix`].
pub fn row_key(section: u64, bit: u16) -> Vec<u8> {
    let mut key = Vec::with_capacity(10);
    key.extend_from_slice(&section.to_be_bytes());
    key.extend_from_slice(&bit.to_be_bytes());
    key
}

/// Key prefix matching every row of `section`.
pub fn section_prefix(section: u64) -> [u8; 8] {
    section.to_be_bytes()
}

/// The section a block number falls in.
pub fn section_of(block_number: u64) -> u64 {
    block_number / SECTION_SIZE
}

/// Iterates the in-section block offsets whose bit is set in `bitmap`.
pub fn set_offsets(bitmap: &[u8]) -> impl Iterator<Item = usize> + '_ {
    bitmap.iter().enumerate().flat_map(|(byte_pos, &byte)| {
        (0..8usize).filter_map(move |bit| ((byte >> bit) & 1 == 1).then_some(byte_pos * 8 + bit))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::{H160, H256};

    /// Builds a bloom the way a block producer does: by accruing each item's
    /// keccak hash. Equivalent to setting the bits from `bloom_bit_indices`.
    fn bloom_with(items: &[&[u8]]) -> Bloom {
        use ethrex_common::BloomInput;
        let mut bloom = Bloom::zero();
        for item in items {
            bloom.accrue(BloomInput::Raw(item));
        }
        bloom
    }

    fn addr(n: u64) -> H160 {
        H160::from_low_u64_be(n)
    }

    fn topic(n: u64) -> H256 {
        H256::from_low_u64_be(n)
    }

    #[test]
    fn bit_indices_match_bloom_layout() {
        // The three bits an item maps to must be exactly the bits set in a bloom
        // built from that item alone.
        let item = addr(42);
        let bloom = bloom_with(&[item.as_bytes()]);
        for bit in bloom_bit_indices(item.as_bytes()) {
            let byte = bloom.as_bytes()[255 - bit / 8];
            assert!((byte >> (bit % 8)) & 1 == 1, "bit {bit} should be set");
        }
    }

    #[test]
    fn transpose_roundtrips_membership() {
        // Block 0 logs addr(1); block 1 logs addr(2); block 2 logs nothing.
        let blooms = vec![
            bloom_with(&[addr(1).as_bytes()]),
            bloom_with(&[addr(2).as_bytes()]),
            bloom_with(&[]),
        ];
        let rows = transpose_section(&blooms);

        // addr(1) present only in block 0.
        for bit in bloom_bit_indices(addr(1).as_bytes()) {
            let (byte, mask) = block_bit(0);
            assert!(rows[bit][byte] & mask != 0);
            let (byte1, mask1) = block_bit(1);
            assert!(rows[bit][byte1] & mask1 == 0);
        }
    }

    /// Convenience: query a set of blooms directly (transpose then run).
    fn run(blooms: &[Bloom], query: &BloomQuery) -> Vec<usize> {
        let rows = transpose_section(blooms);
        let bitmap = query.run_section(blooms.len(), |bit| rows[bit].clone());
        set_offsets(&bitmap).collect()
    }

    #[test]
    fn query_single_address() {
        let blooms = vec![
            bloom_with(&[addr(1).as_bytes()]),
            bloom_with(&[addr(2).as_bytes()]),
            bloom_with(&[addr(1).as_bytes(), addr(3).as_bytes()]),
        ];
        let query = BloomQuery::new([addr(1).as_bytes()], []);
        assert_eq!(run(&blooms, &query), vec![0, 2]);
    }

    #[test]
    fn query_multiple_addresses_is_or() {
        let blooms = vec![
            bloom_with(&[addr(1).as_bytes()]),
            bloom_with(&[addr(2).as_bytes()]),
            bloom_with(&[addr(9).as_bytes()]),
        ];
        let query = BloomQuery::new([addr(1).as_bytes(), addr(2).as_bytes()], []);
        assert_eq!(run(&blooms, &query), vec![0, 1]);
    }

    #[test]
    fn query_topic_positions_and_across_or_within() {
        // Block 0: t1 & t2. Block 1: t1 only. Block 2: t2 & t9.
        let blooms = vec![
            bloom_with(&[topic(1).as_bytes(), topic(2).as_bytes()]),
            bloom_with(&[topic(1).as_bytes()]),
            bloom_with(&[topic(2).as_bytes(), topic(9).as_bytes()]),
        ];
        // position0 = {t1}, position1 = {t2, t9}  → AND across, OR within.
        let (t1, t2, t9) = (topic(1), topic(2), topic(9));
        let pos0: Vec<&[u8]> = vec![t1.as_bytes()];
        let pos1: Vec<&[u8]> = vec![t2.as_bytes(), t9.as_bytes()];
        let query = BloomQuery::new([], [pos0.as_slice(), pos1.as_slice()]);
        // Block 0 has t1 and t2 ✓. Block 1 has t1 but neither t2 nor t9 ✗.
        // Block 2 has t2/t9 but not t1 ✗.
        assert_eq!(run(&blooms, &query), vec![0]);
    }

    #[test]
    fn query_address_and_topic_combined() {
        let blooms = vec![
            bloom_with(&[addr(1).as_bytes(), topic(1).as_bytes()]),
            bloom_with(&[addr(1).as_bytes()]),
            bloom_with(&[topic(1).as_bytes()]),
        ];
        let t1 = topic(1);
        let pos0: Vec<&[u8]> = vec![t1.as_bytes()];
        let query = BloomQuery::new([addr(1).as_bytes()], [pos0.as_slice()]);
        assert_eq!(run(&blooms, &query), vec![0]);
    }

    #[test]
    fn unconstrained_query_matches_all_present_blocks() {
        let blooms = vec![bloom_with(&[addr(1).as_bytes()]), bloom_with(&[])];
        let query = BloomQuery::new([], []);
        assert!(query.is_unconstrained());
        assert_eq!(run(&blooms, &query), vec![0, 1]);
    }

    #[test]
    fn empty_topic_position_is_wildcard() {
        let blooms = vec![
            bloom_with(&[addr(1).as_bytes()]),
            bloom_with(&[addr(2).as_bytes()]),
        ];
        // A wildcard topic position (empty set) imposes no constraint.
        let empty: Vec<&[u8]> = vec![];
        let query = BloomQuery::new([addr(1).as_bytes()], [empty.as_slice()]);
        assert_eq!(run(&blooms, &query), vec![0]);
    }

    #[test]
    fn no_false_negatives_against_brute_force() {
        // Cross-check the index against the bloom's own membership test over a
        // mixed section: every block the index nominates a *miss* for must
        // genuinely lack the item (no false negatives).
        let blooms: Vec<Bloom> = (0..50u64)
            .map(|i| bloom_with(&[addr(i % 7).as_bytes(), topic(i % 5).as_bytes()]))
            .collect();
        let query = BloomQuery::new([addr(3).as_bytes()], []);
        let rows = transpose_section(&blooms);
        let bitmap = query.run_section(blooms.len(), |bit| rows[bit].clone());
        let candidates: std::collections::HashSet<usize> = set_offsets(&bitmap).collect();
        for (j, bloom) in blooms.iter().enumerate() {
            use ethrex_common::BloomInput;
            let really_present = bloom.contains_input(BloomInput::Raw(addr(3).as_bytes()));
            if really_present {
                assert!(candidates.contains(&j), "block {j} wrongly skipped");
            }
        }
    }
}
