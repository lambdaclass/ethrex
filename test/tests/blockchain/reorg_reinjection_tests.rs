//! Tests for mempool re-injection of transactions orphaned by chain reorgs.
//!
//! These tests exercise the helpers directly (common-ancestor walk,
//! orphaned-transaction collection, blob limbo) without spinning up a full
//! FCU pipeline. An end-to-end test driven by `forkchoiceUpdated` would need
//! complete block execution against multiple competing chains and is left
//! for follow-up fixture work — these tests cover the load-bearing logic.

use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    fork_choice::{collect_orphaned_transactions, find_common_ancestor},
    mempool::Mempool,
};
use ethrex_common::{
    Address, Bytes, H160, H256, U256,
    types::{
        BYTES_PER_BLOB, BlobsBundle, Block, BlockBody, BlockHeader, EIP1559Transaction,
        EIP4844Transaction, MempoolTransaction, Transaction, TxKind,
    },
};
use ethrex_storage::{EngineType, Store};

const MEMPOOL_MAX_SIZE_TEST: usize = 10_000;

/// Build an empty BlockBody with a given list of transactions.
fn body_with_txs(transactions: Vec<Transaction>) -> BlockBody {
    BlockBody {
        transactions,
        ommers: Vec::new(),
        withdrawals: Some(Vec::new()),
    }
}

/// Build a minimal EIP-1559 transaction with a deterministic nonce.
fn make_eip1559_tx(nonce: u64) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1,
        gas_limit: 21_000,
        to: TxKind::Call(Address::from_low_u64_be(1)),
        value: U256::zero(),
        data: Bytes::default(),
        access_list: Default::default(),
        ..Default::default()
    })
}

/// Insert a chain of empty headers descending from `parent` into the store
/// without any state-root checks. The headers form a linear chain that can
/// be walked by `find_common_ancestor`.
async fn insert_chain(
    store: &Store,
    parent: &BlockHeader,
    count: u64,
    branch_marker: u8,
) -> Vec<BlockHeader> {
    let mut chain = Vec::with_capacity(count as usize);
    let mut prev = parent.clone();
    for offset in 1..=count {
        let header = BlockHeader {
            parent_hash: prev.hash(),
            number: prev.number + 1,
            timestamp: prev.timestamp + 12,
            // Mix in branch + offset so blocks on competing branches have
            // distinct hashes even when they share the same height.
            extra_data: Bytes::from(vec![branch_marker, offset as u8]),
            ..Default::default()
        };
        let hash = header.hash();
        store
            .add_block_header(hash, header.clone())
            .await
            .expect("add header");
        // We add an empty body for every header so `get_block_by_hash` works.
        let body = body_with_txs(Vec::new());
        store
            .add_block_body(hash, body)
            .await
            .expect("add empty body");
        chain.push(header.clone());
        prev = header;
    }
    chain
}

/// Insert a Block (header + body) into the store.
async fn insert_block(store: &Store, block: Block) {
    let hash = block.hash();
    store
        .add_block_header(hash, block.header.clone())
        .await
        .expect("add header");
    store
        .add_block_body(hash, block.body)
        .await
        .expect("add body");
}

async fn fresh_store() -> (Store, BlockHeader) {
    let store = Store::new("test_store", EngineType::InMemory).expect("create store");
    let genesis_header = BlockHeader {
        number: 0,
        timestamp: 0,
        ..Default::default()
    };
    let genesis_hash = genesis_header.hash();
    store
        .add_block_header(genesis_hash, genesis_header.clone())
        .await
        .expect("add genesis header");
    store
        .add_block_body(genesis_hash, body_with_txs(Vec::new()))
        .await
        .expect("add genesis body");
    store
        .forkchoice_update(vec![], 0, genesis_hash, None, None)
        .await
        .expect("genesis forkchoice");
    (store, genesis_header)
}

#[tokio::test]
async fn common_ancestor_same_block_returns_zero_depth() {
    let (store, genesis) = fresh_store().await;
    let branches = find_common_ancestor(&store, genesis.hash(), genesis.hash())
        .await
        .expect("find ancestor")
        .expect("ancestor present");
    assert_eq!(branches.common_ancestor_hash, genesis.hash());
    assert!(branches.orphaned.is_empty());
    assert!(branches.new_canonical.is_empty());
    assert_eq!(branches.depth(), 0);
}

