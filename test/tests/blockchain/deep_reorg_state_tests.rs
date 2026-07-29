//! Deep-reorg state reachability: after a deep-reorg apply the pivot's state must stay
//! reconstructible, and an already-canonical head whose state was evicted must become
//! readable again.
//!
//! Both properties are about what `STATE_HISTORY` + disk can reproduce once the layer
//! cache has been swapped out, so they need the backend that actually reaches the commit
//! cadence: RocksDB (`DB_COMMIT_THRESHOLD` = 128). The InMemory threshold is 10 000,
//! far past what a test can build.

#![cfg(feature = "rocksdb")]

use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    fork_choice::apply_fork_choice_with_deep_reorg,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256,
    types::{BlockHeader, BlockNumber, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Genesis},
};
use ethrex_storage::{EngineType, Store};
use ethrex_trie::{Nibbles, Node, Trie};
use tempfile::TempDir;

/// Blocks on the original chain. Strictly greater than `DB_COMMIT_THRESHOLD` (128) so the
/// forkchoice update flushes blocks 1 and 2 to disk and evicts their layers.
const CHAIN_LEN: BlockNumber = 130;

/// The pivot for both tests: block 1's state is flushed to disk by the chain-A forkchoice
/// update and then overwritten by block 2's commit, so it is reachable only through the
/// journal.
const PIVOT: BlockNumber = 1;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn load_genesis() -> Genesis {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("open genesis file");
    serde_json::from_reader(BufReader::new(file)).expect("deserialize genesis file")
}

/// Open a RocksDB store in a temporary directory. The returned `TempDir` owns the data
/// directory; keep it alive for as long as the store is used.
async fn setup_store() -> (Store, TempDir) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let mut store = Store::new(dir.path(), EngineType::RocksDB).expect("build RocksDB store");
    store
        .add_initial_state(load_genesis())
        .await
        .expect("add genesis state");

    // Drive flat-KV generation to completion before any block commits, exactly as the node
    // initializer does. Journal entries written while generation is still running omit
    // past-frontier flat-KV pre-images, so an overlay built from them serves stale values
    // and the deep-reorg path refuses to run at all (issue #7001).
    store
        .generate_flatkeyvalue()
        .expect("trigger flat-KV generation");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !store
        .flatkeyvalue_fully_generated()
        .expect("read flat-KV completion marker")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "flat-KV generation did not finish within 30s"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    (store, dir)
}

/// Build and execute blocks `parent.number + 1 ..= up_to` on top of `parent`, returning the
/// headers in ascending order.
///
/// `timestamp_step` distinguishes forks: the EIP-4788 beacon-roots contract keys its ring
/// buffer by timestamp, so two forks built with different steps write different slots and
/// therefore reach different state roots from the same parent.
async fn extend_fork(
    store: &Store,
    blockchain: &Blockchain,
    parent: &BlockHeader,
    up_to: BlockNumber,
    timestamp_step: u64,
) -> Vec<BlockHeader> {
    let mut parent = parent.clone();
    let mut built = Vec::new();
    while parent.number < up_to {
        let args = BuildPayloadArgs {
            parent: parent.hash(),
            timestamp: parent.timestamp + timestamp_step,
            fee_recipient: H160::zero(),
            random: H256::zero(),
            withdrawals: Some(Vec::new()),
            beacon_root: Some(H256::zero()),
            slot_number: None,
            version: 1,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        };
        let payload = create_payload(&args, store, Bytes::new()).expect("create payload");
        let block = blockchain
            .build_payload(payload)
            .expect("build payload")
            .payload;
        blockchain.add_block(block.clone()).expect("execute block");
        parent = block.header.clone();
        built.push(block.header);
    }
    built
}

/// Paths of every internal (non-leaf) node in `trie`. These are the keys the merkle write
/// path batches through `TrieDB::multi_get`.
fn node_paths(trie: Trie) -> Vec<Nibbles> {
    trie.into_iter()
        .filter_map(|(path, node)| (!matches!(node, Node::Leaf(_))).then_some(path))
        .collect()
}

