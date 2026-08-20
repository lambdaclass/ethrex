#![expect(clippy::unnecessary_to_owned, clippy::useless_vec)]
use cita_trie::{MemoryDB as CitaMemoryDB, PatriciaTrie as CitaTrie, Trie as CitaTrieTrait};
use std::sync::Arc;

use ethrex_crypto::NativeCrypto;
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::{InMemoryTrieDB, Nibbles, Node, NodeHash, NodeRef, Trie, db::NodeMap};

use hasher::HasherKeccak;
use hex_literal::hex;
use proptest::{
    collection::{btree_set, vec},
    prelude::*,
    proptest,
};

#[test]
fn compute_hash() {
    let mut trie = Trie::new_temp();
    trie.insert(b"first".to_vec(), b"value".to_vec()).unwrap();
    trie.insert(b"second".to_vec(), b"value".to_vec()).unwrap();

    assert_eq!(
        trie.hash(&NativeCrypto).unwrap().as_ref(),
        hex!("f7537e7f4b313c426440b7fface6bff76f51b3eb0d127356efbe6f2b3c891501")
    );
}

#[test]
fn compute_hash_long() {
    let mut trie = Trie::new_temp();
    trie.insert(b"first".to_vec(), b"value".to_vec()).unwrap();
    trie.insert(b"second".to_vec(), b"value".to_vec()).unwrap();
    trie.insert(b"third".to_vec(), b"value".to_vec()).unwrap();
    trie.insert(b"fourth".to_vec(), b"value".to_vec()).unwrap();

    assert_eq!(
        trie.hash(&NativeCrypto).unwrap().0.to_vec(),
        hex!("e2ff76eca34a96b68e6871c74f2a5d9db58e59f82073276866fdd25e560cedea")
    );
}

#[test]
fn get_insert_words() {
    let mut trie = Trie::new_temp();
    let first_path = b"first".to_vec();
    let first_value = b"value_a".to_vec();
    let second_path = b"second".to_vec();
    let second_value = b"value_b".to_vec();
    // Check that the values dont exist before inserting
    assert!(trie.get(&first_path).unwrap().is_none());
    assert!(trie.get(&second_path).unwrap().is_none());
    // Insert values
    trie.insert(first_path.clone(), first_value.clone())
        .unwrap();
    trie.insert(second_path.clone(), second_value.clone())
        .unwrap();
    // Check values
    assert_eq!(trie.get(&first_path).unwrap(), Some(first_value));
    assert_eq!(trie.get(&second_path).unwrap(), Some(second_value));
}

#[test]
fn get_insert_zero() {
    let mut trie = Trie::new_temp();
    trie.insert(vec![0x0], b"value".to_vec()).unwrap();
    let first = trie.get(&[0x0][..].to_vec()).unwrap();
    assert_eq!(first, Some(b"value".to_vec()));
}

#[test]
fn get_insert_a() {
    let mut trie = Trie::new_temp();
    trie.insert(vec![16], vec![0]).unwrap();
    trie.insert(vec![16, 0], vec![0]).unwrap();

    let item = trie.get(&vec![16]).unwrap();
    assert_eq!(item, Some(vec![0]));

    let item = trie.get(&vec![16, 0]).unwrap();
    assert_eq!(item, Some(vec![0]));
}

#[test]
fn get_insert_b() {
    let mut trie = Trie::new_temp();
    trie.insert(vec![0, 0], vec![0, 0]).unwrap();
    trie.insert(vec![1, 0], vec![1, 0]).unwrap();

    let item = trie.get(&vec![1, 0]).unwrap();
    assert_eq!(item, Some(vec![1, 0]));

    let item = trie.get(&vec![0, 0]).unwrap();
    assert_eq!(item, Some(vec![0, 0]));
}

#[test]
fn get_insert_c() {
    let mut trie = Trie::new_temp();
    let vecs = vec![
        vec![26, 192, 44, 251],
        vec![195, 132, 220, 124, 112, 201, 70, 128, 235],
        vec![126, 138, 25, 245, 146],
        vec![129, 176, 66, 2, 150, 151, 180, 60, 124],
        vec![138, 101, 157],
    ];
    for x in &vecs {
        trie.insert(x.clone(), x.clone()).unwrap();
    }
    for x in &vecs {
        let item = trie.get(x).unwrap();
        assert_eq!(item, Some(x.clone()));
    }
}