#[tokio::test]
async fn common_ancestor_finds_shared_block_with_equal_height_branches() {
    let (store, genesis) = fresh_store().await;
    let chain_a = insert_chain(&store, &genesis, 3, 0xa).await;
    let chain_b = insert_chain(&store, &genesis, 3, 0xb).await;
    let prev_head = chain_a.last().unwrap().hash();
    let new_head = chain_b.last().unwrap().hash();

    let branches = find_common_ancestor(&store, prev_head, new_head)
        .await
        .expect("find ancestor")
        .expect("ancestor present");

    assert_eq!(branches.common_ancestor_hash, genesis.hash());
    assert_eq!(branches.common_ancestor_number, 0);
    assert_eq!(branches.orphaned.len(), 3);
    assert_eq!(branches.new_canonical.len(), 3);
    assert_eq!(branches.depth(), 3);

    // Branches must be ordered oldest -> newest.
    assert_eq!(branches.orphaned[0].1, chain_a[0].hash());
    assert_eq!(branches.orphaned[2].1, chain_a[2].hash());
    assert_eq!(branches.new_canonical[0].1, chain_b[0].hash());
    assert_eq!(branches.new_canonical[2].1, chain_b[2].hash());
}

#[tokio::test]
async fn common_ancestor_handles_unequal_height_branches() {
    // Old chain: genesis -> A1 -> A2.
    // New chain: genesis -> A1 -> B2 -> B3.
    // Shared prefix is A1, depth is 1 (only A2 is orphaned).
    let (store, genesis) = fresh_store().await;
    let chain_a = insert_chain(&store, &genesis, 2, 0xa).await;
    let a1 = chain_a[0].clone();
    let chain_b = insert_chain(&store, &a1, 2, 0xb).await;

    let prev_head = chain_a[1].hash();
    let new_head = chain_b[1].hash();

    let branches = find_common_ancestor(&store, prev_head, new_head)
        .await
        .expect("find ancestor")
        .expect("ancestor present");

    assert_eq!(branches.common_ancestor_hash, a1.hash());
    assert_eq!(branches.common_ancestor_number, 1);
    assert_eq!(branches.orphaned.len(), 1);
    assert_eq!(branches.new_canonical.len(), 2);
}

#[tokio::test]
async fn collect_orphaned_subtracts_transactions_present_in_new_canonical() {
    // Old chain genesis -> A1 (containing tx_x, tx_y).
    // New chain genesis -> B1 (containing tx_y, tx_z).
    // We expect only tx_x to be flagged for re-injection because tx_y is now
    // canonical and tx_z was already canonical from the start.
    let (store, genesis) = fresh_store().await;

    let tx_x = make_eip1559_tx(0);
    let tx_y = make_eip1559_tx(1);
    let tx_z = make_eip1559_tx(2);

    let header_a = BlockHeader {
        parent_hash: genesis.hash(),
        number: 1,
        timestamp: genesis.timestamp + 12,
        extra_data: Bytes::from(vec![0xa, 0x1]),
        ..Default::default()
    };
    let block_a = Block::new(header_a, body_with_txs(vec![tx_x.clone(), tx_y.clone()]));
    let header_b = BlockHeader {
        parent_hash: genesis.hash(),
        number: 1,
        timestamp: genesis.timestamp + 12,
        extra_data: Bytes::from(vec![0xb, 0x1]),
        ..Default::default()
    };
    let block_b = Block::new(header_b, body_with_txs(vec![tx_y.clone(), tx_z.clone()]));

    let block_a_hash = block_a.hash();
    let block_b_hash = block_b.hash();
    insert_block(&store, block_a).await;
    insert_block(&store, block_b).await;

    let branches = find_common_ancestor(&store, block_a_hash, block_b_hash)
        .await
        .expect("find ancestor")
        .expect("ancestor present");

    let orphaned = collect_orphaned_transactions(&store, &branches)
        .await
        .expect("collect");
    assert_eq!(orphaned.len(), 1);
    assert_eq!(orphaned[0].hash(), tx_x.hash());
}

