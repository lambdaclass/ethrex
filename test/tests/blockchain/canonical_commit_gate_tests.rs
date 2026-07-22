//! Wedge regression: non-canonical newPayload state must never overwrite the on-disk
//! genesis state root.
//!
//! The original wedge ("post-state for block 0 is absent ... resume_parent_number=0
//! local_head=0") happened when speculative blocks were committed to disk before any
//! forkchoice update made them canonical, pruning genesis. The canonical+depth commit
//! gate fixes this: while no FCU advances the canonical head, `safe_commit_root` stays
//! `H256::zero()`, `get_commitable` returns None, and nothing is flushed.
//!
//! This test imports blocks via the public `add_block` path WITHOUT calling
//! `forkchoice_update`, so the canonical head stays at genesis and the safe-commit cell
//! stays zero. We assert the genesis state root survives. The property under test is
//! "cell stays zero when no FCU canonicalizes", which holds for ANY layer count >= 1, so
//! ~5 blocks suffice; it is independent of the 10000-layer InMemory commit threshold,
//! because we are proving the NON-commit branch, not the commit branch.

use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER,
        Genesis, GenesisAccount, Transaction, TxKind,
    },
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{DB_COMMIT_THRESHOLD, EngineType, Store};
use secp256k1::SecretKey;

/// Test private key from fixtures/keys/private_keys_tests.txt.
const TEST_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
const TEST_MAX_FEE_PER_GAS: u64 = 10_000_000_000;
const TEST_GAS_LIMIT: u64 = 100_000;

fn test_secret_key() -> SecretKey {
    SecretKey::from_slice(&hex::decode(TEST_PRIVATE_KEY).unwrap()).unwrap()
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Load the execution-api genesis and fund `sender`. Returns the genesis and its chain id.
fn load_funded_genesis(sender: Address) -> (Genesis, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let mut genesis: Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");

    let chain_id = genesis.config.chain_id;

    genesis.alloc.insert(
        sender,
        GenesisAccount {
            balance: U256::from(10).pow(U256::from(20)), // 100 ETH
            code: Bytes::new(),
            storage: Default::default(),
            nonce: 0,
        },
    );

    (genesis, chain_id)
}

/// Load the execution-api genesis, fund `sender`, and return an in-memory store + chain id.
async fn setup_store(sender: Address) -> (Store, u64) {
    let (genesis, chain_id) = load_funded_genesis(sender);
    let mut store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    (store, chain_id)
}

/// Build a block on top of `parent_header`, including whatever is in the mempool.
async fn build_block(store: &Store, blockchain: &Blockchain, parent_header: &BlockHeader) -> Block {
    let args = BuildPayloadArgs {
        parent: parent_header.hash(),
        timestamp: parent_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };

    let block = create_payload(&args, store, Bytes::new()).unwrap();
    let result = blockchain.build_payload(block).unwrap();
    result.payload
}

fn sender_from_key(sk: &SecretKey) -> Address {
    LocalSigner::new(*sk).address
}

/// A simple value-transfer tx so each block changes state (a non-empty diff layer).
async fn transfer_tx(chain_id: u64, nonce: u64, signer: &Signer) -> Transaction {
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit: TEST_GAS_LIMIT,
        to: TxKind::Call(Address::from_low_u64_be(0xBEEF)),
        value: U256::from(1u64),
        data: Bytes::new(),
        ..Default::default()
    });
    tx.sign_inplace(signer).await.unwrap();
    tx
}

