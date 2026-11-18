//! Rebuilding the tries from the snap/2 flat state, and the retention bound on
//! rolling that state forward (devp2p `caps/snap.md`, "Synchronization
//! algorithm").
//!
//! The root check here is the only one a snap/2 sync performs. snap/1 gets one
//! for free because healing walks down from the pivot root and cannot finish
//! unless the trie reaches it; snap/2 removes healing, so if this accepts a
//! flat state that does not describe the pivot, nothing else will catch it.

use ethrex_common::{
    Address, BigEndianHash, H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{
        AccountState, BlockHeader,
        block_access_list::{AccountChanges, BlockAccessList, SlotChange, StorageChange},
    },
};
use ethrex_p2p::sync::{
    SyncError,
    snap2::{
        DownloadCursor, FlatState, MAX_CATCH_UP_BLOCKS, apply_bal_flat, catch_up_exceeds_retention,
        reconstruct_and_verify,
    },
};
use ethrex_storage::{EngineType, Store, hash_address, hash_key};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ethrex-snap2-recon-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp datadir");
    dir
}

fn store() -> Store {
    Store::new("memory", EngineType::InMemory).expect("in-memory store")
}

fn at(value: u64) -> H256 {
    H256::from_uint(&U256::from(value))
}

fn header(number: u64, state_root: H256) -> BlockHeader {
    BlockHeader {
        number,
        state_root,
        ..Default::default()
    }
}

/// Build the same state through the store's own trie API, to get the root the
/// reconstruction has to arrive at independently.
async fn expected_root(store: &Store, accounts: &[(H256, AccountState)]) -> H256 {
    use ethrex_crypto::NativeCrypto;
    use ethrex_rlp::encode::RLPEncode;
    let mut trie = store
        .open_direct_state_trie(*ethrex_common::constants::EMPTY_TRIE_HASH)
        .expect("open state trie");
    for (account_hash, account) in accounts {
        trie.insert(account_hash.0.to_vec(), account.encode_to_vec())
            .expect("insert account");
    }
    trie.hash(&NativeCrypto).expect("hash state trie")
}

/// The root `slots` hash to as `account`'s storage trie.
fn storage_root(store: &Store, account: H256, slots: &[(H256, U256)]) -> H256 {
    use ethrex_rlp::encode::RLPEncode;
    let mut trie = store
        .open_direct_storage_trie(account, *EMPTY_TRIE_HASH)
        .expect("open storage trie");
    for (slot_hash, value) in slots {
        trie.insert(slot_hash.0.to_vec(), value.encode_to_vec())
            .expect("insert slot");
    }
    trie.hash(&ethrex_crypto::NativeCrypto)
        .expect("hash storage trie")
}

/// An account-only state reconstructs to the root its leaves imply.
#[tokio::test]
async fn reconstructs_accounts_to_the_expected_root() {
    let store = store();
    let flat = FlatState::open(&temp_dir("accounts")).expect("open");

    let accounts: Vec<(H256, AccountState)> = (1u64..=5)
        .map(|n| {
            (
                at(n),
                AccountState {
                    nonce: n,
                    balance: U256::from(n * 100),
                    ..Default::default()
                },
            )
        })
        .collect();
    for (account_hash, account) in &accounts {
        flat.put_account(*account_hash, account).expect("write");
    }

    let root = expected_root(&store, &accounts).await;
    reconstruct_and_verify(&store, &flat, &header(10, root))
        .await
        .expect("reconstruction must match");

    flat.destroy().await.expect("destroy");
}

/// A contract's leaf is written with the root its slots hash to, not the one it
/// was served with. This is what makes a flat state assembled across several
/// pivots come out consistent.
#[tokio::test]
async fn a_stale_served_storage_root_is_replaced() {
    let store = store();
    let flat = FlatState::open(&temp_dir("stale-root")).expect("open");

    let contract = at(1);
    // Seed the leaf with a storage root that has nothing to do with its slots,
    // standing in for one served at an older pivot.
    flat.put_account(
        contract,
        &AccountState {
            nonce: 1,
            balance: U256::from(10),
            storage_root: H256::repeat_byte(0xab),
            ..Default::default()
        },
    )
    .expect("write account");
    for slot in 1u64..=3 {
        flat.put_slot(contract, at(slot), U256::from(slot * 7))
            .expect("write slot");
    }

    // The root the storage actually hashes to.
    let mut storage_trie = store
        .open_direct_storage_trie(contract, *ethrex_common::constants::EMPTY_TRIE_HASH)
        .expect("open storage trie");
    for slot in 1u64..=3 {
        use ethrex_rlp::encode::RLPEncode;
        storage_trie
            .insert(at(slot).0.to_vec(), U256::from(slot * 7).encode_to_vec())
            .expect("insert slot");
    }
    let real_storage_root = storage_trie
        .hash(&ethrex_crypto::NativeCrypto)
        .expect("hash storage trie");
    assert_ne!(real_storage_root, H256::repeat_byte(0xab));

    let root = expected_root(
        &store,
        &[(
            contract,
            AccountState {
                nonce: 1,
                balance: U256::from(10),
                storage_root: real_storage_root,
                ..Default::default()
            },
        )],
    )
    .await;

    reconstruct_and_verify(&store, &flat, &header(10, root))
        .await
        .expect("reconstruction must use the recomputed storage root");

    flat.destroy().await.expect("destroy");
}

