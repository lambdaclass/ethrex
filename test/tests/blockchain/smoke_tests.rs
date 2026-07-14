use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    error::{ChainError, InvalidForkChoice},
    fork_choice::{apply_fork_choice, apply_fork_choice_with_deep_reorg},
    is_canonical, latest_canonical_block_hash,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256,
    types::{Block, BlockBody, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER},
};
use ethrex_storage::{EngineType, Store};
use ethrex_trie::EMPTY_TRIE_HASH;
use ethrex_vm::BlockExecutionResult;

#[tokio::test]
async fn test_small_to_long_reorg() {
    // Store and genesis
    let store = test_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let genesis_hash = genesis_header.hash();

    // Create blockchain
    let blockchain = Blockchain::default_with_store(store.clone());

    // Add first block. We'll make it canonical.
    let block_1a = new_block(&store, &genesis_header).await;
    let hash_1a = block_1a.hash();
    blockchain.add_block(block_1a.clone()).unwrap();
    store
        .forkchoice_update(vec![], 1, hash_1a, None, None)
        .await
        .unwrap();
    let retrieved_1a = store.get_block_header(1).unwrap().unwrap();

    assert_eq!(retrieved_1a, block_1a.header);
    assert!(is_canonical(&store, 1, hash_1a).await.unwrap());

    // Add second block at height 1. Will not be canonical.
    let block_1b = new_block(&store, &genesis_header).await;
    let hash_1b = block_1b.hash();
    blockchain
        .add_block(block_1b.clone())
        .expect("Could not add block 1b.");
    let retrieved_1b = store.get_block_header_by_hash(hash_1b).unwrap().unwrap();

    assert_ne!(retrieved_1a, retrieved_1b);
    assert!(!is_canonical(&store, 1, hash_1b).await.unwrap());

    // Add a third block at height 2, child to the non canonical block.
    let block_2 = new_block(&store, &block_1b.header).await;
    let hash_2 = block_2.hash();
    blockchain
        .add_block(block_2.clone())
        .expect("Could not add block 2.");
    let retrieved_2 = store.get_block_header_by_hash(hash_2).unwrap();

    assert!(retrieved_2.is_some());
    assert!(store.get_canonical_block_hash(2).await.unwrap().is_none());

    // Receive block 2 as new head.
    apply_fork_choice(
        &store,
        block_2.hash(),
        genesis_header.hash(),
        genesis_header.hash(),
        None,
    )
    .await
    .unwrap();

    // Check that canonical blocks changed to the new branch.
    assert!(is_canonical(&store, 0, genesis_hash).await.unwrap());
    assert!(is_canonical(&store, 1, hash_1b).await.unwrap());
    assert!(is_canonical(&store, 2, hash_2).await.unwrap());
    assert!(!is_canonical(&store, 1, hash_1a).await.unwrap());
}

#[tokio::test]
async fn test_sync_not_supported_yet() {
    let store = test_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    // Create blockchain
    let blockchain = Blockchain::default_with_store(store.clone());

    // Build a single valid block.
    let block_1 = new_block(&store, &genesis_header).await;
    let hash_1 = block_1.hash();
    blockchain.add_block(block_1.clone()).unwrap();
    apply_fork_choice(&store, hash_1, H256::zero(), H256::zero(), None)
        .await
        .unwrap();

    // Build a child, then change its parent, making it effectively a pending block.
    let mut block_2 = new_block(&store, &block_1.header).await;
    block_2.header.parent_hash = H256::random();
    let hash_2 = block_2.hash();
    let result = blockchain.add_block(block_2.clone());
    assert!(matches!(result, Err(ChainError::ParentNotFound)));

    // block 2 should now be pending.
    assert!(store.get_pending_block(hash_2).await.unwrap().is_some());

    let fc_result = apply_fork_choice(&store, hash_2, H256::zero(), H256::zero(), None).await;
    assert!(matches!(fc_result, Err(InvalidForkChoice::Syncing)));

    // block 2 should still be pending.
    assert!(store.get_pending_block(hash_2).await.unwrap().is_some());
}

