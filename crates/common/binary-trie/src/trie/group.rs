//! Groups: the unit of *storage*, one stored row per subtree rather
//! than per node.
//!
//! **Nothing here is consensus-visible.** A node's encoding is its
//! BLAKE3 preimage and is fixed by the spec; a group is a container
//! that carries those encodings verbatim. Grouping changes how nodes
//! are *stored*, never how they are encoded or hashed — see
//! [`GroupRow`] for the invariant that keeps that true by construction.
//!
//! A group is indexed by *depth*, never by size: the group containing a
//! node at bit-depth `d` starts at `d / g * g` for group depth `g`. So a
//! node's group is a function of its path alone, no group ever splits or
//! merges, and two nodes that belong together today belong together
//! after any sequence of insertions.

use crate::error::BinaryTrieError;

use super::path::BitPath;

/// Deepest group we will store in one row.
///
/// Matches go-ethereum's `MaxGroupDepth` (`trie/bintrie/binary_node.go`),
/// and for the same reason: a group of depth `g` holds up to `2^g - 1`
/// branches, so the member count stays inside one byte exactly while
/// `g <= 8`.
pub const MAX_GROUP_DEPTH: usize = 8;

/// Group depth a backend uses when nothing has told it otherwise.
///
/// **Not settled.** It is the current default, chosen on marginal cost
/// and on max row size against the 16 KiB block, and `g = 7` is a live
/// alternative — so nothing outside this constant may assume the value.
/// Every geometry function takes the depth as an argument and every test
/// that touches geometry sweeps 1..=[`MAX_GROUP_DEPTH`]; changing this
/// line must not change a single assertion.
pub const DEFAULT_GROUP_DEPTH: GroupDepth = GroupDepth(6);

/// How many levels of tree one stored row spans.
///
/// A validated newtype rather than a bare `usize` because every
/// geometry function below divides by it: zero is a panic and anything
/// past [`MAX_GROUP_DEPTH`] overflows the row format's one-byte member
/// count. go-ethereum takes the bare integer and `panic`s on a bad one
/// (`trie/bintrie/trie.go:139`); making the value unrepresentable is
/// cheaper than making it panic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupDepth(usize);

impl GroupDepth {
    /// A group depth, or `None` if `levels` is zero or past
    /// [`MAX_GROUP_DEPTH`].
    pub const fn new(levels: usize) -> Option<Self> {
        if levels == 0 || levels > MAX_GROUP_DEPTH {
            None
        } else {
            Some(Self(levels))
        }
    }

    pub const fn get(self) -> usize {
        self.0
    }

    /// The most members a group of this depth can hold: every node at
    /// relative depths `0..g`, which is a full binary tree of `2^g - 1`.
    ///
    /// A bound, not a count — path compression means real groups hold
    /// far fewer. It exists to size the row format's member count.
    pub const fn max_members(self) -> usize {
        (1usize << self.0) - 1
    }
}

/// Bit depth at which the group containing a node at `depth` begins.
pub const fn group_start(depth: usize, group_depth: GroupDepth) -> usize {
    depth / group_depth.0 * group_depth.0
}

/// The path of the row a node at `path` is stored in: `path` truncated
/// to the start of its group.
///
/// This is the *only* thing a storage key is derived from, so two nodes
/// share a row exactly when this agrees.
pub fn group_root(path: &BitPath, group_depth: GroupDepth) -> BitPath {
    BitPath::from_bits(&path.as_bits()[..group_start(path.len(), group_depth)])
}

/// The bits of `path` below its group root — where inside the row the
/// node sits.
///
/// Always shorter than `group_depth`: a node whose depth is a multiple
/// of the group depth starts a group of its own, with no relative bits
/// at all.
pub fn relative_bits(path: &BitPath, group_depth: GroupDepth) -> &[u8] {
    &path.as_bits()[group_start(path.len(), group_depth)..]
}