fn as_canonical(headers: &[BlockHeader]) -> Vec<(BlockNumber, H256)> {
    headers.iter().map(|h| (h.number, h.hash())).collect()
}

/// Common setup for both tests: build chain A to `CHAIN_LEN`, canonicalize it with a single
/// forkchoice update, and wait for the flush. On return, block `PIVOT`'s state is on neither
/// disk (block 2's commit overwrote it) nor the layer cache (it was pruned by the commit),
/// so it is reachable only by reversing the journal.
async fn chain_a_with_evicted_pivot(
    store: &Store,
    blockchain: &Blockchain,
) -> (Vec<BlockHeader>, BlockHeader) {
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let chain_a = extend_fork(store, blockchain, &genesis_header, CHAIN_LEN, 12).await;
    let head = chain_a.last().expect("chain A non-empty").clone();

    let mut canonical = as_canonical(&chain_a);
    canonical.pop();
    store
        .forkchoice_update(canonical, head.number, head.hash(), None, None)
        .await
        .expect("canonicalize chain A");
    store
        .wait_for_persistence_idle()
        .await
        .expect("flush chain A");

    let pivot = &chain_a[(PIVOT - 1) as usize];
    assert!(
        !store.has_state_root(pivot.state_root).unwrap(),
        "precondition: the pivot's state must be evicted from disk and cache, \
         otherwise neither test exercises the journal"
    );
    (chain_a, head)
}

/// The journal entry written by the deep-reorg reconciliation commit must reverse disk back
/// to the *pivot's* state.
///
/// The reconciliation commit advances disk from the old chain's edge `D` straight to the new
/// chain's `T = pivot + 1` in one batch, folding the overlay in. Its journal entry is the
/// only way back to the pivot afterwards, so its pre-images have to be the pivot's values —
/// not the values disk happened to hold at `D` while the batch was being built. Recording
/// `D`'s values makes the entry reverse to the *old* chain's state instead, and every later
/// deep reorg through that pivot dies with `state root missing for block <pivot>`.
#[tokio::test]
async fn reconciliation_journal_entry_reverses_to_pivot_state() {
    let (store, _dir) = setup_store().await;
    let blockchain = Blockchain::default_with_store(store.clone());

    // Chain B is built before chain A so that chain A's commit prunes B's layers: the
    // replay below must read the pivot through the overlay, not through a stale layer.
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    let block1 = extend_fork(&store, &blockchain, &genesis_header, PIVOT, 12).await;
    let pivot_header = block1.last().expect("block 1 built").clone();
    let chain_b = extend_fork(&store, &blockchain, &pivot_header, CHAIN_LEN, 13).await;
    let head_b = chain_b.last().expect("chain B non-empty").clone();

    let (_chain_a, head_a) = chain_a_with_evicted_pivot(&store, &blockchain).await;
    assert_ne!(
        head_a.hash(),
        head_b.hash(),
        "the two forks must differ or there is no reorg to apply"
    );

    // Deep reorg onto chain B: pivot = block 1, T = 2, and the first commit on the new
    // chain is the reconciliation that folds the overlay into disk.
    apply_fork_choice_with_deep_reorg(&blockchain, head_b.hash(), H256::zero(), H256::zero())
        .await
        .expect("deep reorg onto chain B");
    store
        .wait_for_persistence_idle()
        .await
        .expect("flush chain B");

    // Rebuilding the overlay from the reconciliation entry alone must expose the pivot's
    // state again. This is what the next deep reorg through this pivot does, and it is the
    // step that failed before the fix.
    let t = PIVOT + 1;
    let t_hash = chain_b[0].hash();
    assert_eq!(chain_b[0].number, t, "chain B must start at T");
    store
        .install_overlay_for_reorg(t, t, |n| (n == t).then_some(t_hash))
        .expect("rebuild overlay from the reconciliation journal entry");
    assert!(
        store.has_state_root(pivot_header.state_root).unwrap(),
        "the reconciliation journal entry must reverse disk to the pivot's state; \
         pre-images taken at the old chain's edge reverse to the old chain instead"
    );
}