#[tokio::test]
async fn test_reorg_from_long_to_short_chain() {
    // Store and genesis
    let store = test_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let genesis_hash = genesis_header.hash();

    // Create blockchain
    let blockchain = Blockchain::default_with_store(store.clone());

    // Add first block. Not canonical.
    let block_1a = new_block(&store, &genesis_header).await;
    let hash_1a = block_1a.hash();
    blockchain.add_block(block_1a.clone()).unwrap();
    let retrieved_1a = store.get_block_header_by_hash(hash_1a).unwrap().unwrap();

    assert!(!is_canonical(&store, 1, hash_1a).await.unwrap());

    // Add second block at height 1. Canonical.
    let block_1b = new_block(&store, &genesis_header).await;
    let hash_1b = block_1b.hash();
    blockchain
        .add_block(block_1b.clone())
        .expect("Could not add block 1b.");
    apply_fork_choice(&store, hash_1b, genesis_hash, genesis_hash, None)
        .await
        .unwrap();
    let retrieved_1b = store.get_block_header(1).unwrap().unwrap();

    assert_ne!(retrieved_1a, retrieved_1b);
    assert_eq!(retrieved_1b, block_1b.header);
    assert!(is_canonical(&store, 1, hash_1b).await.unwrap());
    assert_eq!(latest_canonical_block_hash(&store).await.unwrap(), hash_1b);

    // Add a third block at height 2, child to the canonical one.
    let block_2 = new_block(&store, &block_1b.header).await;
    let hash_2 = block_2.hash();
    blockchain
        .add_block(block_2.clone())
        .expect("Could not add block 2.");
    apply_fork_choice(&store, hash_2, genesis_hash, genesis_hash, None)
        .await
        .unwrap();
    let retrieved_2 = store.get_block_header_by_hash(hash_2).unwrap();
    assert_eq!(latest_canonical_block_hash(&store).await.unwrap(), hash_2);

    assert!(retrieved_2.is_some());
    assert!(is_canonical(&store, 2, hash_2).await.unwrap());
    assert_eq!(
        store.get_canonical_block_hash(2).await.unwrap().unwrap(),
        hash_2
    );

    // Receive block 1a as new head.
    apply_fork_choice(
        &store,
        block_1a.hash(),
        genesis_header.hash(),
        genesis_header.hash(),
        None,
    )
    .await
    .unwrap();

    // Check that canonical blocks changed to the new branch.
    assert!(is_canonical(&store, 0, genesis_hash).await.unwrap());
    assert!(is_canonical(&store, 1, hash_1a).await.unwrap());
    assert!(!is_canonical(&store, 1, hash_1b).await.unwrap());
    assert!(!is_canonical(&store, 2, hash_2).await.unwrap());
}

#[tokio::test]
async fn new_head_ancestor_of_finalized_should_skip() {
    // Per execution-apis PR 786, the no-reorg skip optimization only applies when the new
    // head is a VALID canonical ancestor of the latest known finalized block. Build a chain
    // of 3 blocks, finalize block 2, then FCU to block 1 (an ancestor of finalized) and
    // assert that the update is skipped.
    let store = test_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let blockchain = Blockchain::default_with_store(store.clone());

    let block_1 = new_block(&store, &genesis_header).await;
    let hash_1 = block_1.hash();
    blockchain
        .add_block(block_1.clone())
        .expect("Could not add block 1.");

    let block_2 = new_block(&store, &block_1.header).await;
    let hash_2 = block_2.hash();
    blockchain
        .add_block(block_2.clone())
        .expect("Could not add block 2.");

    let block_3 = new_block(&store, &block_2.header).await;
    let hash_3 = block_3.hash();
    blockchain
        .add_block(block_3.clone())
        .expect("Could not add block 3.");

    // Make the chain canonical and finalize block 2.
    apply_fork_choice(&store, hash_3, hash_2, hash_2, None)
        .await
        .unwrap();

    assert!(is_canonical(&store, 1, hash_1).await.unwrap());
    assert!(is_canonical(&store, 2, hash_2).await.unwrap());
    assert!(is_canonical(&store, 3, hash_3).await.unwrap());

    // FCU to block 1 (ancestor of finalized): MUST be skipped.
    let result = apply_fork_choice(&store, hash_1, hash_1, hash_1, None).await;
    assert!(matches!(
        result,
        Err(InvalidForkChoice::NewHeadAlreadyCanonical)
    ));

    // State must be unchanged after the skip.
    assert_eq!(store.get_finalized_block_number().await.unwrap(), Some(2));
    assert_eq!(store.get_safe_block_number().await.unwrap(), Some(2));
    assert_eq!(store.get_latest_block_number().await.unwrap(), 3);
}