/// A contract large enough to take the bulk sorted-trie builder hashes to the
/// same root as one built slot by slot.
///
/// Reconstruction picks between the two by slot count, so the threshold is a
/// place the root can silently change: both paths are exercised on every real
/// sync, and only the combined state root would show a disagreement, far from
/// the contract that caused it. 1500 slots is over the cutoff, the small
/// contracts above are under it.
#[tokio::test]
async fn a_bulk_built_storage_trie_matches_a_slot_by_slot_one() {
    use ethrex_rlp::encode::RLPEncode;

    let store = store();
    let flat = FlatState::open(&temp_dir("bulk-storage")).expect("open");

    let contract = at(1);
    const SLOTS: u64 = 1500;

    flat.put_account(
        contract,
        &AccountState {
            nonce: 1,
            balance: U256::from(10),
            ..Default::default()
        },
    )
    .expect("write account");
    for slot in 1..=SLOTS {
        flat.put_slot(contract, at(slot), U256::from(slot * 7))
            .expect("write slot");
    }

    // The reference: the same slots inserted one at a time through the store's
    // own trie API, which is the path reconstruction takes below the cutoff.
    let mut storage_trie = store
        .open_direct_storage_trie(contract, *ethrex_common::constants::EMPTY_TRIE_HASH)
        .expect("open storage trie");
    for slot in 1..=SLOTS {
        storage_trie
            .insert(at(slot).0.to_vec(), U256::from(slot * 7).encode_to_vec())
            .expect("insert slot");
    }
    let incremental_root = storage_trie
        .hash(&ethrex_crypto::NativeCrypto)
        .expect("hash storage trie");

    let root = expected_root(
        &store,
        &[(
            contract,
            AccountState {
                nonce: 1,
                balance: U256::from(10),
                storage_root: incremental_root,
                ..Default::default()
            },
        )],
    )
    .await;

    reconstruct_and_verify(&store, &flat, &header(10, root))
        .await
        .expect("the bulk builder must agree with the incremental one");

    flat.destroy().await.expect("destroy");
}

/// A flat state that does not describe the pivot is rejected. Nothing
/// downstream would catch it.
#[tokio::test]
async fn a_flat_state_that_misses_the_pivot_is_rejected() {
    let store = store();
    let flat = FlatState::open(&temp_dir("mismatch")).expect("open");

    flat.put_account(
        at(1),
        &AccountState {
            nonce: 1,
            ..Default::default()
        },
    )
    .expect("write");

    let err = reconstruct_and_verify(&store, &flat, &header(10, H256::repeat_byte(0x99)))
        .await
        .expect_err("a wrong root must not be accepted");
    assert!(
        format!("{err}").contains("root"),
        "expected a state root mismatch, got: {err}"
    );

    flat.destroy().await.expect("destroy");
}

/// A single dropped account changes the root, so a partial flat state cannot
/// pass as complete.
#[tokio::test]
async fn a_missing_account_fails_the_root_check() {
    let store = store();
    let flat = FlatState::open(&temp_dir("missing-account")).expect("open");

    let complete: Vec<(H256, AccountState)> = (1u64..=3)
        .map(|n| {
            (
                at(n),
                AccountState {
                    nonce: n,
                    ..Default::default()
                },
            )
        })
        .collect();
    // Everything but the last.
    for (account_hash, account) in &complete[..2] {
        flat.put_account(*account_hash, account).expect("write");
    }

    let root = expected_root(&store, &complete).await;
    reconstruct_and_verify(&store, &flat, &header(10, root))
        .await
        .expect_err("an incomplete flat state must not verify");

    flat.destroy().await.expect("destroy");
}