/// Database key for the row a node at `path` belongs to.
///
/// Deliberately [`BitPath::to_db_key`] of the group root and nothing
/// new: the key *shape* — four-byte big-endian bit count, then packed
/// bits — is unchanged, and only the set of bit counts that can appear
/// shrinks, from every depth to the multiples of `group_depth`. Key
/// lengths therefore form a subset of the lengths the table already
/// holds, which is what keeps the `classify_trie_key` length-collision
/// analysis in `crates/storage` valid rather than merely re-checked.
///
/// **Alternative: key by the group index (`depth / g`) in one byte, or
/// by the packed bits alone with no count.** Both are shorter — one to
/// three bytes per row — and both lost. A one-byte index changes every
/// key length by three, which moves the whole table across the length
/// boundaries `classify_trie_key` disambiguates by; dropping the count
/// entirely is not injective, for exactly the reason
/// [`BitPath::to_db_key`] documents, since group roots at different
/// depths still pack to the same bytes (`[1]` and `[1, 0]`).
pub fn group_db_key(path: &BitPath, group_depth: GroupDepth) -> Vec<u8> {
    group_root(path, group_depth).to_db_key()
}

/// Whether `path` is the root of its own group — the node a descent
/// arriving from the group above lands on.
pub fn starts_a_group(path: &BitPath, group_depth: GroupDepth) -> bool {
    path.len().is_multiple_of(group_depth.0)
}

/// A stored row: the encoded nodes of one group, in ascending
/// `(relative depth, relative bits)` order.
///
/// **The hard invariant.** A member's bytes are *byte-for-byte* what a
/// one-node-per-row layout would have stored — the node's BLAKE3
/// preimage, produced by `encode_leaf` / `encode_branch` and unchanged.
/// The row is a container around them. Nothing in this file can move a
/// root hash, because nothing in this file constructs a preimage.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroupRow {
    /// `(relative bits below the group root, encoded node)`, strictly
    /// ascending by `(len, bits)`.
    members: Vec<(Vec<u8>, Vec<u8>)>,
}

/// Row format version. One byte, first, so a datadir written by an
/// older layout is *recognisably* different rather than silently
/// misparsed.
const ROW_VERSION: u8 = 0x01;

/// Longest node encoding the trie can produce, and therefore the bound
/// that makes a two-byte member length sufficient.
///
/// A branch is `1 + (2 + ceil(bits / 8)) + 64` bytes and its prefix is
/// bounded by `MAX_KEY_LENGTH * 8` bits, giving `1 + 2 + 8192 + 64`.
/// A leaf is smaller: `1 + key + 32` with the key at most
/// `MAX_KEY_LENGTH`.
const MAX_NODE_LEN: usize = 1 + 2 + super::MAX_KEY_LENGTH + 64;

impl GroupRow {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node at `relative` bits below the group root.
    ///
    /// # Errors
    ///
    /// [`BinaryTrieError::MalformedNode`] if the member does not belong
    /// in this row — a relative path at or past the group depth, a
    /// duplicate, or an out-of-order insertion — or if the encoding is
    /// empty or longer than the format's length field.
    pub fn push(
        &mut self,
        relative: &[u8],
        encoded: Vec<u8>,
        group_depth: GroupDepth,
    ) -> Result<(), BinaryTrieError> {
        if relative.len() >= group_depth.get() {
            return Err(BinaryTrieError::MalformedNode(
                "group member sits at or below the next group's root",
            ));
        }
        if encoded.is_empty() || encoded.len() > MAX_NODE_LEN {
            return Err(BinaryTrieError::MalformedNode("group member size"));
        }
        if let Some((previous, _)) = self.members.last()
            && !member_precedes(previous, relative)
        {
            return Err(BinaryTrieError::MalformedNode("group members out of order"));
        }
        self.members.push((relative.to_vec(), encoded));
        Ok(())
    }