#[tokio::test]
async fn latest_block_number_should_always_be_the_canonical_head() {
    // Goal: put a, b in the same branch, both canonical.
    // Then add one in a different branch. Check that the last one is still the same.

    // Store and genesis
    let store = test_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let genesis_hash = genesis_header.hash();

    // Create blockchain
    let blockchain = Blockchain::default_with_store(store.clone());

    // Add block at height 1.
    let block_1 = new_block(&store, &genesis_header).await;
    blockchain
        .add_block(block_1.clone())
        .expect("Could not add block 1b.");

    // Add child at height 2.
    let block_2 = new_block(&store, &block_1.header).await;
    let hash_2 = block_2.hash();
    blockchain
        .add_block(block_2.clone())
        .expect("Could not add block 2.");

    assert_eq!(
        latest_canonical_block_hash(&store).await.unwrap(),
        genesis_hash
    );

    // Make that chain the canonical one.
    apply_fork_choice(&store, hash_2, genesis_hash, genesis_hash, None)
        .await
        .unwrap();

    assert_eq!(latest_canonical_block_hash(&store).await.unwrap(), hash_2);

    // Add a new, non canonical block, starting from genesis.
    let block_1b = new_block(&store, &genesis_header).await;
    let hash_b = block_1b.hash();
    blockchain
        .add_block(block_1b.clone())
        .expect("Could not add block b.");

    // The latest block should be the same.
    assert_eq!(latest_canonical_block_hash(&store).await.unwrap(), hash_2);

    // if we apply fork choice to the new one, then we should
    apply_fork_choice(&store, hash_b, genesis_hash, genesis_hash, None)
        .await
        .unwrap();

    // The latest block should now be the new head.
    assert_eq!(latest_canonical_block_hash(&store).await.unwrap(), hash_b);
}

#[tokio::test]
async fn unfinalized_reorg_deeper_than_32_is_allowed() {
    // Per execution-apis PR 786 point 6, -38006 TooDeepReorg fires when the reorg
    // depth exceeds the implementation-specific limit. ethrex's limit is
    // finality-bounded: ceiling = latest - finalized_number. With finalized at
    // genesis (block 0) and latest at 33, ceiling = 33. A depth-33 reorg exactly
    // hits the ceiling (33 > 33 is false), so the reorg must succeed.

    let store = test_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let genesis_hash = genesis_header.hash();
    let blockchain = Blockchain::default_with_store(store.clone());

    // Build canonical chain A: genesis → A1 → ... → A33.
    let mut parent = genesis_header.clone();
    let mut chain_a_hashes = Vec::new();
    for _ in 0..33 {
        let block = new_block(&store, &parent).await;
        parent = block.header.clone();
        chain_a_hashes.push(block.hash());
        blockchain.add_block(block).unwrap();
    }
    let head_a = *chain_a_hashes.last().unwrap();
    apply_fork_choice(&store, head_a, genesis_hash, genesis_hash, None)
        .await
        .expect("FCU to chain A head should succeed");
    assert!(is_canonical(&store, 33, head_a).await.unwrap());

    // Build alternate chain B from genesis. `new_block` randomizes fee_recipient and
    // beacon_root, so each block hash differs from chain A even at the same height.
    let mut parent = genesis_header.clone();
    let mut chain_b_hashes = Vec::new();
    for _ in 0..33 {
        let block = new_block(&store, &parent).await;
        parent = block.header.clone();
        chain_b_hashes.push(block.hash());
        blockchain.add_block(block).unwrap();
    }
    let head_b = *chain_b_hashes.last().unwrap();
    assert_ne!(head_a, head_b);

    // FCU to chain B head: reorg depth = 33, ceiling = latest(33) - finalized(0) = 33.
    // depth(33) > ceiling(33) is false, so the reorg is allowed.
    apply_fork_choice(&store, head_b, genesis_hash, genesis_hash, None)
        .await
        .expect("33-block finality-bounded reorg should be allowed");

    // Chain B is canonical end-to-end; chain A's 33 blocks are no longer canonical.
    assert!(is_canonical(&store, 33, head_b).await.unwrap());
    assert!(!is_canonical(&store, 33, head_a).await.unwrap());
    for (i, hash) in chain_b_hashes.iter().enumerate() {
        assert!(
            is_canonical(&store, (i + 1) as u64, *hash).await.unwrap(),
            "chain B block at height {} should be canonical",
            i + 1
        );
    }
}