/// Import ~5 blocks via `add_block` (NO forkchoice_update). The canonical head never
/// advances past genesis, so `safe_commit_root` stays zero and nothing is ever flushed
/// to disk; the genesis state root must therefore still be present.
#[tokio::test]
async fn non_canonical_blocks_do_not_prune_genesis() {
    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let (store, chain_id) = setup_store(sender).await;
    let blockchain = Blockchain::default_with_store(store.clone());

    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let genesis_state_root = genesis_header.state_root;

    // Sanity: genesis state is on disk at the start.
    assert!(
        store
            .has_state_root(genesis_state_root)
            .expect("has_state_root genesis"),
        "precondition: genesis state must be present after add_initial_state"
    );

    // Build and import 5 blocks WITHOUT any forkchoice_update.
    let mut parent_header = genesis_header;
    for nonce in 0..5u64 {
        let tx = transfer_tx(chain_id, nonce, &signer).await;
        blockchain
            .add_transaction_to_pool(tx)
            .await
            .expect("tx should enter pool");

        let block = build_block(&store, &blockchain, &parent_header).await;
        blockchain
            .add_block(block.clone())
            .expect("block should be valid via single-block path");
        blockchain
            .remove_block_transactions_from_pool(&block)
            .expect("remove block txs from pool");
        parent_header = block.header;
    }

    // Precondition that makes the property load-bearing: no FCU ran, so the canonical
    // head is still genesis (block 0). safe_commit_root is therefore still zero.
    assert_eq!(
        store.get_latest_block_number().await.unwrap(),
        0,
        "canonical head must stay at genesis when no forkchoice_update is called"
    );

    // The canonical head was never advanced (no FCU), so safe_commit_root stayed zero,
    // get_commitable returned None, and no layer was committed: genesis is intact.
    assert!(
        store
            .has_state_root(genesis_state_root)
            .expect("has_state_root genesis after imports"),
        "genesis state_root must survive non-canonical imports (the wedge regression)"
    );
}

/// Best-effort removal of a test RocksDB directory (ignores absence / transient locks).
#[cfg(feature = "rocksdb")]
fn remove_test_db(path: &str) {
    let _ = std::fs::remove_dir_all(path);
}

