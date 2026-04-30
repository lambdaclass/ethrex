use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    error::{ChainError, InvalidForkChoice},
    fork_choice::apply_fork_choice,
    is_canonical, latest_canonical_block_hash,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256,
    types::{Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER},
};
use ethrex_storage::{EngineType, Store};

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
    apply_fork_choice(&store, hash_1, H256::zero(), H256::zero())
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

    let fc_result = apply_fork_choice(&store, hash_2, H256::zero(), H256::zero()).await;
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
    apply_fork_choice(&store, hash_1b, genesis_hash, genesis_hash)
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
    apply_fork_choice(&store, hash_2, genesis_hash, genesis_hash)
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
    apply_fork_choice(&store, hash_3, hash_2, hash_2)
        .await
        .unwrap();

    assert!(is_canonical(&store, 1, hash_1).await.unwrap());
    assert!(is_canonical(&store, 2, hash_2).await.unwrap());
    assert!(is_canonical(&store, 3, hash_3).await.unwrap());

    // FCU to block 1 (ancestor of finalized): MUST be skipped.
    let result = apply_fork_choice(&store, hash_1, hash_1, hash_1).await;
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
    apply_fork_choice(&store, hash_2, genesis_hash, genesis_hash)
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
    apply_fork_choice(&store, hash_b, genesis_hash, genesis_hash)
        .await
        .unwrap();

    // The latest block should now be the new head.
    assert_eq!(latest_canonical_block_hash(&store).await.unwrap(), hash_b);
}

#[tokio::test]
async fn unfinalized_reorg_deeper_than_32_is_allowed() {
    // Per execution-apis PR 786, the -38006 TooDeepReorg rejection should only fire
    // when the FCU would replace blocks at or below the finalized prefix. A reorg
    // strictly within unfinalized history must be honored regardless of depth (up to
    // the implementation's state-history retention cap).
    //
    // Build two 33-block chains branching from genesis. With finalized = genesis,
    // the alternate chain's reorg depth (33) exceeds the previous limit (32) but
    // does not cross finalized, so the FCU must succeed.

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
    apply_fork_choice(&store, head_a, genesis_hash, genesis_hash)
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

    // FCU to chain B head: reorg depth = 33, finalized = genesis (height 0).
    // Pre-fix this would fail with `TooDeepReorg { reorg_depth: 33, limit: 32 }`.
    // Post-fix the spec check passes (canonical link is at height 0, not strictly
    // below finalized which is also 0) and the implementation cap (128) is not hit.
    apply_fork_choice(&store, head_b, genesis_hash, genesis_hash)
        .await
        .expect("33-block unfinalized reorg should be allowed");

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