// ---------------------------------------------------------------------------
// Finality-bounded reorg cap tests (issue #6685, Phase 2).
// ---------------------------------------------------------------------------

/// Stores a fake (empty, EVM-less) block header whose state_root is the empty
/// trie hash so that `Store::has_state_root` accepts it. Returns the block hash.
async fn store_fake_block(store: &Store, number: u64, parent_hash: H256) -> H256 {
    let header = BlockHeader {
        number,
        parent_hash,
        state_root: *EMPTY_TRIE_HASH,
        timestamp: number * 12,
        ..Default::default()
    };
    let block = Block::new(header.clone(), BlockBody::default());
    let hash = block.hash();
    store.add_block_header(hash, header).await.unwrap();
    hash
}

/// Task 2.7: Reorg ceiling is `latest - finalized_number`; a reorg within the
/// ceiling succeeds and one that exceeds it returns `TooDeepReorg`.
#[tokio::test]
async fn reorg_depth_bounded_by_finalized() {
    let store = test_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    // Chain A: 10 real blocks so state roots are live for the link blocks.
    let blockchain = Blockchain::default_with_store(store.clone());
    let mut parent = genesis_header.clone();
    let mut chain_a = Vec::new();
    for _ in 0..10 {
        let block = new_block(&store, &parent).await;
        parent = block.header.clone();
        chain_a.push((block.header.number, block.hash()));
        blockchain.add_block(block).unwrap();
    }
    let (head_a_num, head_a_hash) = *chain_a.last().unwrap();
    // Finalize block 3. ceiling = 10 - 3 = 7.
    let (_, finalized_hash) = chain_a[2]; // index 2 = block 3
    apply_fork_choice(&store, head_a_hash, finalized_hash, finalized_hash, None)
        .await
        .expect("chain A FCU should succeed");
    assert_eq!(head_a_num, 10);

    // Shallow reorg (depth 5): chain B diverges at block 6.
    // link = block 5 (canonical), reorg_depth = 10 - 5 = 5 <= ceiling 7.
    let (_, link5_hash) = chain_a[4]; // block 5
    let mut parent_hash_b = link5_hash;
    let mut head_b_hash = link5_hash;
    for n in 6..=10u64 {
        let h = store_fake_block(&store, n, parent_hash_b).await;
        parent_hash_b = h;
        head_b_hash = h;
    }
    apply_fork_choice(&store, head_b_hash, finalized_hash, finalized_hash, None)
        .await
        .expect("depth-5 reorg within finality ceiling should succeed");

    // Deep reorg (depth 8): chain C diverges at block 3.
    // link = block 2 (canonical), reorg_depth = 10 - 2 = 8 > ceiling 7.
    // TooDeepReorg fires before the connectivity check (ceiling check is first).
    let (_, link2_hash) = chain_a[1]; // block 2
    let mut parent_hash_c = link2_hash;
    let mut head_c_hash = link2_hash;
    for n in 3..=10u64 {
        let h = store_fake_block(&store, n, parent_hash_c).await;
        parent_hash_c = h;
        head_c_hash = h;
    }
    // Re-establish chain A as canonical so is_canonical works correctly.
    apply_fork_choice(&store, head_a_hash, finalized_hash, finalized_hash, None)
        .await
        .expect("restore chain A canonical");
    let result = apply_fork_choice(&store, head_c_hash, finalized_hash, finalized_hash, None).await;
    assert!(
        matches!(
            result,
            Err(InvalidForkChoice::TooDeepReorg {
                reorg_depth: 8,
                limit: 7
            })
        ),
        "expected TooDeepReorg {{reorg_depth:8, limit:7}}, got {result:?}"
    );
}