/// A contract whose last slot an access list cleared after it was served
/// reconstructs with the empty storage root.
///
/// The leaf still carries the root it was served with, which describes slots
/// that no longer exist. Passing it through misses the pivot root on every
/// attempt, and since a mismatch discards the whole download and starts over,
/// the sync never finishes.
#[tokio::test]
async fn a_contract_whose_storage_was_cleared_reconstructs_to_the_empty_root() {
    use ethrex_rlp::decode::RLPDecode;

    let store = store();
    let flat = FlatState::open(&temp_dir("cleared-storage")).expect("open");

    let address = Address::repeat_byte(0x42);
    let contract = H256::from_slice(&hash_address(&address));
    let slot = U256::from(1);
    let slot_hash = H256::from_slice(&hash_key(&H256::from_uint(&slot)));

    // Served at an older pivot holding a single slot.
    let served = AccountState {
        nonce: 1,
        balance: U256::from(10),
        storage_root: storage_root(&store, contract, &[(slot_hash, U256::from(7))]),
        ..Default::default()
    };
    flat.put_account(contract, &served).expect("write account");
    flat.put_slot(contract, slot_hash, U256::from(7))
        .expect("write slot");

    // A later block zeroes that slot.
    let mut cursor = DownloadCursor::new(1);
    cursor.advance_accounts(H256::repeat_byte(0xff));
    let bal = BlockAccessList::from_accounts(vec![AccountChanges {
        address,
        storage_changes: vec![SlotChange {
            slot,
            slot_changes: vec![StorageChange {
                post_value: U256::zero(),
                ..Default::default()
            }],
        }],
        ..Default::default()
    }]);
    let stats = apply_bal_flat(&flat, &cursor, &store, &bal).expect("apply");
    assert_eq!(stats.slots_deleted, 1);
    assert_eq!(flat.iter_slots(contract).count(), 0);
    assert_eq!(
        flat.get_account(contract)
            .expect("read")
            .map(|a| a.storage_root),
        Some(served.storage_root),
        "the patched leaf keeps its served root, or this is not testing the real case"
    );

    let root = expected_root(
        &store,
        &[(
            contract,
            AccountState {
                storage_root: *EMPTY_TRIE_HASH,
                ..served
            },
        )],
    )
    .await;
    reconstruct_and_verify(&store, &flat, &header(10, root))
        .await
        .expect("a cleared contract must reconstruct to the pivot root");

    let leaf = store
        .open_direct_state_trie(root)
        .expect("open state trie")
        .get(contract.as_bytes())
        .expect("read leaf")
        .expect("the contract is in the reconstructed trie");
    assert_eq!(
        AccountState::decode(&leaf)
            .expect("decode leaf")
            .storage_root,
        *EMPTY_TRIE_HASH
    );

    flat.destroy().await.expect("destroy");
}

/// A contract whose storage never reached the flat state fails the root check.
///
/// The leaf carries the storage root the pivot vouches for, so passing it
/// through would verify while the storage trie behind it was never built: a
/// node that answers the pivot root with none of the contract's slots. Each
/// leaf has to carry what its slots on disk hash to, and none hash to the empty
/// root.
#[tokio::test]
async fn a_contract_served_without_its_storage_fails_the_root_check() {
    let store = store();
    let flat = FlatState::open(&temp_dir("missing-storage")).expect("open");

    let contract = at(1);
    let slots: Vec<(H256, U256)> = (1u64..=3)
        .map(|slot| (at(slot), U256::from(slot * 7)))
        .collect();
    let account = AccountState {
        nonce: 1,
        balance: U256::from(10),
        storage_root: storage_root(&store, contract, &slots),
        ..Default::default()
    };
    // The leaf alone; none of its slots.
    flat.put_account(contract, &account).expect("write account");

    let root = expected_root(&store, &[(contract, account)]).await;
    let err = reconstruct_and_verify(&store, &flat, &header(10, root))
        .await
        .expect_err("a contract missing its storage must not verify");
    assert!(
        matches!(err, SyncError::StateRootMismatch(expected, _) if expected == root),
        "expected a state root mismatch, got: {err}"
    );

    flat.destroy().await.expect("destroy");
}

/// The catch-up bound comes from EIP-7928's retention floor: peers must hold
/// access lists for at least the weak subjectivity period, 3533 epochs.
#[test]
fn the_retention_bound_is_the_weak_subjectivity_period() {
    assert_eq!(MAX_CATCH_UP_BLOCKS, 3533 * 32);
}

/// A gap within the retention window is rolled forward; one past it is not, and
/// caps/snap.md has the node discard its partial state instead.
#[test]
fn only_a_gap_past_retention_is_refused() {
    let from = header(1_000, H256::zero());

    let inside = header(1_000 + MAX_CATCH_UP_BLOCKS, H256::zero());
    assert!(!catch_up_exceeds_retention(&from, &inside));

    let outside = header(1_000 + MAX_CATCH_UP_BLOCKS + 1, H256::zero());
    assert!(catch_up_exceeds_retention(&from, &outside));
}