    pub fn members(&self) -> &[(Vec<u8>, Vec<u8>)] {
        &self.members
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The encoded node at `relative`, if this row holds one there.
    pub fn get(&self, relative: &[u8]) -> Option<&[u8]> {
        self.members
            .iter()
            .find(|(bits, _)| bits == relative)
            .map(|(_, encoded)| encoded.as_slice())
    }

    /// Serialize: `[version][member count][member...]`, each member
    /// `[relative bit count][packed bits][node length BE][node]`.
    ///
    /// Sparse by construction — absent members occupy nothing at all,
    /// so a one-node group costs its node plus five bytes rather than a
    /// slot in a fixed-size table.
    ///
    /// The relative bit count is a whole byte and the packed bits are
    /// derived from it, so a row decodes *without knowing the group
    /// depth it was written under*. That is what lets a datadir be read
    /// back and checked after the depth is reconfigured, instead of
    /// being silently reinterpreted.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![ROW_VERSION, self.members.len() as u8];
        for (relative, encoded) in &self.members {
            out.push(relative.len() as u8);
            let mut packed = vec![0u8; relative.len().div_ceil(8)];
            for (i, bit) in relative.iter().enumerate() {
                packed[i / 8] |= bit << (7 - i % 8);
            }
            out.extend_from_slice(&packed);
            out.extend_from_slice(&(encoded.len() as u16).to_be_bytes());
            out.extend_from_slice(encoded);
        }
        out
    }

    /// Parse a row written by [`GroupRow::encode`].
    ///
    /// Canonical: padding bits in a packed relative path must be zero,
    /// members must strictly ascend, the count must match, and no bytes
    /// may follow the last member. Each rejection closes a way for two
    /// byte strings to mean one row, which would otherwise let a
    /// database hold a row it could not have written.
    ///
    /// # Errors
    ///
    /// [`BinaryTrieError::MalformedNode`] on any of the above.
    pub fn decode(bytes: &[u8]) -> Result<Self, BinaryTrieError> {
        let (&version, rest) = bytes
            .split_first()
            .ok_or(BinaryTrieError::MalformedNode("empty group row"))?;
        if version != ROW_VERSION {
            return Err(BinaryTrieError::MalformedNode("unknown group row version"));
        }
        let (&count, mut rest) = rest
            .split_first()
            .ok_or(BinaryTrieError::MalformedNode("group row has no count"))?;
        let mut members: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let (&bit_count, tail) = rest
                .split_first()
                .ok_or(BinaryTrieError::MalformedNode("group member truncated"))?;
            let bit_count = bit_count as usize;
            if bit_count >= MAX_GROUP_DEPTH {
                return Err(BinaryTrieError::MalformedNode(
                    "group member relative path too long",
                ));
            }
            let packed_len = bit_count.div_ceil(8);
            let packed = tail
                .get(..packed_len)
                .ok_or(BinaryTrieError::MalformedNode("group member truncated"))?;
            let relative: Vec<u8> = (0..bit_count)
                .map(|i| (packed[i / 8] >> (7 - i % 8)) & 1)
                .collect();
            // Padding must be zero, or one relative path would have
            // several encodings.
            let padding = (8 - bit_count % 8) % 8;
            let padding_mask = if padding == 0 {
                0
            } else {
                (1u8 << padding) - 1
            };
            if packed.last().is_some_and(|byte| byte & padding_mask != 0) {
                return Err(BinaryTrieError::MalformedNode(
                    "group member path has non-zero padding",
                ));
            }
            let tail = &tail[packed_len..];
            let length = tail
                .get(..2)
                .ok_or(BinaryTrieError::MalformedNode("group member truncated"))?;
            let length = u16::from_be_bytes([length[0], length[1]]) as usize;
            if length == 0 {
                return Err(BinaryTrieError::MalformedNode("group member is empty"));
            }
            let encoded = tail
                .get(2..2 + length)
                .ok_or(BinaryTrieError::MalformedNode("group member truncated"))?;
            if let Some((previous, _)) = members.last()
                && !member_precedes(previous, &relative)
            {
                return Err(BinaryTrieError::MalformedNode("group members out of order"));
            }
            members.push((relative, encoded.to_vec()));
            rest = &tail[2 + length..];
        }
        if !rest.is_empty() {
            return Err(BinaryTrieError::MalformedNode("trailing bytes after group"));
        }
        Ok(Self { members })
    }
}