/// Regression (PR #6724 CI, Hive Paris engine re-org tests): with no finalized block
/// and an empty journal (case 3 of `compute_reorg_ceiling`), a shallow reorg the node
/// can serve straight from its in-memory layer cache must be allowed. The former
/// `unwrap_or(0)` ceiling rejected every reorg on short, unfinalized chains with
/// `TooDeepReorg { limit: 0 }`, which is exactly what the Hive `N Block Re-Org (Paris)`
/// cases hit (short chain, no finality set). This builds 8 unfinalized blocks (journal
/// empty on the in-memory backend) and asserts a depth-5 reorg is accepted.
#[tokio::test]
async fn unfinalized_shallow_reorg_with_empty_journal_is_allowed() {
    let store = test_store().await;
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let blockchain = Blockchain::default_with_store(store.clone());

    let mut parent = genesis;
    let mut chain_a: Vec<(u64, H256)> = Vec::new();
    for _ in 0..8 {
        let block = new_block(&store, &parent).await;
        parent = block.header.clone();
        chain_a.push((block.header.number, block.hash()));
        blockchain.add_block(block).unwrap();
    }
    let (_, head_a) = *chain_a.last().unwrap();
    store
        .forkchoice_update(chain_a.clone(), 8, head_a, None, None)
        .await
        .unwrap();
    assert!(
        store.lowest_state_history_block_number().unwrap().is_none(),
        "journal must be empty (in-memory threshold not reached) to exercise case 3"
    );

    // Chain B diverges at block 4; link = block 3 (real, live state).
    // reorg_depth = latest(8) - 3 = 5, with no finalized/safe hash.
    let (_, link3) = chain_a[2];
    let mut phash = link3;
    let mut head_b = link3;
    for n in 4..=8u64 {
        let h = store_fake_block(&store, n, phash).await;
        phash = h;
        head_b = h;
    }
    apply_fork_choice(&store, head_b, H256::zero(), H256::zero(), None)
        .await
        .expect("depth-5 unfinalized reorg with empty journal must be allowed (case-3 physical ceiling), not TooDeepReorg");
    assert!(is_canonical(&store, 8, head_b).await.unwrap());
}

/// Task 2.7b: A reorg of depth 129 succeeds when finality ceiling is 129,
/// proving the old hardcoded 128-block cap is gone.
#[tokio::test]
async fn deep_reorg_beyond_legacy_128_cap_succeeds() {
    let store = test_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    // Build 2 real blocks so the link block (block 1) has a live state root.
    let blockchain = Blockchain::default_with_store(store.clone());
    let block_a1 = new_block(&store, &genesis_header).await;
    let hash_a1 = block_a1.hash();
    blockchain.add_block(block_a1.clone()).unwrap();
    let block_a2 = new_block(&store, &block_a1.header).await;
    let hash_a2 = block_a2.hash();
    blockchain.add_block(block_a2.clone()).unwrap();

    // Fake chain A: blocks 3..=130. Store headers only (no EVM).
    let mut canonical_blocks: Vec<(u64, H256)> = vec![(1, hash_a1), (2, hash_a2)];
    let mut parent_hash_a = hash_a2;
    for n in 3u64..=130 {
        let h = store_fake_block(&store, n, parent_hash_a).await;
        canonical_blocks.push((n, h));
        parent_hash_a = h;
    }
    let (_, head_a_hash) = *canonical_blocks.last().unwrap();
    // Set canonical chain A (latest = 130, finalized = block 1).
    // ceiling = 130 - 1 = 129.
    store
        .forkchoice_update(canonical_blocks.clone(), 130, head_a_hash, None, Some(1))
        .await
        .unwrap();

    // Chain B diverges at block 2 (parent = block 1 = A1).
    // new_canonical_blocks.last() = (2, hash_B2), canonical_link_height = 1,
    // reorg_depth = 130 - 1 = 129. ceiling = 129. 129 > 129 is false: SUCCEED.
    // The old hardcoded cap of 128 would have rejected this (129 > 128).
    let mut parent_hash_b = hash_a1;
    let mut head_b_hash = hash_a1;
    for n in 2u64..=130 {
        let h = store_fake_block(&store, n, parent_hash_b).await;
        parent_hash_b = h;
        head_b_hash = h;
    }
    apply_fork_choice(&store, head_b_hash, hash_a1, hash_a1, None)
        .await
        .expect("depth-129 reorg should succeed with finality ceiling 129 (old 128 cap lifted)");
}