#[test]
fn get_insert_d() {
    let mut trie = Trie::new_temp();
    let vecs = vec![
        vec![52, 53, 143, 52, 206, 112],
        vec![14, 183, 34, 39, 113],
        vec![55, 5],
        vec![134, 123, 19],
        vec![0, 59, 240, 89, 83, 167],
        vec![22, 41],
        vec![13, 166, 159, 101, 90, 234, 91],
        vec![31, 180, 161, 122, 115, 51, 37, 61, 101],
        vec![208, 192, 4, 12, 163, 254, 129, 206, 109],
    ];
    for x in &vecs {
        trie.insert(x.clone(), x.clone()).unwrap();
    }
    for x in &vecs {
        let item = trie.get(x).unwrap();
        assert_eq!(item, Some(x.clone()));
    }
}

#[test]
fn get_insert_e() {
    let mut trie = Trie::new_temp();
    trie.insert(vec![0x00], vec![0x00]).unwrap();
    trie.insert(vec![0xC8], vec![0xC8]).unwrap();
    trie.insert(vec![0xC8, 0x00], vec![0xC8, 0x00]).unwrap();

    assert_eq!(trie.get(&vec![0x00]).unwrap(), Some(vec![0x00]));
    assert_eq!(trie.get(&vec![0xC8]).unwrap(), Some(vec![0xC8]));
    assert_eq!(trie.get(&vec![0xC8, 0x00]).unwrap(), Some(vec![0xC8, 0x00]));
}

#[test]
fn get_insert_f() {
    let mut trie = Trie::new_temp();
    trie.insert(vec![0x00], vec![0x00]).unwrap();
    trie.insert(vec![0x01], vec![0x01]).unwrap();
    trie.insert(vec![0x10], vec![0x10]).unwrap();
    trie.insert(vec![0x19], vec![0x19]).unwrap();
    trie.insert(vec![0x19, 0x00], vec![0x19, 0x00]).unwrap();
    trie.insert(vec![0x1A], vec![0x1A]).unwrap();

    assert_eq!(trie.get(&vec![0x00]).unwrap(), Some(vec![0x00]));
    assert_eq!(trie.get(&vec![0x01]).unwrap(), Some(vec![0x01]));
    assert_eq!(trie.get(&vec![0x10]).unwrap(), Some(vec![0x10]));
    assert_eq!(trie.get(&vec![0x19]).unwrap(), Some(vec![0x19]));
    assert_eq!(trie.get(&vec![0x19, 0x00]).unwrap(), Some(vec![0x19, 0x00]));
    assert_eq!(trie.get(&vec![0x1A]).unwrap(), Some(vec![0x1A]));
}

#[test]
fn get_insert_remove_a() {
    let mut trie = Trie::new_temp();
    trie.insert(b"do".to_vec(), b"verb".to_vec()).unwrap();
    trie.insert(b"horse".to_vec(), b"stallion".to_vec())
        .unwrap();
    trie.insert(b"doge".to_vec(), b"coin".to_vec()).unwrap();
    trie.remove(&b"horse".to_vec()).unwrap();
    assert_eq!(trie.get(&b"do".to_vec()).unwrap(), Some(b"verb".to_vec()));
    assert_eq!(trie.get(&b"doge".to_vec()).unwrap(), Some(b"coin".to_vec()));
}

#[test]
fn get_insert_remove_b() {
    let mut trie = Trie::new_temp();
    trie.insert(vec![185], vec![185]).unwrap();
    trie.insert(vec![185, 0], vec![185, 0]).unwrap();
    trie.insert(vec![185, 1], vec![185, 1]).unwrap();
    trie.remove(&vec![185, 1]).unwrap();
    assert_eq!(trie.get(&vec![185, 0]).unwrap(), Some(vec![185, 0]));
    assert_eq!(trie.get(&vec![185]).unwrap(), Some(vec![185]));
    assert!(trie.get(&vec![185, 1]).unwrap().is_none());
}

#[test]
fn compute_hash_a() {
    let mut trie = Trie::new_temp();
    trie.insert(b"do".to_vec(), b"verb".to_vec()).unwrap();
    trie.insert(b"horse".to_vec(), b"stallion".to_vec())
        .unwrap();
    trie.insert(b"doge".to_vec(), b"coin".to_vec()).unwrap();
    trie.insert(b"dog".to_vec(), b"puppy".to_vec()).unwrap();

    assert_eq!(
        trie.hash(&NativeCrypto).unwrap().0.as_slice(),
        hex!("5991bb8c6514148a29db676a14ac506cd2cd5775ace63c30a4fe457715e9ac84").as_slice()
    );
}