/// A pivot that has not advanced is not a gap at all.
#[test]
fn a_pivot_that_did_not_move_is_within_retention() {
    let from = header(1_000, H256::zero());
    assert!(!catch_up_exceeds_retention(
        &from,
        &header(1_000, H256::zero())
    ));
    assert!(!catch_up_exceeds_retention(
        &from,
        &header(999, H256::zero())
    ));
}

// ---------------------------------------------------------------------------
// Resolving the block span a catch-up has to apply.
//
// This is also where a reorg past the pivot is detected, so both the happy path
// and the divergence are pinned here.
// ---------------------------------------------------------------------------

use ethrex_p2p::sync::snap2::gap_headers;

fn linked(number: u64, parent: H256) -> BlockHeader {
    BlockHeader {
        number,
        parent_hash: parent,
        // Distinguish otherwise-identical headers so sibling forks hash apart.
        state_root: at(number),
        ..Default::default()
    }
}

/// Headers ahead of the pivot are stored by hash during a snap sync and only
/// become canonical when the sync commits at the very end. Resolving the span
/// has to work from the hash chain alone; reading it out of the canonical index
/// would find nothing.
#[tokio::test]
async fn the_span_resolves_from_headers_that_are_not_canonical() {
    let store = store();

    let from = linked(100, H256::repeat_byte(0x11));
    let mut chain = vec![from.clone()];
    for number in 101..=105 {
        let parent = chain.last().expect("previous header").hash();
        chain.push(linked(number, parent));
    }
    // Stored by hash only: no forkchoice_update, so nothing is canonical.
    store
        .add_block_headers(chain[1..].to_vec())
        .await
        .expect("store headers");
    assert!(
        store
            .get_canonical_block_hash(103)
            .await
            .expect("read canonical")
            .is_none(),
        "the span must not be canonical, or this is not testing the real case"
    );

    let to = chain.last().expect("target header").clone();
    let span = gap_headers(&store, &from, &to).expect("resolve span");

    let numbers: Vec<u64> = span.iter().map(|header| header.number).collect();
    assert_eq!(numbers, vec![101, 102, 103, 104, 105]);
    assert_eq!(span.last().expect("last").hash(), to.hash());
}

/// The shortest span is the one block between adjacent pivots.
#[tokio::test]
async fn an_adjacent_pivot_resolves_to_one_block() {
    let store = store();
    let from = linked(100, H256::repeat_byte(0x11));
    let to = linked(101, from.hash());
    store
        .add_block_headers(vec![to.clone()])
        .await
        .expect("store header");

    let span = gap_headers(&store, &from, &to).expect("resolve span");
    assert_eq!(span.len(), 1);
    assert_eq!(span[0].hash(), to.hash());
}

/// A pivot the new chain does not descend from was reorged out. Its flat state
/// describes an abandoned fork, so the catch-up must refuse rather than roll
/// the wrong diffs onto it.
#[tokio::test]
async fn a_pivot_the_new_chain_does_not_reach_is_reported_as_a_reorg() {
    let store = store();

    // Two forks from a shared ancestor.
    let ancestor = linked(100, H256::repeat_byte(0x11));
    let orphaned = BlockHeader {
        state_root: H256::repeat_byte(0xaa),
        ..linked(101, ancestor.hash())
    };
    let mut canonical = vec![linked(101, ancestor.hash())];
    for number in 102..=104 {
        let parent = canonical.last().expect("previous header").hash();
        canonical.push(linked(number, parent));
    }
    assert_ne!(orphaned.hash(), canonical[0].hash());

    store
        .add_block_headers(canonical.clone())
        .await
        .expect("store headers");

    let to = canonical.last().expect("target").clone();
    let err = gap_headers(&store, &orphaned, &to).expect_err("must detect the reorg");
    assert!(
        matches!(err, ethrex_p2p::sync::SyncError::ChainReorgDetected { .. }),
        "expected a reorg, got: {err}"
    );
}

/// A hole in the header chain is not silently treated as a reorg: the span
/// simply cannot be resolved.
#[tokio::test]
async fn a_missing_header_in_the_span_is_reported_as_missing() {
    let store = store();

    let from = linked(100, H256::repeat_byte(0x11));
    let mut chain = vec![from.clone()];
    for number in 101..=104 {
        let parent = chain.last().expect("previous header").hash();
        chain.push(linked(number, parent));
    }
    // Store everything except block 102.
    let stored: Vec<BlockHeader> = chain[1..]
        .iter()
        .filter(|header| header.number != 102)
        .cloned()
        .collect();
    store
        .add_block_headers(stored)
        .await
        .expect("store headers");

    let to = chain.last().expect("target").clone();
    let err = gap_headers(&store, &from, &to).expect_err("must not resolve");
    assert!(
        matches!(err, ethrex_p2p::sync::SyncError::MissingHeaderForBal(_)),
        "expected a missing header, got: {err}"
    );
}