/// Task 2.8: When no finalized block is known and the journal is empty (case 3 of
/// `compute_reorg_ceiling`), the ceiling is the operator-configured `max_reorg_depth`
/// (or 0 if unset). This test uses the operator override to simulate the pre-merge /
/// fresh-node scenario where the ceiling must be set explicitly.
///
/// NOTE: Testing case 2 (journal non-empty, no finalized) via the public `Store` API
/// would require the InMemory backend's commit threshold (10 000) to be reached,
/// which is impractical in a unit test. The ceiling formula
/// `latest - lowest_journal_block` is exercised by `compute_reorg_ceiling` directly
/// and is covered by the storage-layer unit tests. This test validates that the
/// operator-cap plumbing reaches `apply_fork_choice` correctly.
#[tokio::test]
async fn reorg_depth_bounded_by_journal_extent_pre_merge() {
    let store = test_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    // Build 10 real blocks on chain A (latest = 10). No finalized block (zero hash).
    // Journal is empty (InMemory threshold = 10 000 >> 10 blocks).
    // With max_reorg_depth = Some(7), ceiling = 7 (case 3: no journal, no finalized).
    let blockchain = Blockchain::default_with_store(store.clone());
    let mut parent = genesis_header.clone();
    let mut chain_a: Vec<(u64, H256)> = Vec::new();
    for _ in 0..10 {
        let block = new_block(&store, &parent).await;
        parent = block.header.clone();
        chain_a.push((block.header.number, block.hash()));
        blockchain.add_block(block).unwrap();
    }
    let (_, head_a_hash) = *chain_a.last().unwrap(); // block 10
    // Make chain A canonical with no finalized block.
    store
        .forkchoice_update(chain_a.clone(), 10, head_a_hash, None, None)
        .await
        .unwrap();

    // Confirm journal is empty (InMemory threshold not reached).
    assert!(
        store.lowest_state_history_block_number().unwrap().is_none(),
        "journal should be empty for InMemory store after only 10 blocks"
    );

    // Shallow reorg (depth 5 <= cap 7): chain B diverges at block 6, link = block 5.
    let (_, link5_hash) = chain_a[4]; // block 5
    let mut parent_hash_b = link5_hash;
    let mut head_b_hash = link5_hash;
    for n in 6u64..=10 {
        let h = store_fake_block(&store, n, parent_hash_b).await;
        parent_hash_b = h;
        head_b_hash = h;
    }
    // Case 3: finalized=zero, journal empty. ceiling = max_reorg_depth = 7.
    apply_fork_choice(&store, head_b_hash, H256::zero(), H256::zero(), Some(7))
        .await
        .expect("depth-5 reorg within operator-cap ceiling of 7 should succeed");

    // Deep reorg (depth 8 > cap 7): chain C diverges at block 3, link = block 2.
    // Restore chain A first (allow up to 10 depth to cover the chain-B-to-chain-A reorg).
    apply_fork_choice(&store, head_a_hash, H256::zero(), H256::zero(), Some(10))
        .await
        .expect("restore chain A");
    let (_, link2_hash) = chain_a[1]; // block 2
    let mut parent_hash_c = link2_hash;
    let mut head_c_hash = link2_hash;
    for n in 3u64..=10 {
        let h = store_fake_block(&store, n, parent_hash_c).await;
        parent_hash_c = h;
        head_c_hash = h;
    }
    // ceiling = 7; reorg_depth = 10 - 2 = 8 > 7 → TooDeepReorg.
    let result = apply_fork_choice(&store, head_c_hash, H256::zero(), H256::zero(), Some(7)).await;
    assert!(
        matches!(
            result,
            Err(InvalidForkChoice::TooDeepReorg {
                reorg_depth: 8,
                limit: 7
            })
        ),
        "depth-8 reorg exceeding cap-7 ceiling should return TooDeepReorg, got {result:?}"
    );
}