#[test]
fn compute_hash_b() {
    let mut trie = Trie::new_temp();
    assert_eq!(
        trie.hash(&NativeCrypto).unwrap().0.as_slice(),
        hex!("56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421").as_slice(),
    );
}

#[test]
fn compute_hash_c() {
    let mut trie = Trie::new_temp();
    let data = [
        (
            hex!("0000000000000000000000000000000000000000000000000000000000000045").to_vec(),
            hex!("22b224a1420a802ab51d326e29fa98e34c4f24ea").to_vec(),
        ),
        (
            hex!("0000000000000000000000000000000000000000000000000000000000000046").to_vec(),
            hex!("67706c2076330000000000000000000000000000000000000000000000000000").to_vec(),
        ),
        (
            hex!("000000000000000000000000697c7b8c961b56f675d570498424ac8de1a918f6").to_vec(),
            hex!("1234567890").to_vec(),
        ),
        (
            hex!("0000000000000000000000007ef9e639e2733cb34e4dfc576d4b23f72db776b2").to_vec(),
            hex!("4655474156000000000000000000000000000000000000000000000000000000").to_vec(),
        ),
        (
            hex!("000000000000000000000000ec4f34c97e43fbb2816cfd95e388353c7181dab1").to_vec(),
            hex!("4e616d6552656700000000000000000000000000000000000000000000000000").to_vec(),
        ),
        (
            hex!("4655474156000000000000000000000000000000000000000000000000000000").to_vec(),
            hex!("7ef9e639e2733cb34e4dfc576d4b23f72db776b2").to_vec(),
        ),
        (
            hex!("4e616d6552656700000000000000000000000000000000000000000000000000").to_vec(),
            hex!("ec4f34c97e43fbb2816cfd95e388353c7181dab1").to_vec(),
        ),
        (
            hex!("000000000000000000000000697c7b8c961b56f675d570498424ac8de1a918f6").to_vec(),
            hex!("6f6f6f6820736f2067726561742c207265616c6c6c793f000000000000000000").to_vec(),
        ),
        (
            hex!("6f6f6f6820736f2067726561742c207265616c6c6c793f000000000000000000").to_vec(),
            hex!("697c7b8c961b56f675d570498424ac8de1a918f6").to_vec(),
        ),
    ];

    for (path, value) in data {
        trie.insert(path, value).unwrap();
    }

    assert_eq!(
        trie.hash(&NativeCrypto).unwrap().0.as_slice(),
        hex!("9f6221ebb8efe7cff60a716ecb886e67dd042014be444669f0159d8e68b42100").as_slice(),
    );
}

#[test]
fn compute_hash_d() {
    let mut trie = Trie::new_temp();

    let data = [
        (
            b"key1aa".to_vec(),
            b"0123456789012345678901234567890123456789xxx".to_vec(),
        ),
        (
            b"key1".to_vec(),
            b"0123456789012345678901234567890123456789Very_Long".to_vec(),
        ),
        (b"key2bb".to_vec(), b"aval3".to_vec()),
        (b"key2".to_vec(), b"short".to_vec()),
        (b"key3cc".to_vec(), b"aval3".to_vec()),
        (
            b"key3".to_vec(),
            b"1234567890123456789012345678901".to_vec(),
        ),
    ];

    for (path, value) in data {
        trie.insert(path, value).unwrap();
    }

    assert_eq!(
        trie.hash(&NativeCrypto).unwrap().0.as_slice(),
        hex!("cb65032e2f76c48b82b5c24b3db8f670ce73982869d38cd39a624f23d62a9e89").as_slice(),
    );
}

#[test]
fn compute_hash_e() {
    let mut trie = Trie::new_temp();
    trie.insert(b"abc".to_vec(), b"123".to_vec()).unwrap();
    trie.insert(b"abcd".to_vec(), b"abcd".to_vec()).unwrap();
    trie.insert(b"abc".to_vec(), b"abc".to_vec()).unwrap();

    assert_eq!(
        trie.hash(&NativeCrypto).unwrap().0.as_slice(),
        hex!("7a320748f780ad9ad5b0837302075ce0eeba6c26e3d8562c67ccc0f1b273298a").as_slice(),
    );
}