/// Commit-on-forkchoice regression (hive devp2p `snap`/`AccountRange` Test 11).
///
/// `import` executes every block and *then* issues a single `forkchoice_update`. The
/// commit step (Phase 2 of the trie-update worker) only runs while blocks execute, so
/// before the fix the now-canonical backlog was never flushed: the path-keyed on-disk
/// state stayed frozen at genesis, and the snap server kept serving genesis state that
/// should be unavailable because it is older than the in-memory layer window. Hive's
/// `AccountRange` Test 11 asserts the genesis root returns no accounts; ethrex returned 27.
///
/// Here we import `> DB_COMMIT_THRESHOLD` blocks via `add_block` (nothing commits while
/// `safe_commit_root` is zero), then `forkchoice_update` to canonicalize them. After the
/// fix that advances `safe_commit_root` and pokes the worker to flush the backlog up to
/// `head - 128`, advancing the disk past genesis so the genesis root is no longer
/// serveable, while recent (head) state stays available.
///
/// RocksDB-only: it is the backend that uses `DB_COMMIT_THRESHOLD` (128); the InMemory
/// backend's threshold is 10000, too high to reach the commit branch with this few blocks.
#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn forkchoice_flushes_committable_backlog_and_prunes_genesis() {
    // Strictly greater than DB_COMMIT_THRESHOLD (128) so the canonical block at
    // `head - 128` exists and is a committable layer.
    const BLOCKS: u64 = 130;

    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let path = format!("commit-flush-test-db-{:x}", H256::random());
    remove_test_db(&path); // clean any stale dir from a previous failed run

    let (genesis, chain_id) = load_funded_genesis(sender);
    let mut store = Store::new(&path, EngineType::RocksDB).expect("Failed to build RocksDB store");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    let blockchain = Blockchain::default_with_store(store.clone());

    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let genesis_state_root = genesis_header.state_root;
    assert!(
        store
            .has_state_root(genesis_state_root)
            .expect("has_state_root genesis"),
        "precondition: genesis state must be present after add_initial_state"
    );

    // Import BLOCKS blocks via `add_block`, WITHOUT any forkchoice_update (mirrors `import`).
    let mut parent_header = genesis_header;
    let mut canonical: Vec<(u64, H256)> = Vec::with_capacity(BLOCKS as usize);
    for nonce in 0..BLOCKS {
        let tx = transfer_tx(chain_id, nonce, &signer).await;
        blockchain
            .add_transaction_to_pool(tx)
            .await
            .expect("tx should enter pool");

        let block = build_block(&store, &blockchain, &parent_header).await;
        blockchain
            .add_block(block.clone())
            .expect("block should be valid via single-block path");
        blockchain
            .remove_block_transactions_from_pool(&block)
            .expect("remove block txs from pool");
        canonical.push((block.header.number, block.hash()));
        parent_header = block.header;
    }
    let head_state_root = parent_header.state_root;

    // No FCU has run yet: safe_commit_root is still zero, nothing flushed, genesis present.
    assert!(
        store
            .has_state_root(genesis_state_root)
            .expect("has_state_root genesis pre-fcu"),
        "before forkchoice_update nothing is flushed: genesis must still be present"
    );

    // Canonicalize the imported chain exactly like `cli.rs` import does (one FCU at the end).
    let (head_number, head_hash) = canonical.pop().expect("at least one block imported");
    store
        .forkchoice_update(
            canonical,
            head_number,
            head_hash,
            Some(head_number),
            Some(head_number),
        )
        .await
        .expect("forkchoice_update");

    // The flush runs on the trie worker after the Commit message; wait until it is idle.
    store
        .wait_for_persistence_idle()
        .await
        .expect("wait_for_persistence_idle");

    // The fix: forkchoice advanced safe_commit_root and flushed the backlog up to
    // head - 128, advancing the path-keyed disk past genesis. The genesis state root is
    // therefore no longer serveable.
    assert!(
        !store
            .has_state_root(genesis_state_root)
            .expect("has_state_root genesis post-fcu"),
        "after forkchoice_update the committable backlog must flush and prune genesis \
         (regression: genesis stayed serveable because the commit was never triggered)"
    );
    // Recent state stays available: the head layer is retained in memory above the commit.
    assert!(
        store
            .has_state_root(head_state_root)
            .expect("has_state_root head post-fcu"),
        "recent (head) state must remain serveable after the flush"
    );

    drop(blockchain);
    drop(store);
    remove_test_db(&path);
}