/// Regression test for the case-2 ceiling formula.
///
/// `compute_reorg_ceiling` case 2 fires when `finalized_hash` is zero but the
/// state-history journal is non-empty. Each entry at block N is the reverse-diff
/// that unwinds N -> N-1, so the deepest pivot the journal can reach is
/// `lowest - 1`, not `lowest`. The previous formula `latest - lowest`
/// under-allowed by 1: with `lowest = 4` and `latest = 10`, it returned 6 even
/// though the journal can support depth 7 (pivot at block 3).
///
/// This test seeds STATE_HISTORY directly (bypassing the layer-cache flush
/// threshold) and verifies the boundary: depth 7 must be accepted, depth 8 must
/// be rejected with TooDeepReorg{limit: 7}.
#[tokio::test]
async fn reorg_depth_bounded_by_journal_extent_case2() {
    let store = test_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    let blockchain = Blockchain::default_with_store(store.clone());
    let mut parent = genesis_header.clone();
    let mut chain_a: Vec<(u64, H256)> = Vec::new();
    for _ in 0..10 {
        let block = new_block(&store, &parent).await;
        parent = block.header.clone();
        chain_a.push((block.header.number, block.hash()));
        blockchain.add_block(block).unwrap();
    }
    let (_, head_a_hash) = *chain_a.last().unwrap();
    store
        .forkchoice_update(chain_a.clone(), 10, head_a_hash, None, None)
        .await
        .unwrap();

    // Seed STATE_HISTORY with entries for blocks 4..=10 (lowest = 4).
    // Values can be arbitrary; the ceiling check only reads keys via first_key.
    for n in 4u64..=10 {
        store.put_state_history_entry_for_test(n, &[0u8]).unwrap();
    }
    assert_eq!(
        store.lowest_state_history_block_number().unwrap(),
        Some(4),
        "seed setup: lowest STATE_HISTORY key must be 4"
    );

    // ceiling = latest - (lowest - 1) = 10 - 3 = 7. A reorg to chain B that
    // diverges at block 4 has link = block 3, reorg_depth = 10 - 3 = 7. With
    // the buggy formula (latest - lowest = 6) this would fail; with the fixed
    // formula it must succeed.
    let (_, link3_hash) = chain_a[2]; // block 3
    let mut parent_hash_b = link3_hash;
    let mut head_b_hash = link3_hash;
    for n in 4u64..=10 {
        let h = store_fake_block(&store, n, parent_hash_b).await;
        parent_hash_b = h;
        head_b_hash = h;
    }
    apply_fork_choice(&store, head_b_hash, H256::zero(), H256::zero(), None)
        .await
        .expect(
            "depth-7 reorg at journal floor should be ACCEPTED (case-2 ceiling = 7); \
             rejection here indicates the pre-fix off-by-one is back",
        );

    // Restore chain A so the canonical-state invariant holds for the next check.
    apply_fork_choice(&store, head_a_hash, H256::zero(), H256::zero(), Some(10))
        .await
        .expect("restore chain A");

    // depth = 8 (chain C diverges at block 3, link = block 2). This is past the
    // journal floor (lowest entry is 4 -> can't unwind block 3), so it must be
    // rejected with TooDeepReorg{limit: 7}.
    let (_, link2_hash) = chain_a[1]; // block 2
    let mut parent_hash_c = link2_hash;
    let mut head_c_hash = link2_hash;
    for n in 3u64..=10 {
        let h = store_fake_block(&store, n, parent_hash_c).await;
        parent_hash_c = h;
        head_c_hash = h;
    }
    let result = apply_fork_choice(&store, head_c_hash, H256::zero(), H256::zero(), None).await;
    assert!(
        matches!(
            result,
            Err(InvalidForkChoice::TooDeepReorg {
                reorg_depth: 8,
                limit: 7
            })
        ),
        "depth-8 reorg past journal floor should return TooDeepReorg{{8, 7}}, got {result:?}"
    );
}

/// Task 3.9: Verifies that `deep_reorg_attempts_total` increments when
/// `reorg_apply_deep` is entered. The InMemory store has an empty journal
/// (threshold = 10_000 >> blocks in test), so the deep path exits early with
/// `StateNotReachable` — but the attempt counter fires unconditionally at the
/// top of the function before any journal access.
#[serial_test::serial]
#[tokio::test]
async fn metrics_emitted_during_deep_reorg() {
    use ethrex_metrics::reorg::METRICS_REORG;

    let store = test_store().await;
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let genesis_hash = genesis_header.hash();

    // Chain A: one fake block at height 1 with a random state root so that
    // `has_state_root` returns false, triggering the deep-reorg path.
    let a1_state_root = H256::random();
    let a1_header = BlockHeader {
        number: 1,
        parent_hash: genesis_hash,
        state_root: a1_state_root,
        timestamp: 12,
        ..Default::default()
    };
    let a1_hash = Block::new(a1_header.clone(), BlockBody::default()).hash();
    store.add_block_header(a1_hash, a1_header).await.unwrap();
    store
        .forkchoice_update(vec![(1, a1_hash)], 1, a1_hash, None, None)
        .await
        .unwrap();

    // Chain B: fake block B1 at height 1 with a different random state root,
    // also diverging from genesis. Its parent is canonical (genesis) but its
    // own state root is not on disk, so `apply_fork_choice` returns
    // `StateNotReachable` and hands off to `reorg_apply_deep`.
    let b1_state_root = H256::random();
    let b1_header = BlockHeader {
        number: 1,
        parent_hash: genesis_hash,
        state_root: b1_state_root,
        timestamp: 24,
        ..Default::default()
    };
    let b1_hash = Block::new(b1_header.clone(), BlockBody::default()).hash();
    store.add_block_header(b1_hash, b1_header).await.unwrap();

    let before = METRICS_REORG.deep_reorg_attempts_total.get();

    // max_reorg_depth = Some(100) so the ceiling check (case 3: no finalized,
    // no journal) uses 100 instead of 0 and passes the depth-1 reorg.
    let blockchain = Blockchain::new(
        store.clone(),
        BlockchainOptions {
            max_reorg_depth: Some(100),
            ..Default::default()
        },
    );

    // The call enters `reorg_apply_deep` (incrementing the counter), then
    // fails because STATE_HISTORY is empty on the InMemory backend.
    let _ =
        apply_fork_choice_with_deep_reorg(&blockchain, b1_hash, H256::zero(), H256::zero()).await;

    assert!(
        METRICS_REORG.deep_reorg_attempts_total.get() > before,
        "deep_reorg_attempts_total should have incremented (before={before})"
    );
}