/// Builds a committed trie backed by a real DB whose root is an extension node with
/// a *hashed* child, and re-opens it from its root hash.
///
/// Re-opening matters: the root is then a `NodeRef::Hash`, so every node below it is
/// fetched from the DB by its path key instead of being served from the in-memory
/// `NodeRef::Node` cache, which is what `Trie::get_node` needs to exercise.
fn extension_rooted_trie() -> (Trie, Vec<[u8; 32]>) {
    // All keys share the leading nibbles `[0xa, 0xb]`, so the root is an extension
    // node with that prefix, and then diverge on the next nibble, so the extension's
    // child is a branch node holding six hashed leaves. Both are far over the
    // 32-byte limit under which a node would be inlined into its parent.
    let keys: Vec<[u8; 32]> = (0u8..6)
        .map(|i| {
            let mut key = [0u8; 32];
            key[0] = 0xab;
            key[1] = i << 4;
            key[31] = i;
            key
        })
        .collect();

    let db: NodeMap = Default::default();
    let mut trie = Trie::new(Box::new(InMemoryTrieDB::new(db.clone())));
    for key in &keys {
        trie.insert(key.to_vec(), vec![0x11; 40]).unwrap();
    }
    let root = trie.hash(&NativeCrypto).unwrap();

    (Trie::open(Box::new(InMemoryTrieDB::new(db)), root), keys)
}

/// Regression test for the child key used when `get_node` walks an extension node.
/// The child is stored under `extension path ++ prefix`; computing it from the
/// *remaining* path instead made every lookup that crosses an extension node with a
/// hashed child fail with `TrieError::InconsistentTree`.
#[test]
fn get_node_partial_path_crossing_extension_node() {
    let (trie, _keys) = extension_rooted_trie();

    let root = trie.root_node().unwrap().expect("trie should have a root");
    let Node::Extension(extension) = root.as_ref() else {
        panic!("expected an extension node at the root, got {root:?}");
    };
    let branch_path = extension.prefix.clone();
    assert_eq!(branch_path, Nibbles::from_hex(vec![0xa, 0xb]));
    // Precondition: an inline child would be read straight out of the extension node,
    // never from the DB, and so would not exercise the child path at all.
    assert!(matches!(
        extension.child,
        NodeRef::Hash(NodeHash::Hashed(_))
    ));

    let branch = extension
        .child
        .get_node(trie.db(), branch_path.as_ref())
        .unwrap()
        .expect("extension child should be stored under its path");
    let Node::Branch(branch_node) = branch.as_ref() else {
        panic!("expected a branch node below the extension, got {branch:?}");
    };
    let leaf_path = branch_path.append_new(0);
    let leaf = branch_node.choices[0]
        .get_node(trie.db(), leaf_path.as_ref())
        .unwrap()
        .expect("branch child should be stored under its path");

    // Partial path ending exactly at the extension node's child.
    assert_eq!(
        trie.get_node(&branch_path.encode_compact()).unwrap(),
        branch.encode_to_vec()
    );
    // Partial path continuing past the extension node, so the remaining path is no
    // longer empty once its prefix is skipped. This is the case the wrong child key
    // broke.
    assert_eq!(
        trie.get_node(&leaf_path.encode_compact()).unwrap(),
        leaf.encode_to_vec()
    );
}

/// Same traversal, entered through the full-32-byte-path shape of `get_node`.
#[test]
fn get_node_full_path_crossing_extension_node() {
    let (trie, keys) = extension_rooted_trie();
    let path = keys[0].to_vec();

    // The key is in the trie.
    assert!(trie.get(&path).unwrap().is_some());
    // `get_node` still answers with the documented empty vector: a full path expands
    // to 65 nibbles (64 plus the leaf flag) and only a node whose own path consumes
    // all of them is returned, which no leaf ever does because it keeps its remaining
    // partial path. The lookup does walk the root extension node though, and with the
    // wrong child key it failed with `TrieError::InconsistentTree` instead.
    assert!(trie.get_node(&path).unwrap().is_empty());
}

/// Paths that lead nowhere are reported as an empty node, not as an error.
#[test]
fn get_node_missing_paths_return_empty() {
    let (trie, _keys) = extension_rooted_trie();

    // Diverges from the root extension node's prefix.
    let diverging = Nibbles::from_hex(vec![0xa, 0xc]).encode_compact();
    assert!(trie.get_node(&diverging).unwrap().is_empty());
    // Crosses the extension node, then hits an empty branch choice.
    let empty_choice = Nibbles::from_hex(vec![0xa, 0xb, 0xf]).encode_compact();
    assert!(trie.get_node(&empty_choice).unwrap().is_empty());
    // Paths longer than a full 32-byte path are not handled.
    assert!(trie.get_node(&vec![0u8; 33]).unwrap().is_empty());
}