/// Regression (OOM): startup state regeneration, full-sync block-by-block, and block
/// import re-execute a SINGLE canonical chain via `add_block_pipeline_bounded`, WITHOUT
/// issuing a forkchoice_update as they go. Before the fix these used the canonical
/// safe-commit gate: with `safe_commit_root` stuck at zero (nothing canonicalized),
/// `get_commitable` returned None and NO layer ever committed, so the in-memory
/// trie-layer backlog grew with the re-execution gap until the process ran out of memory
/// (~60 GB on an ~11k-block regen). The fix routes these paths through the depth gate
/// (`add_block_pipeline_bounded(.., DB_COMMIT_THRESHOLD)`), which commits by depth
/// regardless of the safe-commit cell, keeping ~128 layers resident.
///
/// This drives that exact shape — >128 blocks through the bounded path, no FCU — and
/// asserts both properties end-to-end on real execution:
///   (a) BOUNDED: with NO forkchoice_update the backlog is still flushed forward, so the
///       genesis root is pruned/clobbered off disk. Under the buggy canonical gate
///       nothing flushes without an FCU, so genesis would survive — this assertion fails
///       if a re-exec path is reverted to `add_block_pipeline` (the canonical gate).
///   (b) WINDOW RETAINED, NO CLOBBER: the recent `DB_COMMIT_THRESHOLD` layers stay in
///       memory and serve CORRECT historical state — a balance query at a mid-window root
///       returns that block's value, not the latest/disk root's (the historical-window
///       clobber the retained window guards against).
///
/// RocksDB-only: it is the backend whose disk commit path-clobbers older roots, which is
/// what makes (a)'s `has_state_root(genesis) == false` load-bearing.
#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn bounded_reexec_without_fcu_bounds_memory_and_serves_window() {
    // Comfortably above DB_COMMIT_THRESHOLD (128) so the depth gate fires, the flush
    // boundary lands well past genesis (~head-128), and a mid-window block stays resident.
    const BLOCKS: u64 = 135;

    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();
    let recipient = Address::from_low_u64_be(0xBEEF); // transfer_tx sends 1 wei/block here

    let path = format!("bounded-reexec-test-db-{:x}", H256::random());
    remove_test_db(&path); // clean any stale dir from a previous failed run

    let (genesis, chain_id) = load_funded_genesis(sender);
    let mut store = Store::new(&path, EngineType::RocksDB).expect("Failed to build RocksDB store");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    let blockchain = Blockchain::default_with_store(store.clone());

    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let genesis_state_root = genesis_header.state_root;
    assert!(
        store
            .has_state_root(genesis_state_root)
            .expect("has_state_root genesis"),
        "precondition: genesis state must be present after add_initial_state"
    );

    // Re-execute a single canonical chain through the BOUNDED path, block by block, with
    // NO forkchoice_update — exactly what regenerate_head_state / run_blocks_pipeline /
    // import do. Record each block's state root to query the retained window later.
    let mut parent_header = genesis_header;
    let mut roots: Vec<H256> = Vec::with_capacity(BLOCKS as usize); // roots[N-1] == block N
    for nonce in 0..BLOCKS {
        let tx = transfer_tx(chain_id, nonce, &signer).await;
        blockchain
            .add_transaction_to_pool(tx)
            .await
            .expect("tx should enter pool");

        let block = build_block(&store, &blockchain, &parent_header).await;
        blockchain
            .add_block_pipeline_bounded(block.clone(), None, DB_COMMIT_THRESHOLD)
            .expect("block should import via the bounded re-exec path");
        blockchain
            .remove_block_transactions_from_pool(&block)
            .expect("remove block txs from pool");
        roots.push(block.header.state_root);
        parent_header = block.header;
    }
    let head_state_root = parent_header.state_root;

    // The bounded path acks after flush; make sure the worker has drained the backlog.
    store
        .wait_for_persistence_idle()
        .await
        .expect("wait_for_persistence_idle");

    // No FCU ever ran: the canonical head is still genesis, so the safe-commit cell stayed
    // zero — the canonical gate would have committed nothing.
    assert_eq!(
        store.get_latest_block_number().await.unwrap(),
        0,
        "precondition: no forkchoice_update was issued, canonical head stays at genesis"
    );

    // (a) BOUNDED — the depth gate flushed the backlog forward WITHOUT any FCU, clobbering
    // the genesis root off disk. This is the discriminator: the buggy canonical gate would
    // flush nothing here (safe_commit_root == 0), leaving genesis serveable and memory
    // growing without bound.
    assert!(
        !store
            .has_state_root(genesis_state_root)
            .expect("has_state_root genesis post-import"),
        "depth gate must flush the backlog without an FCU (regression: canonical gate \
         leaves safe_commit_root at zero, nothing commits, and memory grows to OOM)"
    );

    // (b) WINDOW RETAINED + NO CLOBBER — the recent window stays resident and serves the
    // correct historical state. A mid-window block sits well inside the retained ~128
    // layers (mid > BLOCKS - DB_COMMIT_THRESHOLD).
    let mid = BLOCKS - 40; // block number
    let mid_root = roots[(mid - 1) as usize];
    assert!(
        store
            .has_state_root(mid_root)
            .expect("has_state_root mid-window"),
        "a mid-window layer must stay resident in memory"
    );
    assert!(
        store
            .has_state_root(head_state_root)
            .expect("has_state_root head"),
        "the head layer must stay resident in memory"
    );

    // The recipient receives exactly 1 wei per block, so its balance at block N's root is
    // N. Distinct, correct per-root balances prove the window is not clobbered to the
    // latest/disk root.
    let mid_balance = store
        .get_account_state_by_root(mid_root, recipient)
        .expect("read recipient at mid-window root")
        .expect("recipient account exists at mid-window root")
        .balance;
    assert_eq!(
        mid_balance,
        U256::from(mid),
        "mid-window root must serve that block's balance, not the latest root's"
    );
    let head_balance = store
        .get_account_state_by_root(head_state_root, recipient)
        .expect("read recipient at head root")
        .expect("recipient account exists at head root")
        .balance;
    assert_eq!(
        head_balance,
        U256::from(BLOCKS),
        "head root must serve the full accrued transfers"
    );

    drop(blockchain);
    drop(store);
    remove_test_db(&path);
}