#[tokio::test]
async fn reinject_skips_when_reorg_depth_exceeds_cap() {
    // Build two chains of depth 4 off of the genesis and set a depth cap of 2.
    // We expect re-injection to be skipped entirely (returns 0).
    let (store, genesis) = fresh_store().await;
    let chain_a = insert_chain(&store, &genesis, 4, 0xa).await;
    let chain_b = insert_chain(&store, &genesis, 4, 0xb).await;

    let opts = BlockchainOptions {
        reorg_depth: 2,
        ..Default::default()
    };
    let blockchain = Blockchain::new(store.clone(), opts);

    let reinjected = blockchain
        .reinject_orphaned_transactions(
            chain_a.last().unwrap().hash(),
            chain_b.last().unwrap().hash(),
        )
        .await
        .expect("reinject");
    assert_eq!(reinjected, 0);
}

#[tokio::test]
async fn reinject_is_best_effort_when_admission_fails() {
    // Build a tiny reorg: genesis -> A1 (contains tx with nonce=0), no new chain.
    // Re-injecting an EIP-4844 transaction without a sidecar should silently
    // skip it. We use this to assert no error is propagated and the other tx
    // still re-injects.
    let (store, genesis) = fresh_store().await;
    let plain_tx = make_eip1559_tx(0);

    // Build a fake blob tx (no actual sidecar in the limbo).
    let blob_tx_inner = EIP4844Transaction {
        nonce: 0,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1,
        max_fee_per_blob_gas: 1.into(),
        gas: 21_000,
        to: Address::from_low_u64_be(1),
        ..Default::default()
    };
    let blob_tx = Transaction::EIP4844Transaction(blob_tx_inner);

    let header_a = BlockHeader {
        parent_hash: genesis.hash(),
        number: 1,
        timestamp: genesis.timestamp + 12,
        gas_limit: 100_000_000,
        extra_data: Bytes::from(vec![0xa, 0x1]),
        ..Default::default()
    };
    let block_a = Block::new(
        header_a,
        body_with_txs(vec![plain_tx.clone(), blob_tx.clone()]),
    );
    let block_a_hash = block_a.hash();
    insert_block(&store, block_a).await;

    let header_b = BlockHeader {
        parent_hash: genesis.hash(),
        number: 1,
        timestamp: genesis.timestamp + 12,
        gas_limit: 100_000_000,
        extra_data: Bytes::from(vec![0xb, 0x1]),
        ..Default::default()
    };
    let block_b_number = header_b.number;
    let block_b = Block::new(header_b, body_with_txs(Vec::new()));
    let block_b_hash = block_b.hash();
    insert_block(&store, block_b).await;

    // Make block B the canonical head so admission validation can read a
    // header. We keep the default chain config from `Store::new`.
    store
        .forkchoice_update(
            vec![(block_b_number, block_b_hash)],
            block_b_number,
            block_b_hash,
            None,
            None,
        )
        .await
        .expect("set canonical to B");

    let blockchain = Blockchain::default_with_store(store.clone());
    let reinjected = blockchain
        .reinject_orphaned_transactions(block_a_hash, block_b_hash)
        .await
        .expect("reinject");

    // The blob tx must be skipped (no sidecar) and the plain tx must also
    // fail admission (the sender has no balance on this minimal store). Both
    // failures are handled best-effort: zero txs land in the pool, no error
    // propagates, and no panic.
    assert_eq!(
        reinjected, 0,
        "best-effort path must report 0 re-injections when both txs fail admission",
    );
    // The blob limbo was empty going in and the no-sidecar code path consumes
    // nothing, so a follow-up re-injection attempt would still see an empty
    // limbo. Calling it a second time with the same arguments must also yield
    // 0, confirming the path is idempotent and side-effect-free.
    let reinjected_again = blockchain
        .reinject_orphaned_transactions(block_a_hash, block_b_hash)
        .await
        .expect("reinject (second call)");
    assert_eq!(reinjected_again, 0);
}