// Proptests
proptest! {
    #[test]
    fn proptest_get_insert(data in btree_set(vec(any::<u8>(), 1..100), 1..100)) {
        let mut trie = Trie::new_temp();

        for val in data.iter(){
            trie.insert(val.clone(), val.clone()).unwrap();
        }

        for val in data.iter() {
            let item = trie.get(val).unwrap();
            prop_assert!(item.is_some());
            prop_assert_eq!(&item.unwrap(), val);
        }
    }

    #[test]
    fn proptest_get_insert_with_removals(mut data in vec((vec(any::<u8>(), 5..100), any::<bool>()), 1..100)) {
        let mut trie = Trie::new_temp();
        // Remove duplicate values with different expected status
        data.sort_by_key(|(val, _)| val.clone());
        data.dedup_by_key(|(val, _)| val.clone());
        // Insertions
        for (val, _) in data.iter() {
            trie.insert(val.clone(), val.clone()).unwrap();
        }
        // Removals
        for (val, should_remove) in data.iter() {
            if *should_remove {
                let removed = trie.remove(val).unwrap();
                prop_assert_eq!(removed, Some(val.clone()));
            }
        }
        // Check trie values
        for (val, removed) in data.iter() {
            let item = trie.get(val).unwrap();
            if !removed {
                prop_assert_eq!(item, Some(val.clone()));
            } else {
                prop_assert!(item.is_none());
            }
        }
    }

    #[test]
    // The previous test needs to sort the input values in order to get rid of duplicate entries, leading to ordered insertions
    // This check has a fixed way of determining whether a value should be removed but doesn't require ordered insertions
    fn proptest_get_insert_with_removals_unsorted(data in btree_set(vec(any::<u8>(), 5..100), 1..100)) {
        let mut trie = Trie::new_temp();
        // Remove all values that have an odd first value
        let remove = |value: &Vec<u8>| -> bool {
            value.first().is_some_and(|v| v % 2 != 0)
        };
        // Insertions
        for val in data.iter() {
            trie.insert(val.clone(), val.clone()).unwrap();
        }
        // Removals
        for val in data.iter() {
            if remove(val) {
                let removed = trie.remove(&val.clone()).unwrap();
                prop_assert_eq!(removed, Some(val.clone()));
            }
        }
        // Check trie values
        for val in data.iter() {
            let item = trie.get(val).unwrap();
            if !remove(val) {
                prop_assert_eq!(item, Some(val.clone()));
            } else {
                prop_assert!(item.is_none());
            }
        }
    }

    #[test]
    fn proptest_compare_hash(data in btree_set(vec(any::<u8>(), 1..100), 1..100)) {
        let mut trie = Trie::new_temp();
        let mut cita_trie = cita_trie();

        for val in data.iter(){
            trie.insert(val.clone(), val.clone()).unwrap();
            cita_trie.insert(val.clone(), val.clone()).unwrap();
        }

        let hash = trie.hash(&NativeCrypto).unwrap().0.to_vec();
        let cita_hash = cita_trie.root().unwrap();
        prop_assert_eq!(hash, cita_hash);
    }

    #[test]
    fn proptest_compare_hash_with_removals(mut data in vec((vec(any::<u8>(), 5..100), any::<bool>()), 1..100)) {
        let mut trie = Trie::new_temp();
        let mut cita_trie = cita_trie();
        // Remove duplicate values with different expected status
        data.sort_by_key(|(val, _)| val.clone());
        data.dedup_by_key(|(val, _)| val.clone());
        // Insertions
        for (val, _) in data.iter() {
            trie.insert(val.clone(), val.clone()).unwrap();
            cita_trie.insert(val.clone(), val.clone()).unwrap();
        }
        // Removals
        for (val, should_remove) in data.iter() {
            if *should_remove {
                trie.remove(val).unwrap();
                cita_trie.remove(val).unwrap();
                // Compare hashes
                let hash = trie.hash(&NativeCrypto).unwrap().0.to_vec();
                let cita_hash = cita_trie.root().unwrap();
                prop_assert_eq!(hash, cita_hash);
            }
        }
    }

    #[test]
    // The previous test needs to sort the input values in order to get rid of duplicate entries, leading to ordered insertions
    // This check has a fixed way of determining whether a value should be removed but doesn't require ordered insertions
    fn proptest_compare_hash_with_removals_unsorted(data in btree_set(vec(any::<u8>(), 5..100), 1..100)) {
        let mut trie = Trie::new_temp();
        let mut cita_trie = cita_trie();
        // Remove all values that have an odd first value
        let remove = |value: &Vec<u8>| -> bool {
            value.first().is_some_and(|v| v % 2 != 0)
        };
        // Insertions
        for val in data.iter() {
            trie.insert(val.clone(), val.clone()).unwrap();
            cita_trie.insert(val.clone(), val.clone()).unwrap();
        }
        // Removals
        for val in data.iter() {
            if remove(val) {
                trie.remove(val).unwrap();
                cita_trie.remove(val).unwrap();
                // Compare hashes
                let hash = trie.hash(&NativeCrypto).unwrap().0.to_vec();
                let cita_hash = cita_trie.root().unwrap();
                prop_assert_eq!(hash, cita_hash);
            }
        }
    }

    #[test]
    fn proptest_compare_hash_between_inserts(data in btree_set(vec(any::<u8>(), 1..100), 1..100)) {
        let mut trie = Trie::new_temp();
        let mut cita_trie = cita_trie();

        for val in data.iter(){
            trie.insert(val.clone(), val.clone()).unwrap();
            cita_trie.insert(val.clone(), val.clone()).unwrap();
            let hash = trie.hash(&NativeCrypto).unwrap().0.to_vec();
            let cita_hash = cita_trie.root().unwrap();
            prop_assert_eq!(hash, cita_hash);
        }

    }

    #[test]
    fn proptest_compare_proof(data in btree_set(vec(any::<u8>(), 1..100), 1..100)) {
        let mut trie = Trie::new_temp();
        let mut cita_trie = cita_trie();

        for val in data.iter(){
            trie.insert(val.clone(), val.clone()).unwrap();
            cita_trie.insert(val.clone(), val.clone()).unwrap();
        }
        let _ = cita_trie.root();
        for val in data.iter(){
            let proof = trie.get_proof(val).unwrap();
            let cita_proof = cita_trie.get_proof(val).unwrap();
            prop_assert_eq!(proof, cita_proof);
        }
    }

    #[test]
    fn proptest_compare_proof_with_removals(mut data in vec((vec(any::<u8>(), 5..100), any::<bool>()), 1..100)) {
        let mut trie = Trie::new_temp();
        let mut cita_trie = cita_trie();
        // Remove duplicate values with different expected status
        data.sort_by_key(|(val, _)| val.clone());
        data.dedup_by_key(|(val, _)| val.clone());
        // Insertions
        for (val, _) in data.iter() {
            trie.insert(val.clone(), val.clone()).unwrap();
            cita_trie.insert(val.clone(), val.clone()).unwrap();
        }
        // Removals
        for (val, should_remove) in data.iter() {
            if *should_remove {
                trie.remove(val).unwrap();
                cita_trie.remove(val).unwrap();
            }
        }
        // Compare proofs
        let _ = cita_trie.root();
        for (val, _) in data.iter() {
            let proof = trie.get_proof(val).unwrap();
            let cita_proof = cita_trie.get_proof(val).unwrap();
            prop_assert_eq!(proof, cita_proof);
        }
    }

    #[test]
    // The previous test needs to sort the input values in order to get rid of duplicate entries, leading to ordered insertions
    // This check has a fixed way of determining whether a value should be removed but doesn't require ordered insertions
    fn proptest_compare_proof_with_removals_unsorted(data in btree_set(vec(any::<u8>(), 5..100), 1..100)) {
        let mut trie = Trie::new_temp();
        let mut cita_trie = cita_trie();
        // Remove all values that have an odd first value
        let remove = |value: &Vec<u8>| -> bool {
            value.first().is_some_and(|v| v % 2 != 0)
        };
        // Insertions
        for val in data.iter() {
            trie.insert(val.clone(), val.clone()).unwrap();
            cita_trie.insert(val.clone(), val.clone()).unwrap();
        }
        // Removals
        for val in data.iter() {
            if remove(val) {
                trie.remove(val).unwrap();
                cita_trie.remove(val).unwrap();
            }
        }
        // Compare proofs
        let _ = cita_trie.root();
        for val in data.iter() {
            let proof = trie.get_proof(val).unwrap();
            let cita_proof = cita_trie.get_proof(val).unwrap();
            prop_assert_eq!(proof, cita_proof);
        }
    }

}