/// A forkchoice update onto an already-canonical head whose state was evicted must make that
/// head's state readable again.
///
/// There is no side chain to replay in this case, so the overlay install *is* the whole fix:
/// it has to expose head's own post-state. Building it one block lower leaves head's state
/// unreachable, every payload built on head is stashed as ACCEPTED, and the node bounces
/// through a second deep reorg on the next forkchoice update.
#[tokio::test]
async fn canonical_head_with_evicted_state_becomes_readable() {
    let (store, _dir) = setup_store().await;
    let blockchain = Blockchain::default_with_store(store.clone());

    let (chain_a, _head_a) = chain_a_with_evicted_pivot(&store, &blockchain).await;
    let pivot_header = chain_a[(PIVOT - 1) as usize].clone();

    apply_fork_choice_with_deep_reorg(&blockchain, pivot_header.hash(), H256::zero(), H256::zero())
        .await
        .expect("forkchoice update onto the canonical, state-evicted head");

    assert!(
        store.has_state_root(pivot_header.state_root).unwrap(),
        "the deep-reorg apply must expose the canonical head's own post-state, \
         not its parent's"
    );
}

/// Batched trie reads must walk the same cascade as single reads: layer cache ->
/// overlay -> disk.
///
/// While an overlay serves the pivot, disk still holds the old chain's edge `D`, so a
/// batched read that skips the overlay stage answers with nodes from the chain that was
/// reorged away, and reports keys the overlay knows were absent at the pivot as present.
/// `Trie::prefetch_sorted` installs whatever it reads into the trie arena under the hash
/// the reference already carried, so those nodes are merkleized without complaint and the
/// block's state root comes out wrong while gas, receipts and the block access list all
/// match.
#[tokio::test]
async fn batched_trie_reads_resolve_through_the_overlay() {
    let (store, _dir) = setup_store().await;
    let blockchain = Blockchain::default_with_store(store.clone());

    let (chain_a, _head_a) = chain_a_with_evicted_pivot(&store, &blockchain).await;
    let pivot_header = chain_a[(PIVOT - 1) as usize].clone();
    // Disk edge `D`: the last block the chain-A forkchoice update canonicalized, and the
    // state the deep reorg below leaves on disk untouched.
    let disk_header = chain_a[(CHAIN_LEN - 2) as usize].clone();
    assert!(
        store.has_state_root(disk_header.state_root).unwrap(),
        "precondition: the disk edge's state must be on disk"
    );

    apply_fork_choice_with_deep_reorg(&blockchain, pivot_header.hash(), H256::zero(), H256::zero())
        .await
        .expect("forkchoice update onto the canonical, state-evicted head");

    // Probe both sides of the divergence: nodes the pivot references (the overlay holds
    // them, disk has since moved on) and nodes only `D` has (the overlay reports them
    // absent, disk still answers with bytes).
    let mut paths = node_paths(store.open_state_trie(pivot_header.state_root).unwrap());
    paths.extend(node_paths(
        store
            .open_direct_state_trie(disk_header.state_root)
            .unwrap(),
    ));
    assert!(
        paths.len() > 1,
        "no internal trie nodes harvested; the probe would be vacuous"
    );

    let layered = store.open_state_trie(pivot_header.state_root).unwrap();
    let db = layered.db();
    let on_disk = store
        .open_direct_state_trie(disk_header.state_root)
        .unwrap();
    let disk_db = on_disk.db();

    let batched = db.multi_get(&paths);
    assert_eq!(batched.len(), paths.len());
    let mut resolved_above_disk = 0usize;
    for (path, batched) in paths.iter().zip(batched) {
        let batched = batched.unwrap_or_else(|e| panic!("batched read failed at {path:?}: {e}"));
        let single = db
            .get(path.clone())
            .unwrap_or_else(|e| panic!("single read failed at {path:?}: {e}"));
        assert_eq!(
            batched, single,
            "batched read at {path:?} diverged from a single read: the cascade was not \
             walked the same way"
        );
        if single != disk_db.get(path.clone()).unwrap() {
            resolved_above_disk += 1;
        }
    }
    assert!(
        resolved_above_disk > 0,
        "no probed key was answered above disk, so this test cannot observe a read that \
         bypasses the overlay"
    );
}