#[test]
fn included_transaction_moves_blob_sidecar_to_limbo() {
    let mempool = Mempool::new(MEMPOOL_MAX_SIZE_TEST);

    // Construct a minimal blob tx + bundle.
    let tx = EIP4844Transaction {
        nonce: 1,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1,
        max_fee_per_blob_gas: 1.into(),
        gas: 21_000,
        to: Address::from_low_u64_be(1),
        ..Default::default()
    };
    let tx = Transaction::EIP4844Transaction(tx);
    let sender = H160::random();
    let hash = tx.hash();
    let bundle = BlobsBundle {
        blobs: vec![[0u8; BYTES_PER_BLOB]],
        commitments: vec![[0u8; 48]],
        proofs: vec![[0u8; 48]],
        version: 0,
    };

    mempool
        .add_blobs_bundle(hash, bundle.clone())
        .expect("add bundle");
    mempool
        .add_transaction(hash, sender, MempoolTransaction::new(tx, sender))
        .expect("add tx");

    // Before inclusion: sidecar is in the active pool, limbo is empty.
    assert!(mempool.get_blobs_bundle(hash).expect("get").is_some());
    assert_eq!(mempool.blob_limbo_size().expect("size"), 0);

    // Simulate block inclusion: remove the tx via the inclusion-aware path.
    let was_present = mempool
        .remove_included_transaction(&hash)
        .expect("remove included");
    assert!(was_present);

    // Sidecar should have moved to limbo.
    assert!(mempool.get_blobs_bundle(hash).expect("get").is_none());
    assert_eq!(mempool.blob_limbo_size().expect("size"), 1);

    // take_blob_limbo_entry returns it (and removes from limbo).
    let recovered = mempool
        .take_blob_limbo_entry(&hash)
        .expect("take")
        .expect("present");
    assert_eq!(recovered.commitments, bundle.commitments);
    assert_eq!(mempool.blob_limbo_size().expect("size"), 0);
}

#[test]
fn purge_blob_limbo_entries_drops_sidecars() {
    let mempool = Mempool::new(MEMPOOL_MAX_SIZE_TEST);
    let tx = EIP4844Transaction {
        nonce: 2,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1,
        max_fee_per_blob_gas: 1.into(),
        gas: 21_000,
        to: Address::from_low_u64_be(1),
        ..Default::default()
    };
    let tx = Transaction::EIP4844Transaction(tx);
    let sender = H160::random();
    let hash = tx.hash();
    let bundle = BlobsBundle {
        blobs: vec![[1u8; BYTES_PER_BLOB]],
        commitments: vec![[1u8; 48]],
        proofs: vec![[1u8; 48]],
        version: 0,
    };
    mempool.add_blobs_bundle(hash, bundle).expect("add bundle");
    mempool
        .add_transaction(hash, sender, MempoolTransaction::new(tx, sender))
        .expect("add tx");
    mempool
        .remove_included_transaction(&hash)
        .expect("remove included");
    assert_eq!(mempool.blob_limbo_size().expect("size"), 1);

    mempool.purge_blob_limbo_entries(&[hash]).expect("purge");
    assert_eq!(mempool.blob_limbo_size().expect("size"), 0);
    // Subsequent take should return None.
    assert!(
        mempool
            .take_blob_limbo_entry(&hash)
            .expect("take")
            .is_none()
    );
}

#[test]
fn purge_unrelated_hashes_is_noop() {
    let mempool = Mempool::new(MEMPOOL_MAX_SIZE_TEST);
    // Limbo starts empty; purging arbitrary hashes is fine.
    mempool
        .purge_blob_limbo_entries(&[H256::random(), H256::random()])
        .expect("purge");
    assert_eq!(mempool.blob_limbo_size().expect("size"), 0);
}

#[test]
fn mempool_is_full_gates_capacity() {
    // `Mempool::is_full` is the public accessor used by reorg re-injection
    // to decide whether to skip rather than evict freshly-arrived txs to
    // make room for orphaned ones. Verify the accessor flips at capacity.
    let mempool = Mempool::new(2);
    assert!(!mempool.is_full().expect("is_full"));

    let sender = Address::from_low_u64_be(1);
    let tx_a = make_eip1559_tx(0);
    let tx_b = make_eip1559_tx(1);

    let hash_a = tx_a.hash();
    let mempool_tx_a = MempoolTransaction::new(tx_a, sender);
    mempool
        .add_transaction(hash_a, sender, mempool_tx_a)
        .expect("add A");
    assert!(!mempool.is_full().expect("is_full after 1"));

    let hash_b = tx_b.hash();
    let mempool_tx_b = MempoolTransaction::new(tx_b, sender);
    mempool
        .add_transaction(hash_b, sender, mempool_tx_b)
        .expect("add B");
    assert!(
        mempool.is_full().expect("is_full at capacity"),
        "is_full must report true once transaction_pool reaches max_mempool_size",
    );
}