/// Build and store a linear canonical chain of `n` blocks (one 1-wei transfer to the 0xBEEF
/// recipient each) in `store`, returning them in order. Used to generate a chain to replay
/// through `add_blocks_in_batch` on a fresh store.
async fn build_stored_chain(
    store: &Store,
    blockchain: &Blockchain,
    chain_id: u64,
    signer: &Signer,
    n: u64,
) -> Vec<Block> {
    let mut parent_header = store.get_block_header(0).unwrap().unwrap();
    let mut blocks = Vec::with_capacity(n as usize);
    for nonce in 0..n {
        let tx = transfer_tx(chain_id, nonce, signer).await;
        blockchain
            .add_transaction_to_pool(tx)
            .await
            .expect("tx should enter pool");
        let block = build_block(store, blockchain, &parent_header).await;
        blockchain
            .add_block(block.clone())
            .expect("store scratch block");
        blockchain
            .remove_block_transactions_from_pool(&block)
            .expect("remove block txs from pool");
        parent_header = block.header.clone();
        blocks.push(block);
    }
    blocks
}

/// Distance-gated sync commit — SAFETY: the reorg window stays resident.
///
/// `add_blocks_in_batch` must never commit a block within `REORG_DEPTH_LIMIT` of the sync target
/// to the path-keyed disk — doing so would clobber the recent-state window that reorg handling and
/// snap/historical queries read from. Replaying a chain with the target at its tip, every block in
/// the reorg window must stay serveable from memory, while bulk blocks far behind are written
/// through and clobbered off disk.
#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn distance_gated_sync_keeps_reorg_window_resident() {
    use ethrex_blockchain::fork_choice::REORG_DEPTH_LIMIT;
    use tokio_util::sync::CancellationToken;

    // Past the warmup (2 * REORG_DEPTH_LIMIT) so the chain has a genuine write-through zone.
    let target: u64 = 2 * REORG_DEPTH_LIMIT + 24;

    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();
    let recipient = Address::from_low_u64_be(0xBEEF);

    // Generate the canonical chain on a scratch store.
    let (gen_store, chain_id) = setup_store(sender).await;
    let gen_bc = Blockchain::default_with_store(gen_store.clone());
    let blocks = build_stored_chain(&gen_store, &gen_bc, chain_id, &signer, target).await;
    let root_of = |n: u64| blocks[(n - 1) as usize].header.state_root;

    // Replay the whole chain through the distance-gated batch path on a fresh RocksDB store.
    let path = format!("dist-gate-window-{:x}", H256::random());
    remove_test_db(&path);
    let (genesis, _) = load_funded_genesis(sender);
    let mut store = Store::new(&path, EngineType::RocksDB).expect("rocksdb store");
    store.add_initial_state(genesis).await.expect("genesis");
    let blockchain = Blockchain::default_with_store(store.clone());
    blockchain
        .add_blocks_in_batch(blocks.clone(), &[], CancellationToken::new(), target)
        .await
        .expect("add_blocks_in_batch");
    store
        .wait_for_persistence_idle()
        .await
        .expect("wait_for_persistence_idle");

    // (safety) every block within REORG_DEPTH_LIMIT of the target must stay resident; if the
    // disk floor advanced into this range it would have clobbered these roots.
    for n in (target - REORG_DEPTH_LIMIT + 1)..=target {
        assert!(
            store
                .has_state_root(root_of(n))
                .expect("has_state_root window"),
            "block {n} is within REORG_DEPTH_LIMIT of the target and must stay resident, not committed to disk"
        );
    }
    // a bulk block far behind the target must be committed and clobbered off disk (bounded memory).
    assert!(
        !store
            .has_state_root(root_of(50))
            .expect("has_state_root bulk"),
        "a bulk block far behind the target must be committed+clobbered off disk"
    );
    // the resident window serves correct historical state (recipient accrues 1 wei/block).
    let mid = target - 64;
    let bal = store
        .get_account_state_by_root(root_of(mid), recipient)
        .expect("read recipient")
        .expect("recipient exists")
        .balance;
    assert_eq!(
        bal,
        U256::from(mid),
        "mid-window root must serve that block's balance"
    );

    drop(blockchain);
    drop(store);
    remove_test_db(&path);
}