async fn new_block(store: &Store, parent: &BlockHeader) -> Block {
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::random(),
        random: H256::random(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::random()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };

    // Create blockchain
    let blockchain = Blockchain::default_with_store(store.clone());

    let block = create_payload(&args, store, Bytes::new()).unwrap();
    let result = blockchain.build_payload(block).unwrap();
    result.payload
}

/// Builds a block on `parent`, executes it, and stores it via `store_block` (the
/// L2 sequencer path) WITHOUT canonicalizing it: the block lands in the store and
/// advances `latest`, but `CANONICAL_BLOCK_HASHES` is left untouched, exactly like
/// `BlockProducer::produce_block`. Returns the stored block hash.
async fn store_block_l2_style(store: &Store, parent: &BlockHeader) -> H256 {
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::random(),
        random: H256::random(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::random()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let blockchain = Blockchain::default_with_store(store.clone());
    let block = create_payload(&args, store, Bytes::new()).unwrap();
    let result = blockchain.build_payload(block).unwrap();
    let block = result.payload;
    let block_hash = block.hash();
    let account_updates_list = store
        .apply_account_updates_batch(block.header.parent_hash, &result.account_updates)
        .unwrap()
        .expect("parent state must be present");
    let execution_result = BlockExecutionResult {
        receipts: result.receipts,
        requests: Vec::new(),
        block_gas_used: block.header.gas_used,
        tx_gas_breakdowns: Vec::new(),
    };
    blockchain
        .store_block(block, account_updates_list, execution_result)
        .unwrap();
    block_hash
}

/// Regression for PR #6724 review (ElFantasma, blocking): the L2 sequencer
/// canonicalizes every freshly produced block with
/// `apply_fork_choice(hash, hash, hash, None)` after `store_block` (which does
/// not touch `CANONICAL_BLOCK_HASHES`). That is a depth-1 canonical extend, but
/// passing the new block as its own `finalized` makes the finality ceiling
/// `latest - finalized = 0`. Without the floor-of-1 in `compute_reorg_ceiling`
/// the FCU is rejected with `TooDeepReorg { reorg_depth: 1, limit: 0 }` and every
/// sequencer block fails to canonicalize. This asserts the extend is allowed.
#[tokio::test]
async fn self_finalized_canonical_extend_is_allowed() {
    let store = test_store().await;
    let mut parent = store.get_block_header(0).unwrap().unwrap();

    for _ in 0..3 {
        let block_hash = store_block_l2_style(&store, &parent).await;
        apply_fork_choice(&store, block_hash, block_hash, block_hash, None)
            .await
            .expect("L2-style self-finalized canonical extend must be allowed");
        let header = store.get_block_header_by_hash(block_hash).unwrap().unwrap();
        assert!(
            is_canonical(&store, header.number, block_hash)
                .await
                .unwrap(),
            "self-finalized block at height {} should be canonical",
            header.number
        );
        parent = header;
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn test_store() -> Store {
    // Get genesis
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let genesis = serde_json::from_reader(reader).expect("Failed to deserialize genesis file");

    // Build store with genesis
    let mut store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");

    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");

    store
}