/// Row order: shallower first, then by bits.
///
/// Depth first rather than bits first so that a group's own root — the
/// zero-length relative path — is always the first member, which is the
/// node a descent entering the row wants.
fn member_precedes(previous: &[u8], next: &[u8]) -> bool {
    (previous.len(), previous) < (next.len(), next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depth(levels: usize) -> GroupDepth {
        GroupDepth::new(levels).expect("valid group depth")
    }

    #[test]
    fn group_depth_rejects_zero_and_overflow() {
        assert_eq!(GroupDepth::new(0), None);
        assert_eq!(GroupDepth::new(MAX_GROUP_DEPTH + 1), None);
        assert_eq!(GroupDepth::new(1).map(GroupDepth::get), Some(1));
        assert_eq!(
            GroupDepth::new(MAX_GROUP_DEPTH).map(GroupDepth::get),
            Some(MAX_GROUP_DEPTH)
        );
    }

    #[test]
    fn max_members_stays_inside_the_one_byte_count() {
        // The row format writes the member count in one byte, so the
        // bound has to hold at the deepest group we accept.
        assert_eq!(depth(1).max_members(), 1);
        assert_eq!(depth(5).max_members(), 31);
        assert_eq!(depth(MAX_GROUP_DEPTH).max_members(), 255);
        assert!(depth(MAX_GROUP_DEPTH).max_members() <= u8::MAX as usize);
    }

    #[test]
    fn a_group_starts_at_a_multiple_of_the_depth() {
        let g = depth(5);
        assert_eq!(group_start(0, g), 0);
        assert_eq!(group_start(4, g), 0);
        assert_eq!(group_start(5, g), 5);
        assert_eq!(group_start(9, g), 5);
        assert_eq!(group_start(10, g), 10);
        // Depth 1 is today's layout: every node starts its own group.
        assert_eq!(group_start(7, depth(1)), 7);
    }

    #[test]
    fn group_root_truncates_and_relative_bits_are_the_remainder() {
        let g = depth(5);
        let path = BitPath::from_bits(&[1, 0, 1, 1, 0, 1, 0]);
        assert_eq!(group_root(&path, g), BitPath::from_bits(&[1, 0, 1, 1, 0]));
        assert_eq!(relative_bits(&path, g), &[1, 0]);
        // Together they reconstruct the path, which is what makes the
        // pair a lossless address.
        let mut rebuilt = group_root(&path, g).as_bits().to_vec();
        rebuilt.extend_from_slice(relative_bits(&path, g));
        assert_eq!(rebuilt, path.as_bits());
    }

    #[test]
    fn a_node_at_a_multiple_of_the_depth_starts_its_own_group() {
        let g = depth(5);
        let path = BitPath::from_bits(&[1, 0, 1, 1, 0]);
        assert!(starts_a_group(&path, g));
        assert_eq!(group_root(&path, g), path);
        assert!(relative_bits(&path, g).is_empty());
        assert!(!starts_a_group(&BitPath::from_bits(&[1, 0, 1, 1, 0, 1]), g));
        assert!(starts_a_group(&BitPath::new(), g));
    }

    #[test]
    fn the_db_key_is_the_group_roots_key_worked_from_bytes() {
        let g = depth(5);
        // Root group: no bits, so the four-byte count and nothing else.
        assert_eq!(
            group_db_key(&BitPath::from_bits(&[1, 0, 1, 1]), g),
            vec![0x00, 0x00, 0x00, 0x00]
        );
        // Depth 7 under group depth 5: group root is the first five
        // bits, 1 0 1 1 0, packed MSB-first into 0b1011_0000 = 0xb0,
        // behind a count of 5.
        assert_eq!(
            group_db_key(&BitPath::from_bits(&[1, 0, 1, 1, 0, 1, 0]), g),
            vec![0x00, 0x00, 0x00, 0x05, 0xb0]
        );
        // Every node in a group agrees on the key, which is the point.
        assert_eq!(
            group_db_key(&BitPath::from_bits(&[1, 0, 1, 1, 0]), g),
            group_db_key(&BitPath::from_bits(&[1, 0, 1, 1, 0, 1, 1, 0, 1]), g)
        );
    }

    #[test]
    fn key_lengths_are_a_subset_of_todays() {
        // The property the `classify_trie_key` collision analysis rests
        // on: grouping introduces no key length the one-node-per-row
        // table did not already produce. Checked against lengths
        // computed independently — `4 + ceil(bits / 8)` written out
        // here — rather than against another call to the same geometry,
        // which would agree with itself under any mutation.
        let ungrouped: std::collections::BTreeSet<usize> =
            (0..600usize).map(|bits| 4 + bits.div_ceil(8)).collect();
        for levels in 1..=MAX_GROUP_DEPTH {
            let g = depth(levels);
            for bits in 0..600usize {
                let grouped = group_db_key(&BitPath::from_bits(&vec![1u8; bits]), g).len();
                let group_bits = bits - bits % levels;
                assert_eq!(
                    grouped,
                    4 + group_bits.div_ceil(8),
                    "g={levels}, {bits} bits"
                );
                assert!(ungrouped.contains(&grouped), "g={levels}, {bits} bits");
            }
            // The two lengths the collision analysis names, at *every*
            // group depth: 34 bytes at bit-depth 240 and 66 at 496 are
            // byte-for-byte the account-zone and storage-zone tree-key
            // lengths. Grouping removes neither collision at any depth,
            // so no positional-routing argument in `crates/storage` may
            // be relaxed on the strength of it.
            assert!(
                (0..=600)
                    .any(|bits| group_db_key(&BitPath::from_bits(&vec![1u8; bits]), g).len() == 34),
                "g={levels} still produces a 34-byte key"
            );
            assert!(
                (0..=600)
                    .any(|bits| group_db_key(&BitPath::from_bits(&vec![1u8; bits]), g).len() == 66),
                "g={levels} still produces a 66-byte key"
            );
        }
    }

    #[test]
    fn decode_rejects_a_zero_length_member() {
        // `push` refuses an empty encoding, so this row can only be
        // hand-built — but a database could hold one, and accepting it
        // would put a node with no bytes into the tree, which then
        // fails to decode far away from here.
        let mut bytes = vec![ROW_VERSION, 1, 0];
        bytes.extend_from_slice(&0u16.to_be_bytes());
        assert!(GroupRow::decode(&bytes).is_err());
    }

    #[test]
    fn a_row_round_trips() {
        let g = depth(5);
        let mut row = GroupRow::new();
        row.push(&[], vec![0x01, 0xaa], g).unwrap();
        row.push(&[0], vec![0x00, 0xbb, 0xcc], g).unwrap();
        row.push(&[1, 0, 1, 1], vec![0x01; 70], g).unwrap();
        let encoded = row.encode();
        assert_eq!(GroupRow::decode(&encoded).unwrap(), row);
        assert_eq!(row.get(&[0]), Some(&[0x00, 0xbb, 0xcc][..]));
        assert_eq!(row.get(&[1]), None);
    }

    #[test]
    fn a_one_member_row_is_small() {
        // Sparsity is the whole reason a group is affordable: an almost
        // empty group costs its one node plus a fixed five bytes, not a
        // slot per absent child.
        let g = depth(8);
        let mut row = GroupRow::new();
        row.push(&[], vec![0xab; 67], g).unwrap();
        // version + count + relative bit count + (no packed bits, the
        // group root has none) + two-byte length + the node itself.
        assert_eq!(row.encode().len(), 2 + 1 + 2 + 67);
    }

    #[test]
    fn members_carry_node_bytes_verbatim() {
        // The hard constraint: a group is a container. Whatever a
        // one-node-per-row layout would have stored is what comes back
        // out, byte for byte, so the hashing preimage cannot move.
        let g = depth(4);
        let node = super::super::node::encode_branch(
            &[1, 0, 1],
            ethereum_types::H256([7u8; 32]),
            ethereum_types::H256([9u8; 32]),
        );
        let mut row = GroupRow::new();
        row.push(&[1, 1], node.clone(), g).unwrap();
        let decoded = GroupRow::decode(&row.encode()).unwrap();
        assert_eq!(decoded.get(&[1, 1]), Some(node.as_slice()));
        assert_eq!(
            super::super::node::blake3_hash(decoded.get(&[1, 1]).unwrap()),
            super::super::node::blake3_hash(&node)
        );
    }

    #[test]
    fn push_rejects_members_that_do_not_belong() {
        let g = depth(3);
        let mut row = GroupRow::new();
        // At the group depth: this node is the *next* group's root.
        assert!(row.push(&[0, 0, 0], vec![0x01], g).is_err());
        assert!(row.push(&[], Vec::new(), g).is_err());
        row.push(&[0, 1], vec![0x01], g).unwrap();
        // Out of order, and a duplicate, are both refused.
        assert!(row.push(&[0, 0], vec![0x01], g).is_err());
        assert!(row.push(&[0, 1], vec![0x01], g).is_err());
        row.push(&[1, 1], vec![0x01], g).unwrap();
    }

    #[test]
    fn decode_rejects_malformed_rows() {
        let g = depth(5);
        let mut row = GroupRow::new();
        row.push(&[], vec![0x01, 0xaa], g).unwrap();
        row.push(&[1, 0], vec![0x02, 0xbb], g).unwrap();
        let good = row.encode();

        assert!(GroupRow::decode(&[]).is_err(), "empty");
        let mut wrong_version = good.clone();
        wrong_version[0] = 0x02;
        assert!(GroupRow::decode(&wrong_version).is_err(), "version");
        assert!(GroupRow::decode(&good[..1]).is_err(), "no count");
        assert!(
            GroupRow::decode(&good[..good.len() - 1]).is_err(),
            "truncated member"
        );

        // A count that under-reads leaves bytes over.
        let mut short_count = good.clone();
        short_count[1] = 1;
        assert!(GroupRow::decode(&short_count).is_err(), "trailing bytes");

        // A count that over-reads runs out.
        let mut long_count = good;
        long_count[1] = 3;
        assert!(GroupRow::decode(&long_count).is_err(), "count too high");
    }

    #[test]
    fn decode_rejects_non_canonical_padding() {
        // Two bits declared, but a padding bit set in the packed byte.
        // The encoder never produces this, and accepting it would give
        // one member two encodings.
        let g = depth(5);
        let mut row = GroupRow::new();
        row.push(&[1, 0], vec![0x02, 0xbb], g).unwrap();
        let mut encoded = row.encode();
        // [version][count][bit count][packed] -> index 3 is the packed byte.
        assert_eq!(encoded[2], 2, "member declares two relative bits");
        encoded[3] |= 0b0000_0001;
        assert!(GroupRow::decode(&encoded).is_err());
    }

    #[test]
    fn decode_rejects_out_of_order_members() {
        // Hand-built, because `push` will not produce it: two members
        // written deepest-first. Accepting it would let one group have
        // several byte encodings.
        let mut bytes = vec![ROW_VERSION, 2];
        for (relative, node) in [(vec![1u8, 0u8], vec![0x02u8]), (vec![0u8], vec![0x01u8])] {
            bytes.push(relative.len() as u8);
            let mut packed = vec![0u8; relative.len().div_ceil(8)];
            for (i, bit) in relative.iter().enumerate() {
                packed[i / 8] |= bit << (7 - i % 8);
            }
            bytes.extend_from_slice(&packed);
            bytes.extend_from_slice(&(node.len() as u16).to_be_bytes());
            bytes.extend_from_slice(&node);
        }
        assert!(GroupRow::decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_a_relative_path_past_the_deepest_group() {
        // A row is decoded without knowing its group depth, so the only
        // bound available is `MAX_GROUP_DEPTH`. Past it, the member
        // cannot belong to any group this crate writes.
        let mut bytes = vec![ROW_VERSION, 1, MAX_GROUP_DEPTH as u8];
        bytes.push(0x00);
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.push(0x01);
        assert!(GroupRow::decode(&bytes).is_err());
    }
}