/// Distance-gated sync commit — BEHAVIOR: blocks far from the target write straight through.
///
/// With the target set far ahead (every block beyond `SYNC_RETAIN_WARMUP`), the batch path writes
/// each layer straight to disk instead of retaining a `DB_COMMIT_THRESHOLD` window: a block a few
/// back from the tip is committed and clobbered, where a full window would keep it resident.
/// Together with the window test above this pins the gate from both sides — neither an
/// always-write-through nor an always-retain implementation passes both.
#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn distance_gated_sync_writes_through_far_from_target() {
    use ethrex_blockchain::fork_choice::SYNC_RETAIN_WARMUP;
    use tokio_util::sync::CancellationToken;

    let n: u64 = 150;
    // Target far ahead so every processed block is beyond the warmup -> all write-through.
    let far_target = n + SYNC_RETAIN_WARMUP + 100;

    let sk = test_secret_key();
    let sender = sender_from_key(&sk);
    let signer: Signer = LocalSigner::new(sk).into();

    let (gen_store, chain_id) = setup_store(sender).await;
    let gen_bc = Blockchain::default_with_store(gen_store.clone());
    let blocks = build_stored_chain(&gen_store, &gen_bc, chain_id, &signer, n).await;
    let root_of = |b: u64| blocks[(b - 1) as usize].header.state_root;

    let path = format!("dist-gate-writethrough-{:x}", H256::random());
    remove_test_db(&path);
    let (genesis, _) = load_funded_genesis(sender);
    let mut store = Store::new(&path, EngineType::RocksDB).expect("rocksdb store");
    store.add_initial_state(genesis).await.expect("genesis");
    let blockchain = Blockchain::default_with_store(store.clone());
    blockchain
        .add_blocks_in_batch(blocks.clone(), &[], CancellationToken::new(), far_target)
        .await
        .expect("add_blocks_in_batch");
    store
        .wait_for_persistence_idle()
        .await
        .expect("wait_for_persistence_idle");

    // Write-through keeps ~1 layer resident: the tip is serveable, but a block a few back is
    // committed and clobbered -- a DB_COMMIT_THRESHOLD window would have kept it resident.
    assert!(
        store
            .has_state_root(root_of(n))
            .expect("has_state_root tip"),
        "the tip must be serveable"
    );
    assert!(
        !store
            .has_state_root(root_of(n - 10))
            .expect("has_state_root back"),
        "far from the target, blocks must be written straight through (a full window would retain this)"
    );

    drop(blockchain);
    drop(store);
    remove_test_db(&path);
}