fn cita_trie() -> CitaTrie<CitaMemoryDB, HasherKeccak> {
    let memdb = Arc::new(CitaMemoryDB::new(true));
    let hasher = Arc::new(HasherKeccak::new());

    CitaTrie::new(Arc::clone(&memdb), Arc::clone(&hasher))
}

#[test]
fn get_proof_one_leaf() {
    // Trie -> Leaf["duck"]
    let mut cita_trie = cita_trie();
    let mut trie = Trie::new_temp();
    cita_trie
        .insert(b"duck".to_vec(), b"duckling".to_vec())
        .unwrap();
    trie.insert(b"duck".to_vec(), b"duckling".to_vec()).unwrap();
    let cita_proof = cita_trie.get_proof(b"duck".as_ref()).unwrap();
    let trie_proof = trie.get_proof(&b"duck".to_vec()).unwrap();
    assert_eq!(cita_proof, trie_proof);
}

#[test]
fn get_proof_two_leaves() {
    // Trie -> Extension[Branch[Leaf["duck"] Leaf["goose"]]]
    let mut cita_trie = cita_trie();
    let mut trie = Trie::new_temp();
    cita_trie
        .insert(b"duck".to_vec(), b"duck".to_vec())
        .unwrap();
    cita_trie
        .insert(b"goose".to_vec(), b"goose".to_vec())
        .unwrap();
    trie.insert(b"duck".to_vec(), b"duck".to_vec()).unwrap();
    trie.insert(b"goose".to_vec(), b"goose".to_vec()).unwrap();
    let _ = cita_trie.root();
    let cita_proof = cita_trie.get_proof(b"duck".as_ref()).unwrap();
    let trie_proof = trie.get_proof(&b"duck".to_vec()).unwrap();
    assert_eq!(cita_proof, trie_proof);
}

