//! The ordering property a flat key-value mirror of this tree rests on:
//! **bytewise tree-key order is leaf order**.
//!
//! A column family sorted by the raw tree key is an ordered leaf index by
//! construction — but only because sorting keys as bytes produces the same
//! sequence as walking the tree left-then-right. That holds for two reasons,
//! and both are pinned here:
//!
//! - Keys are expanded to bits **MSB-first**, so a key's bit path is its bytes
//!   read most-significant bit first, and bytewise order is bitwise order.
//! - The key set is **prefix-free**, so the two orders never have to compare a
//!   key against something it is a prefix of, which is the only case where byte
//!   order and bit order disagree.
//!
//! Neither is checked by anything else. `bytes_to_bits` could be rewritten
//! LSB-first and every existing test — roots, round trips, spec vectors — would
//! still pass, because they all agree with each other about which bit is first.
//! The mirror would then be sorted by a key order that is not leaf order, and
//! every range it served would be wrong.
//!
//! The direct statement of the property — a depth-first leaf walk compared
//! against `Vec::sort` — needs to see the tree's node structure, which is not
//! public, so it lives in the crate's own unit tests
//! (`bytewise_key_order_is_leaf_order`). What is asserted here is the half that
//! is observable from outside: the embedding's prefix-freeness, and the
//! structural consequence that a *bottom-up* build over byte-sorted leaves
//! reaches the same tree as inserting them one at a time by bit path.

use ethereum_types::{H160, U256};
use ethrex_binary_trie::embedding::{
    ACCOUNT_KEY_LENGTH, ACCOUNT_ZONE, CODE_KEY_LENGTH, CODE_ZONE, STORAGE_KEY_LENGTH, STORAGE_ZONE,
    address20_to_address32, get_tree_key_for_basic_data, get_tree_key_for_code_chunk,
    get_tree_key_for_code_hash, get_tree_key_for_delegation, get_tree_key_for_storage_slot,
};
use ethrex_binary_trie::trie::{BinaryTrie, InMemoryBinaryTrieDB};
use proptest::prelude::*;

/// Every key the embedding derives for one account, spanning all three zones:
/// its header leaves, storage inside the header stem and past it in the
/// overflow zone, and the content-addressed chunks of its code.
fn keys_for(address: H160, slots: &[U256], code_hash: [u8; 32], chunks: &[u64]) -> Vec<Vec<u8>> {
    let address = address20_to_address32(address);
    let mut keys = vec![
        get_tree_key_for_basic_data(&address),
        get_tree_key_for_code_hash(&address),
        get_tree_key_for_delegation(&address),
    ];
    keys.extend(
        slots
            .iter()
            .map(|slot| get_tree_key_for_storage_slot(&address, *slot)),
    );
    keys.extend(
        chunks
            .iter()
            .map(|chunk| get_tree_key_for_code_chunk(&code_hash, *chunk)),
    );
    keys
}

/// The zone byte determines the key length, which is what keeps the whole key
/// set prefix-free across zones as well as within one.
fn assert_zone_shape(key: &[u8]) {
    let expected = match key[0] {
        ACCOUNT_ZONE => ACCOUNT_KEY_LENGTH,
        CODE_ZONE => CODE_KEY_LENGTH,
        STORAGE_ZONE => STORAGE_KEY_LENGTH,
        other => panic!("key in an unallocated zone {other}"),
    };
    assert_eq!(key.len(), expected, "zone {} key: {key:?}", key[0]);
}

/// An address, a spread of storage slots on both sides of the
/// header/overflow boundary, a code hash, and chunk ids on both sides of a
/// code-group boundary.
fn account_strategy() -> impl Strategy<Value = (H160, Vec<U256>, [u8; 32], Vec<u64>)> {
    (
        any::<[u8; 20]>(),
        proptest::collection::vec(any::<u64>(), 1..6),
        any::<[u8; 32]>(),
        proptest::collection::vec(0u64..1024, 1..4),
    )
        .prop_map(|(address, raw_slots, code_hash, chunks)| {
            // Half the slots forced below `HEADER_STORAGE_SLOTS` so the header
            // stem is exercised as well as the 66-byte overflow zone; the rest
            // left arbitrary across the whole `U256` range.
            let slots = raw_slots
                .into_iter()
                .enumerate()
                .map(|(i, raw)| {
                    if i.is_multiple_of(2) {
                        U256::from(raw % 64)
                    } else {
                        U256::from(raw)
                    }
                })
                .collect();
            (H160::from(address), slots, code_hash, chunks)
        })
}

proptest! {
    // A few dozen cases, each building two whole tries over a few hundred
    // keys: the shapes that matter are the zone boundaries and the two key
    // lengths, and those are hit by every case.
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// No key the embedding derives is a byte-prefix of another.
    ///
    /// This is what `BinaryTrieError::PrefixViolation` enforces per insert —
    /// but only for the keys some test happened to insert. Asserted here of
    /// the *embedding*, over arbitrary addresses, slots and code hashes, so it
    /// pins that the key derivation upholds it rather than that the trie
    /// caught a violation.
    #[test]
    fn the_embedding_derives_a_prefix_free_key_set(
        accounts in proptest::collection::vec(account_strategy(), 4..12)
    ) {
        let mut keys: Vec<Vec<u8>> = accounts
            .iter()
            .flat_map(|(address, slots, code_hash, chunks)| {
                keys_for(*address, slots, *code_hash, chunks)
            })
            .collect();
        for key in &keys {
            assert_zone_shape(key);
        }

        keys.sort();
        keys.dedup();
        // A proper prefix sorts immediately before everything extending it, so
        // comparing neighbours in sorted order catches every prefix pair.
        for pair in keys.windows(2) {
            prop_assert!(
                !pair[1].starts_with(&pair[0]),
                "{:?} is a prefix of {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    /// Byte order is bit order, observed through the public API.
    ///
    /// `from_sorted_leaves` folds a **byte**-sorted run bottom-up, splitting
    /// each run at the first bit its ends disagree on and relying on the two
    /// sides being contiguous. `insert` descends bit by bit from the root and
    /// makes no ordering assumption at all. The two agree on a root only if
    /// byte order really is the order the bottom-up build needs — which is the
    /// mirror's ordering property, restated.
    ///
    /// If `bytes_to_bits` were LSB-first, a byte-sorted run would not be
    /// bit-sorted, the fold's `partition_point` would cut in the wrong place,
    /// and the roots would diverge.
    #[test]
    fn a_bottom_up_build_over_byte_sorted_keys_matches_insertion(
        accounts in proptest::collection::vec(account_strategy(), 4..12)
    ) {
        let mut keys: Vec<Vec<u8>> = accounts
            .iter()
            .flat_map(|(address, slots, code_hash, chunks)| {
                keys_for(*address, slots, *code_hash, chunks)
            })
            .collect();
        keys.sort();
        keys.dedup();

        let leaves: Vec<(Vec<u8>, [u8; 32])> = keys
            .iter()
            .enumerate()
            .map(|(i, key)| (key.clone(), [(i % 251) as u8; 32]))
            .collect();

        let sorted = BinaryTrie::from_sorted_leaves(
            Box::new(InMemoryBinaryTrieDB::new_empty()),
            leaves.clone(),
        )
        .expect("byte-sorted embedding keys are accepted");

        let mut inserted = BinaryTrie::new_temp();
        // Insert back to front, so the incremental build cannot accidentally
        // agree with the fold by sharing its traversal order.
        for (key, value) in leaves.iter().rev() {
            inserted.insert(key.clone(), *value).expect("insert");
        }

        prop_assert_eq!(sorted.root(), inserted.root());
    }
}