#[test]
fn get_proof_one_big_leaf() {
    // Trie -> Leaf[[0,0,0,0,0,0,0,0,0,0,0,0,0,0]]
    let val = vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut cita_trie = cita_trie();
    let mut trie = Trie::new_temp();
    cita_trie.insert(val.clone(), val.clone()).unwrap();
    trie.insert(val.clone(), val.clone()).unwrap();
    let _ = cita_trie.root();
    let cita_proof = cita_trie.get_proof(&val).unwrap();
    let trie_proof = trie.get_proof(&val).unwrap();
    assert_eq!(cita_proof, trie_proof);
}

#[test]
fn get_proof_path_in_branch() {
    // Trie -> Extension[Branch[ [Leaf[[183,0,0,0,0,0]]], [183]]]
    let mut cita_trie = cita_trie();
    let mut trie = Trie::new_temp();
    cita_trie.insert(vec![183], vec![183]).unwrap();
    cita_trie
        .insert(vec![183, 0, 0, 0, 0, 0], vec![183, 0, 0, 0, 0, 0])
        .unwrap();
    trie.insert(vec![183], vec![183]).unwrap();
    trie.insert(vec![183, 0, 0, 0, 0, 0], vec![183, 0, 0, 0, 0, 0])
        .unwrap();
    let _ = cita_trie.root();
    let cita_proof = cita_trie.get_proof(&[183]).unwrap();
    let trie_proof = trie.get_proof(&vec![183]).unwrap();
    assert_eq!(cita_proof, trie_proof);
}

#[test]
fn get_proof_removed_value() {
    let a = vec![5, 0, 0, 0, 0];
    let b = vec![6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut cita_trie = cita_trie();
    let mut trie = Trie::new_temp();
    cita_trie.insert(a.clone(), a.clone()).unwrap();
    cita_trie.insert(b.clone(), b.clone()).unwrap();
    trie.insert(a.clone(), a.clone()).unwrap();
    trie.insert(b.clone(), b).unwrap();
    trie.remove(&a).unwrap();
    cita_trie.remove(&a).unwrap();
    let _ = cita_trie.root();
    let cita_proof = cita_trie.get_proof(&a).unwrap();
    let trie_proof = trie.get_proof(&a).unwrap();
    assert_eq!(cita_proof, trie_proof);
}
