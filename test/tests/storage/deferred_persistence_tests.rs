use ethrex_common::{
    H256,
    types::{Block, BlockBody, BlockHeader, BlockNumber},
};
use ethrex_storage::{EngineType, Store, UpdateBatch};

#[tokio::test]
async fn flushed_upto_defaults_to_zero() {
    let store = Store::new("", EngineType::InMemory).expect("store");
    assert_eq!(store.read_flushed_upto().expect("read"), 0);
}

#[tokio::test]
async fn manual_flush_writes_buffered_block_and_advances_marker() {
    let store = Store::new("", EngineType::InMemory).expect("store");
    let header = BlockHeader {
        number: 5,
        ..Default::default()
    };
    let block = Block::new(header, BlockBody::default());
    let hash = block.hash();

    store.buffer_block_for_test(&block); // test-only helper
    // With the read overlay the header is already visible from the buffer before flush.
    assert_eq!(
        store
            .get_block_header_by_hash(hash)
            .expect("hdr")
            .map(|h| h.number),
        Some(5)
    );

    store.flush_block_data_for_test().expect("flush"); // test-only helper
    assert_eq!(store.read_flushed_upto().expect("marker"), 5);
    // header is still visible after flush (now served from disk)
    assert_eq!(
        store
            .get_block_header_by_hash(hash)
            .expect("disk hdr")
            .map(|h| h.number),
        Some(5)
    );
}

/// buffer_block_for_test inserts directly; verify the test accessor sees it.
#[tokio::test]
async fn buffered_header_is_readable_before_flush() {
    let store = Store::new("", EngineType::InMemory).expect("store");
    let header = BlockHeader {
        number: 9,
        ..Default::default()
    };
    let block = Block::new(header, BlockBody::default());
    let hash = block.hash();

    store.buffer_block_for_test(&block);
    assert_eq!(
        store
            .get_block_header_by_hash(hash)
            .expect("hdr")
            .map(|h| h.number),
        Some(9)
    );
}

/// Helper: build a minimal UpdateBatch for a single block at `number` whose
/// parent is `parent_hash`. No trie nodes, no receipts, no codes.
fn minimal_batch(number: BlockNumber, parent_hash: H256) -> (Block, UpdateBatch) {
    let header = BlockHeader {
        number,
        parent_hash,
        ..Default::default()
    };
    let block = Block::new(header, BlockBody::default());
    let batch = UpdateBatch {
        account_updates: vec![],
        storage_updates: vec![],
        receipts: vec![(block.hash(), vec![])],
        blocks: vec![block.clone()],
        code_updates: vec![],
        batch_mode: false,
    };
    (block, batch)
}

/// Drive several sequential store_block_updates (batch_mode: false) through the
/// live path and assert every block is readable (buffer or disk) and
/// read_flushed_upto advances monotonically with no gaps.
///
/// Regression guard for a lost-update bug: if apply_updates were to swap the
/// buffer on the newPayload thread concurrently with the worker, one insert
/// would be silently dropped.
#[tokio::test]
async fn sequential_live_updates_no_lost_inserts() {
    // End-state + monotonicity guard for the lost-update risk, not a deterministic
    // race detector: the inserts are sequential and block on the worker ack, so the
    // race cannot be forced here — this asserts the invariant holds under the real
    // live path (every block stays readable, flushed_upto never goes backward).
    let store = Store::new("", EngineType::InMemory).expect("store");

    const N: BlockNumber = 5;
    let mut parent_hash = H256::zero();
    let mut hashes = Vec::new();
    let mut prev_flushed = store.read_flushed_upto().expect("flushed_upto");

    for n in 1..=N {
        let (block, batch) = minimal_batch(n, parent_hash);
        let hash = block.hash();
        parent_hash = hash;
        hashes.push((n, hash));
        store
            .store_block_updates(batch)
            .expect("store_block_updates");

        // 1. Every block inserted so far must be readable (buffer or disk).
        for (bn, bh) in &hashes {
            let hdr = store.get_block_header_by_hash(*bh).expect("db");
            assert_eq!(
                hdr.map(|h| h.number),
                Some(*bn),
                "block {bn} missing from buffer and disk after storing block {n}"
            );
        }

        // 2. flushed_upto must advance monotonically (never go backward).
        let f = store.read_flushed_upto().expect("flushed_upto");
        assert!(
            f >= prev_flushed,
            "flushed_upto regressed at block {n}: {f} < {prev_flushed}"
        );
        prev_flushed = f;
    }

    // flushed_upto must reach N. The live path acks after staging and flushes
    // asynchronously, so the final block's flush may still be in flight the
    // instant the loop ends — poll with a bounded timeout instead of racing it.
    poll_flushed_upto_reaches(&store, N).await;
}

/// Poll `read_flushed_upto` until it reaches `target` (or fail after a bounded
/// timeout). The live persist path acks after staging and flushes asynchronously,
/// so the durable marker for the last block lands shortly after the call returns.
async fn poll_flushed_upto_reaches(store: &Store, target: BlockNumber) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let flushed = store.read_flushed_upto().expect("flushed_upto");
        if flushed >= target {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "flushed_upto={flushed} expected >= {target} within the timeout"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// Live path routed through the single persist worker: several sequential
/// `store_block_updates` (batch_mode: false) must each stage their block and
/// the worker must flush them, advancing the durable `flushed_upto` marker to
/// the full block count. Same invariant as `sequential_live_updates_no_lost_inserts`
/// but a minimal smoke test that the unified persist worker stages + flushes.
///
/// The live path acks after staging and flushes asynchronously, so the final
/// flush of the last block may not have landed the instant the last
/// `store_block_updates` returns. Poll the durable marker with a bounded timeout
/// (the flush is microseconds on InMemory) instead of reading once and racing it.
#[tokio::test]
async fn live_path_single_worker_persists_and_advances_marker() {
    let store = Store::new("", EngineType::InMemory).expect("store");
    let mut parent = H256::zero();
    for n in 1..=5u64 {
        let (block, batch) = minimal_batch(n, parent);
        parent = block.hash();
        store.store_block_updates(batch).expect("store");
    }
    poll_flushed_upto_reaches(&store, 5).await;
}

/// `wait_for_persistence_idle` must block until the persist worker has fully
/// drained, including the *asynchronous* flush the live path acks before. After
/// it returns, the durable `flushed_upto` marker must already be at the last
/// block with NO polling: the ack-based `Ping` proves the worker handled (and
/// flushed) every prior `Block` message because it is FIFO and single-threaded.
///
/// Without the ack (a bare buffered send), this would race the flush and read a
/// stale marker. Reading `flushed_upto` exactly once right after the await is the
/// regression guard.
#[tokio::test]
async fn wait_for_persistence_idle_blocks_until_flush_durable() {
    let store = Store::new("", EngineType::InMemory).expect("store");

    const N: BlockNumber = 5;
    let mut parent = H256::zero();
    for n in 1..=N {
        let (block, batch) = minimal_batch(n, parent);
        parent = block.hash();
        store.store_block_updates(batch).expect("store");
    }

    store
        .wait_for_persistence_idle()
        .await
        .expect("wait_for_persistence_idle");

    // No poll: the Ping ack already proves every block's flush completed.
    assert_eq!(
        store.read_flushed_upto().expect("flushed_upto"),
        N,
        "wait_for_persistence_idle returned before the last block's flush was durable"
    );
}

#[tokio::test]
async fn reads_hit_buffer_then_fall_through_to_disk() {
    let store = Store::new("", EngineType::InMemory).expect("store");
    let header = BlockHeader {
        number: 11,
        ..Default::default()
    };
    let block = Block::new(header, BlockBody::default());
    let hash = block.hash();

    store.buffer_block_for_test(&block);

    // Served from buffer (not on disk):
    assert_eq!(
        store
            .get_block_header_by_hash(hash)
            .expect("h")
            .map(|h| h.number),
        Some(11)
    );
    assert_eq!(store.get_block_number(hash).await.expect("n"), Some(11));
    assert!(
        store
            .get_block_body_by_hash(hash)
            .await
            .expect("b")
            .is_some()
    );

    // After flush + eviction, same answers come from disk:
    store.flush_block_data_for_test().expect("flush");
    assert_eq!(
        store
            .get_block_header_by_hash(hash)
            .expect("h2")
            .map(|h| h.number),
        Some(11)
    );
    assert_eq!(store.get_block_number(hash).await.expect("n2"), Some(11));
    assert!(
        store
            .get_block_body_by_hash(hash)
            .await
            .expect("b2")
            .is_some()
    );
}

/// Crash-recovery, real boot path: a canonical header exists on disk at block 10
/// (the durable head), but `LatestBlockNumber` was advanced to 12 by FCU while the
/// headers/bodies for 11 and 12 were still buffered and never reached disk (they are
/// NOT on disk, so `load_block_header(12)` returns `None`). `flushed_upto` is 10.
///
/// After a "crash" (drop) and reopen, the real node boot entry `add_initial_state`
/// (→ `add_initial_state_inner`) must clamp the head to `flushed_upto` and re-anchor
/// `LatestBlockNumber` to 10. WITHOUT the clamp, `add_initial_state_inner` would call
/// `load_block_header(12)` → `None` → `MissingLatestBlockNumber` and brick the boot,
/// so this test FAILS without the fix.
#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn boot_clamps_head_to_flushed_upto() {
    use ethrex_common::types::Genesis;

    const GENESIS_KURTOSIS: &str = include_str!("../../../fixtures/genesis/kurtosis.json");
    let genesis: Genesis =
        serde_json::from_str(GENESIS_KURTOSIS).expect("deserialize kurtosis.json");
    let genesis_block = genesis.get_block();
    let genesis_hash = genesis_block.hash();

    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().to_str().unwrap();

    {
        let mut store = Store::new(path, EngineType::RocksDB).expect("store");
        store
            .add_initial_state(genesis.clone())
            .await
            .expect("genesis");

        // Block 10 durable: buffer it, then flush via the REAL flush path (advances
        // flushed_upto to 10), then real FCU makes it canonical head.
        let b10 = Block::new(
            BlockHeader {
                number: 10,
                parent_hash: genesis_hash,
                ..Default::default()
            },
            BlockBody::default(),
        );
        let hash10 = b10.hash();
        store.buffer_block_for_test(&b10);
        store.flush_block_data_for_test().expect("flush 10");
        store
            .forkchoice_update(vec![(10, hash10)], 10, hash10, None, None)
            .await
            .expect("fcu 10");
        assert_eq!(
            store.read_flushed_upto().expect("marker"),
            10,
            "block 10 durable"
        );

        // Blocks 11 and 12: buffered, NEVER flushed -> lost on the crash. Real FCU
        // advances LatestBlockNumber to 12 (it reads the head from the buffer, exactly
        // as production FCU does while 11/12 are buffered-and-unflushed).
        let b11 = Block::new(
            BlockHeader {
                number: 11,
                parent_hash: hash10,
                ..Default::default()
            },
            BlockBody::default(),
        );
        let hash11 = b11.hash();
        let b12 = Block::new(
            BlockHeader {
                number: 12,
                parent_hash: hash11,
                ..Default::default()
            },
            BlockBody::default(),
        );
        let hash12 = b12.hash();
        store.buffer_block_for_test(&b11);
        store.buffer_block_for_test(&b12);
        store
            .forkchoice_update(vec![(11, hash11), (12, hash12)], 12, hash12, None, None)
            .await
            .expect("fcu 12");
        // Precondition: head advanced past the durable marker; 11/12 only in the buffer.
        assert_eq!(
            store.read_flushed_upto().expect("marker"),
            10,
            "11/12 unflushed"
        );
    } // drop = "crash": buffered 11/12 are lost

    // Reopen and run the REAL node boot entry. This must NOT brick: the clamp inside
    // add_initial_state_inner rewinds the head to the durable block 10.
    let mut store = Store::new(path, EngineType::RocksDB).expect("reopen");
    store
        .add_initial_state(genesis)
        .await
        .expect("boot must not brick: head clamped to durable block 10");

    assert_eq!(
        store.get_latest_block_number().await.expect("head"),
        10,
        "boot must clamp LatestBlockNumber to the durable head"
    );
}

/// Crash-recovery, legacy DB: a node synced by a binary that predates deferred
/// persistence has `LatestBlockNumber` advanced and all block data on disk, but
/// NO `flushed_upto` marker. Boot must treat the full head as durable (NOT clamp
/// to genesis) and seed the marker so later crashes clamp correctly.
///
/// Without the absent-marker handling, `read_flushed_upto()` reads as 0, the head
/// is clamped to `min(0, latest) = 0`, and `LatestBlockNumber` is rewound to
/// genesis — which would brick every existing node on its first restart onto the
/// deferred-persistence binary. This test FAILS in that case.
#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn boot_on_legacy_db_without_marker_keeps_head() {
    use ethrex_common::types::Genesis;

    const GENESIS_KURTOSIS: &str = include_str!("../../../fixtures/genesis/kurtosis.json");
    let genesis: Genesis =
        serde_json::from_str(GENESIS_KURTOSIS).expect("deserialize kurtosis.json");
    let genesis_block = genesis.get_block();
    let genesis_hash = genesis_block.hash();

    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().to_str().unwrap();

    {
        let mut store = Store::new(path, EngineType::RocksDB).expect("store");
        store
            .add_initial_state(genesis.clone())
            .await
            .expect("genesis");

        // Canonical header at block 10 written synchronously to disk; head = 10.
        let header = BlockHeader {
            number: 10,
            parent_hash: genesis_hash,
            ..Default::default()
        };
        let block = Block::new(header, BlockBody::default());
        let block_hash = block.hash();
        store.add_block(block).await.expect("add block 10");
        store
            .forkchoice_update(vec![(10, block_hash)], 10, block_hash, None, None)
            .await
            .expect("fcu to 10");

        // Legacy DB precondition: no flushed_upto marker was ever written.
        assert_eq!(
            store.read_flushed_upto().expect("marker"),
            0,
            "precondition: legacy DB has no marker"
        );
    } // drop = restart onto the deferred-persistence binary

    let mut store = Store::new(path, EngineType::RocksDB).expect("reopen");
    store
        .add_initial_state(genesis)
        .await
        .expect("boot must not brick on a legacy marker-less DB");

    // Head must be preserved at 10, NOT rewound to genesis.
    assert_eq!(
        store.get_latest_block_number().await.expect("head"),
        10,
        "legacy DB must keep its head, not clamp to genesis"
    );
    // The marker must now be seeded to the durable head for future crashes.
    assert_eq!(
        store.read_flushed_upto().expect("seeded marker"),
        10,
        "first boot must seed flushed_upto to the durable head"
    );
}

/// Regression: the batch (full-sync) path must advance the durable `flushed_upto`
/// marker, exactly like the live path. The pre-Task-3 `apply_updates_synchronous`
/// wrote block data directly but never called `write_flushed_upto`, so after a
/// live → full-sync → restart sequence the marker lagged and the boot clamp
/// silently rewound the head.
///
/// RED (pre-fix): `apply_updates_synchronous` runs for `batch_mode: true` and never
/// touches the marker, so `read_flushed_upto()` stays 0 and this FAILS.
/// GREEN (post-fix): the batch path routes through the single persist worker, whose
/// `flush_block_data` drains all staged blocks in one tx and writes the max block
/// number as the marker, so `read_flushed_upto()` reaches 3.
#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn batch_path_advances_flushed_upto() {
    use ethrex_common::types::Genesis;

    const GENESIS_KURTOSIS: &str = include_str!("../../../fixtures/genesis/kurtosis.json");
    let genesis: Genesis =
        serde_json::from_str(GENESIS_KURTOSIS).expect("deserialize kurtosis.json");
    let genesis_block = genesis.get_block();
    let genesis_hash = genesis_block.hash();

    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().to_str().unwrap();

    let mut store = Store::new(path, EngineType::RocksDB).expect("store");
    store.add_initial_state(genesis).await.expect("genesis");

    // Build a single batch_mode=true UpdateBatch carrying blocks 1..=3 (the
    // full-sync shape: many blocks, one aggregate trie diff, one fsync). The
    // first block's parent is genesis so `batch_state_roots` resolves a parent
    // state root.
    const N: BlockNumber = 3;
    let mut parent_hash = genesis_hash;
    let mut blocks = Vec::new();
    let mut receipts = Vec::new();
    for n in 1..=N {
        let header = BlockHeader {
            number: n,
            parent_hash,
            ..Default::default()
        };
        let block = Block::new(header, BlockBody::default());
        parent_hash = block.hash();
        receipts.push((block.hash(), vec![]));
        blocks.push(block);
    }
    let batch = UpdateBatch {
        account_updates: vec![],
        storage_updates: vec![],
        receipts,
        blocks,
        code_updates: vec![],
        batch_mode: true,
    };

    store
        .store_block_updates(batch)
        .expect("store_block_updates");

    // The batch path blocks on the worker ack until durable, so the marker is
    // already at the last block number when the call returns (no polling needed).
    assert_eq!(
        store.read_flushed_upto().expect("flushed_upto"),
        N,
        "batch path must advance flushed_upto to the last block number"
    );
}

// ── configurable backpressure cap ─────────────────────────────────────────────

/// The `StoreConfig` default must keep the production-tuned capacity of 2.
#[test]
fn store_config_default_persist_channel_capacity_is_two() {
    assert_eq!(
        ethrex_storage::StoreConfig::default().persist_channel_capacity,
        2
    );
}

/// A `Store` built with cap=1 must still persist every block and advance
/// `flushed_upto` to the full block count; a tighter channel must not lose
/// inserts or deadlock the pipeline.
#[tokio::test]
async fn small_cap_pipeline_persists_all_blocks() {
    use ethrex_storage::StoreConfig;

    let store = Store::new_with_config(
        "",
        EngineType::InMemory,
        StoreConfig {
            persist_channel_capacity: 1,
            ..StoreConfig::default()
        },
    )
    .expect("store");

    const N: BlockNumber = 5;
    let mut parent_hash = H256::zero();
    let mut hashes = Vec::new();

    for n in 1..=N {
        let (block, batch) = minimal_batch(n, parent_hash);
        let hash = block.hash();
        parent_hash = hash;
        hashes.push((n, hash));
        store
            .store_block_updates(batch)
            .expect("store_block_updates");
    }

    // Every block must be readable (buffer or disk).
    for (bn, bh) in &hashes {
        let hdr = store.get_block_header_by_hash(*bh).expect("db");
        assert_eq!(
            hdr.map(|h| h.number),
            Some(*bn),
            "block {bn} missing from buffer and disk"
        );
    }

    // flushed_upto must have reached N. As above, the last block's async flush
    // may still be in flight when the loop ends — poll with a bounded timeout.
    poll_flushed_upto_reaches(&store, N).await;
}

/// Regression: `forkchoice_update` must succeed when the head block is in the
/// in-memory buffer but has not yet been flushed to disk.
///
/// Before the fix, `forkchoice_update` called `load_block_header_by_hash` (disk-only),
/// which missed the buffered header and returned `MissingLatestBlockNumber`, making a
/// valid block appear invalid to the consensus client.  After the fix it calls
/// `get_block_header_by_hash`, which checks the buffer first.
///
/// RED reasoning: reverting the fix (switching back to `load_block_header_by_hash`)
/// would cause this test to fail with `Err(MissingLatestBlockNumber)` because
/// `load_block_header_by_hash` goes straight to the InMemory backend, which has no
/// entry for the buffered block.
#[tokio::test]
async fn forkchoice_update_succeeds_with_buffered_head() {
    let store = Store::new("", EngineType::InMemory).expect("store");

    // Build a minimal block at number 1 and insert it into the buffer ONLY —
    // no disk write, no flush.
    let header = BlockHeader {
        number: 1,
        ..Default::default()
    };
    let block = Block::new(header, BlockBody::default());
    let hash = block.hash();

    store.buffer_block_for_test(&block);

    // Confirm the block is in the buffer but NOT on disk.
    assert!(
        store.get_block_header_by_hash(hash).expect("hdr").is_some(),
        "block must be in the buffer before calling forkchoice_update"
    );
    // load_block_header_by_hash (disk-only) must return None to prove
    // the fix is necessary — if this were Some the test would not be a
    // meaningful regression guard.
    // We verify via get_block_header_by_hash that ONLY the buffer path finds it.
    // (Calling the private load_block_header_by_hash is not possible from here,
    //  so instead we assert that flushed_upto is still 0, confirming no flush occurred.)
    assert_eq!(
        store.read_flushed_upto().expect("flushed_upto"),
        0,
        "nothing must be flushed yet — the block lives only in the buffer"
    );

    // This is the call under test: must return Ok(()) not Err(MissingLatestBlockNumber).
    store
        .forkchoice_update(vec![(1, hash)], 1, hash, None, None)
        .await
        .expect("forkchoice_update must succeed for a buffered-but-not-flushed head");
}

/// The EVM code-read path (`StoreVmDatabase::get_account_code` → `Store::get_account_code`)
/// must resolve code that exists only in the in-memory buffer (not yet flushed to disk).
///
/// Without the buffer overlay in `Store::get_account_code` this test would fail
/// with "Code not found" because the store would only query the DB, which has no entry.
#[tokio::test]
async fn evm_reads_buffered_code() {
    use ethrex_blockchain::vm::StoreVmDatabase;
    use ethrex_common::Bytes;
    use ethrex_common::types::{Block, BlockBody, BlockHeader, Code};
    use ethrex_crypto::keccak::keccak_hash;
    use ethrex_vm::VmDatabase;

    let store = Store::new("", EngineType::InMemory).expect("store");

    // Build a Code whose hash matches the real keccak of the bytecode.
    let bytecode: Bytes = Bytes::from_static(&[0x60, 0x00]); // PUSH1 0
    let code_hash = H256(keccak_hash(&bytecode));
    let code = Code::from_bytecode_unchecked(bytecode, code_hash);
    let expected_len = code.len();

    // Buffer a block that introduces this code — no disk write.
    let header = BlockHeader {
        number: 3,
        ..Default::default()
    };
    let block = Block::new(header, BlockBody::default());
    store.buffer_block_with_codes_for_test(&block, vec![(code_hash, code)]);

    // Construct StoreVmDatabase directly (bypassing the state-root check which
    // requires a persisted state, not needed to test the code-read path).
    let vmdb = StoreVmDatabase::new_for_test(store);

    // The VmDatabase implementation must find the code in the buffer.
    let resolved = vmdb
        .get_account_code(code_hash)
        .expect("code must be found in buffer");
    assert_eq!(
        resolved.len(),
        expected_len,
        "resolved code length must match the buffered code"
    );
}

/// Regression: `get_block_header(number)` must consult the buffer the same way
/// `get_block_body(number)` does. For a block that is canonical (its hash is on
/// disk via FCU) but whose header is still buffered and is NOT the cached head,
/// the by-number header read must return the header, not `None`.
///
/// Before the fix `get_block_header` resolved through the disk-only
/// `load_block_header_by_hash` and returned `None` for such a block, while
/// `get_block_body` (buffer-aware) returned `Some` — an asymmetry that surfaced
/// as RPC returning null for a block the node actually holds.
#[tokio::test]
async fn get_block_header_by_number_is_buffer_aware_for_canonical_nonhead_block() {
    let store = Store::new("", EngineType::InMemory).expect("store");

    // Two buffered-but-unflushed blocks: 2 becomes the head, 1 a canonical non-head.
    let b1 = Block::new(
        BlockHeader {
            number: 1,
            ..Default::default()
        },
        BlockBody::default(),
    );
    let h1 = b1.hash();
    let b2 = Block::new(
        BlockHeader {
            number: 2,
            parent_hash: h1,
            ..Default::default()
        },
        BlockBody::default(),
    );
    let h2 = b2.hash();
    store.buffer_block_for_test(&b1);
    store.buffer_block_for_test(&b2);

    // Make both canonical on disk (CANONICAL_BLOCK_HASHES) and set the cached
    // head to block 2.
    store
        .forkchoice_update(vec![(1, h1), (2, h2)], 2, h2, None, None)
        .await
        .expect("fcu");

    // Nothing has been flushed: block 1's header lives only in the buffer.
    assert_eq!(store.read_flushed_upto().expect("flushed_upto"), 0);

    // Block 1 is canonical but not the head: header-by-number and body-by-number
    // must agree that it exists.
    assert_eq!(
        store.get_block_header(1).expect("header").map(|h| h.number),
        Some(1),
        "get_block_header(number) must find a canonical buffered non-head block"
    );
    assert!(
        store.get_block_body(1).await.expect("body").is_some(),
        "get_block_body(number) must find the same block"
    );
}

/// Regression: the ancestor walk (used by the BLOCKHASH opcode on non-canonical
/// branches) must consult the buffer, so a not-yet-flushed ancestor is visible.
/// Before the fix it used the disk-only `load_block_header_by_hash` and would
/// terminate early on a buffered chain, which could wrongly reject a valid block
/// during a reorg.
#[tokio::test]
async fn ancestors_walk_is_buffer_aware() {
    let store = Store::new("", EngineType::InMemory).expect("store");

    let b1 = Block::new(
        BlockHeader {
            number: 1,
            ..Default::default()
        },
        BlockBody::default(),
    );
    let h1 = b1.hash();
    let b2 = Block::new(
        BlockHeader {
            number: 2,
            parent_hash: h1,
            ..Default::default()
        },
        BlockBody::default(),
    );
    let h2 = b2.hash();
    store.buffer_block_for_test(&b1);
    store.buffer_block_for_test(&b2);

    // Nothing flushed: both headers live only in the buffer.
    assert_eq!(store.read_flushed_upto().expect("flushed_upto"), 0);

    let walked: Vec<BlockNumber> = store
        .ancestors(h2)
        .map(|r| r.expect("ancestor").1.number)
        .collect();
    assert_eq!(
        walked,
        vec![2, 1],
        "ancestor walk must traverse buffered (unflushed) headers"
    );
}

/// Read barrier for deferred trie-layer builds: opening a trie at a state root
/// whose diff layer is still being built (registered pending) must BLOCK until
/// the layer is installed, never snapshot a layer-less cache and read stale
/// on-disk state. This is what makes returning from `store` before the build
/// completes safe.
#[test]
fn open_state_trie_blocks_on_pending_root() {
    use std::sync::mpsc;
    use std::time::Duration;

    let store = Store::new("", EngineType::InMemory).expect("store");
    let root = H256::repeat_byte(0xc1);
    store
        .register_pending_root_for_test(root)
        .expect("register");

    let (tx, rx) = mpsc::channel();
    let reader = store.clone();
    let handle = std::thread::spawn(move || {
        // Must block here until the pending root is cleared.
        let _trie = reader.open_state_trie(root).expect("open");
        tx.send(()).unwrap();
    });

    // While the root's layer is in-flight, the open must not return.
    assert!(
        rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "open_state_trie returned while the root's layer was still in-flight (stale-read window)"
    );

    // Clearing the root (worker installed the layer) unblocks the open.
    store.clear_pending_root_for_test(root);
    rx.recv_timeout(Duration::from_secs(5))
        .expect("open_state_trie did not unblock after the layer was installed");
    handle.join().unwrap();
}

#[tokio::test]
async fn get_block_bodies_range_is_buffer_aware() {
    let store = Store::new("", EngineType::InMemory).expect("store");
    // canonical-but-buffered block at number 1
    let b1 = Block::new(
        BlockHeader {
            number: 1,
            ..Default::default()
        },
        BlockBody::default(),
    );
    let h1 = b1.hash();
    store.buffer_block_for_test(&b1);
    store
        .forkchoice_update(vec![(1, h1)], 1, h1, None, None)
        .await
        .expect("fcu");
    assert_eq!(
        store.read_flushed_upto().expect("marker"),
        0,
        "must be unflushed"
    );
    let bodies = store.get_block_bodies(1, 1).await.expect("range");
    assert_eq!(bodies.len(), 1);
    assert!(
        bodies[0].is_some(),
        "buffered canonical body must not read as None"
    );
}

/// Crash-recovery, tip reorg inside the flush window: block `10a` is flushed
/// (durable, `flushed_upto` = 10), then a sibling `10b` at the same height is
/// buffered-but-unflushed and real FCU durably repoints `canonical[10]` to
/// `10b`. After a crash, `canonical[10]` resolves to `10b`'s hash, whose header
/// never reached disk — so `load_block_header(10)` is `None` even though the
/// marker is 10.
///
/// Boot must not brick: `anchor_to_durable_head` walks the head down to block 9
/// (the highest height whose canonical hash resolves on disk), re-anchors
/// `LatestBlockNumber` to 9, and lowers the marker to 9. WITHOUT the walk-down,
/// boot hits `MissingLatestBlockNumber` and bricks on every restart.
#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn boot_walks_past_reorged_unflushed_head() {
    use ethrex_common::types::Genesis;

    const GENESIS_KURTOSIS: &str = include_str!("../../../fixtures/genesis/kurtosis.json");
    let genesis: Genesis =
        serde_json::from_str(GENESIS_KURTOSIS).expect("deserialize kurtosis.json");
    let genesis_block = genesis.get_block();
    let genesis_hash = genesis_block.hash();

    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().to_str().unwrap();

    {
        let mut store = Store::new(path, EngineType::RocksDB).expect("store");
        store
            .add_initial_state(genesis.clone())
            .await
            .expect("genesis");

        // Block 9 durable: the fallback the walk-down must land on.
        let b9 = Block::new(
            BlockHeader {
                number: 9,
                parent_hash: genesis_hash,
                ..Default::default()
            },
            BlockBody::default(),
        );
        let hash9 = b9.hash();
        store.buffer_block_for_test(&b9);
        store.flush_block_data_for_test().expect("flush 9");
        store
            .forkchoice_update(vec![(9, hash9)], 9, hash9, None, None)
            .await
            .expect("fcu 9");

        // Block 10a durable: flushed and made canonical, advancing the marker to 10.
        let b10a = Block::new(
            BlockHeader {
                number: 10,
                parent_hash: hash9,
                ..Default::default()
            },
            BlockBody::default(),
        );
        let hash10a = b10a.hash();
        store.buffer_block_for_test(&b10a);
        store.flush_block_data_for_test().expect("flush 10a");
        store
            .forkchoice_update(vec![(10, hash10a)], 10, hash10a, None, None)
            .await
            .expect("fcu 10a");
        assert_eq!(
            store.read_flushed_upto().expect("marker"),
            10,
            "block 10a durable"
        );

        // Reorg to sibling 10b at the SAME height (differ by timestamp so the hash
        // differs): buffered, NEVER flushed. Real FCU durably repoints
        // canonical[10] -> hash10b while 10b is only in the buffer.
        let b10b = Block::new(
            BlockHeader {
                number: 10,
                parent_hash: hash9,
                timestamp: 1,
                ..Default::default()
            },
            BlockBody::default(),
        );
        let hash10b = b10b.hash();
        assert_ne!(hash10a, hash10b, "siblings must hash differently");
        store.buffer_block_for_test(&b10b);
        store
            .forkchoice_update(vec![(10, hash10b)], 10, hash10b, None, None)
            .await
            .expect("fcu 10b");
        // Precondition: marker still 10, canonical[10] now points at unflushed 10b.
        assert_eq!(
            store.read_flushed_upto().expect("marker"),
            10,
            "10b unflushed; marker stays at the height of flushed 10a"
        );
    } // drop = "crash": buffered 10b is lost, canonical[10] dangles

    // Reopen and run the REAL node boot entry. Must NOT brick: the walk-down
    // rewinds the head past the dangling canonical[10] to the durable block 9.
    let mut store = Store::new(path, EngineType::RocksDB).expect("reopen");
    store
        .add_initial_state(genesis)
        .await
        .expect("boot must not brick: head walks down to durable block 9");

    assert_eq!(
        store.get_latest_block_number().await.expect("head"),
        9,
        "boot must re-anchor LatestBlockNumber to the highest resolvable head"
    );
    assert_eq!(
        store.read_flushed_upto().expect("marker"),
        9,
        "marker must be lowered to the resolved durable head"
    );
}

/// A short key that `commit_trie_layers` classifies as an account trie node:
/// length is neither 65 nor 131 (so not a leaf) and is <= 65 (so account, not
/// storage), routing it to ACCOUNT_TRIE_NODES.
#[cfg(feature = "rocksdb")]
fn account_trie_node_key(tag: u8) -> Vec<u8> {
    vec![tag, 0, 1, 2]
}

/// A graceful `shutdown()` must flush block data that only lives in the buffer,
/// so a block a crash would lose survives a restart.
///
/// Mirror of `boot_clamps_head_to_flushed_upto` (where a drop loses the buffered
/// block): here `shutdown()` is called instead of dropping, and the block must
/// be durable on reopen.
#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn shutdown_flushes_buffered_block_a_crash_would_lose() {
    use ethrex_common::types::Genesis;

    const GENESIS_KURTOSIS: &str = include_str!("../../../fixtures/genesis/kurtosis.json");
    let genesis: Genesis =
        serde_json::from_str(GENESIS_KURTOSIS).expect("deserialize kurtosis.json");
    let genesis_hash = genesis.get_block().hash();

    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().to_str().unwrap();

    {
        let mut store = Store::new(path, EngineType::RocksDB).expect("store");
        store
            .add_initial_state(genesis.clone())
            .await
            .expect("genesis");

        // Block 10 is only buffered, never flushed via the per-block path.
        let b10 = Block::new(
            BlockHeader {
                number: 10,
                parent_hash: genesis_hash,
                ..Default::default()
            },
            BlockBody::default(),
        );
        let hash10 = b10.hash();
        store.buffer_block_for_test(&b10);
        store
            .forkchoice_update(vec![(10, hash10)], 10, hash10, None, None)
            .await
            .expect("fcu 10");
        assert_eq!(
            store.read_flushed_upto().expect("marker"),
            0,
            "precondition: block 10 is buffered, not yet flushed"
        );

        // Graceful shutdown must persist the buffered block (a crash would lose it).
        store.shutdown().await.expect("shutdown flush");
    }

    let mut store = Store::new(path, EngineType::RocksDB).expect("reopen");
    store
        .add_initial_state(genesis)
        .await
        .expect("boot after clean shutdown");

    assert_eq!(
        store.read_flushed_upto().expect("marker"),
        10,
        "shutdown must have flushed the buffered block to disk"
    );
    assert_eq!(
        store.get_latest_block_number().await.expect("head"),
        10,
        "head must stay at the durable block after a clean shutdown"
    );
}

/// The shutdown trie flush must anchor to the confirmed canonical head, not the
/// newest-inserted layer. A sidechain sibling imported after the head (higher
/// layer id) must NOT become the durable state; the canonical chain must.
///
/// On-disk genesis -> A -> B (canonical head). B' is a sibling of B (also a child
/// of A) stored LAST, so it holds the highest layer id. Committing from the head
/// persists A + B and leaves B' unwritten.
#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn shutdown_commits_canonical_head_trie_not_sidechain() {
    use ethrex_common::types::Genesis;
    use ethrex_trie::Nibbles;

    const GENESIS_KURTOSIS: &str = include_str!("../../../fixtures/genesis/kurtosis.json");
    let genesis: Genesis =
        serde_json::from_str(GENESIS_KURTOSIS).expect("deserialize kurtosis.json");
    let genesis_hash = genesis.get_block().hash();

    let root_a = H256::from_low_u64_be(0xA1);
    let root_b = H256::from_low_u64_be(0xB2);
    let root_b_prime = H256::from_low_u64_be(0xC3);

    let key_a = account_trie_node_key(0xA0);
    let key_b = account_trie_node_key(0xB0);
    let key_b_prime = account_trie_node_key(0xC0);

    // Build a single-block UpdateBatch with one account trie-node diff.
    let make_batch = |number: BlockNumber, parent: H256, state_root: H256, key: Vec<u8>| {
        let header = BlockHeader {
            number,
            parent_hash: parent,
            state_root,
            ..Default::default()
        };
        let block = Block::new(header, BlockBody::default());
        let batch = UpdateBatch {
            account_updates: vec![(Nibbles::from_hex(key), vec![0x42])],
            storage_updates: vec![],
            receipts: vec![(block.hash(), vec![])],
            blocks: vec![block.clone()],
            code_updates: vec![],
            batch_mode: false,
        };
        (block, batch)
    };

    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().to_str().unwrap();

    {
        let mut store = Store::new(path, EngineType::RocksDB).expect("store");
        store
            .add_initial_state(genesis.clone())
            .await
            .expect("genesis");

        let (block_a, batch_a) = make_batch(1, genesis_hash, root_a, key_a.clone());
        store.store_block_updates(batch_a).expect("store A");
        let hash_a = block_a.hash();

        let (block_b, batch_b) = make_batch(2, hash_a, root_b, key_b.clone());
        store.store_block_updates(batch_b).expect("store B");
        let hash_b = block_b.hash();

        // Sidechain sibling of B, stored LAST so it holds the highest layer id.
        let (_block_b_prime, batch_b_prime) =
            make_batch(2, hash_a, root_b_prime, key_b_prime.clone());
        store.store_block_updates(batch_b_prime).expect("store B'");

        // Make the A -> B chain canonical (head = B).
        store
            .forkchoice_update(vec![(1, hash_a), (2, hash_b)], 2, hash_b, None, None)
            .await
            .expect("fcu to B");

        store.shutdown().await.expect("shutdown flush");
    }

    let store = Store::new(path, EngineType::RocksDB).expect("reopen");

    // Canonical head chain (A + B) is durable...
    assert_eq!(
        store.get_trie_node_for_test(true, &key_a).expect("read A"),
        Some(vec![0x42]),
        "canonical ancestor A must be committed on shutdown"
    );
    assert_eq!(
        store.get_trie_node_for_test(true, &key_b).expect("read B"),
        Some(vec![0x42]),
        "canonical head B must be committed on shutdown"
    );
    // ...but the later-inserted sidechain layer must not be persisted.
    assert_eq!(
        store
            .get_trie_node_for_test(true, &key_b_prime)
            .expect("read B'"),
        None,
        "non-canonical sidechain layer must not be durably committed"
    );
}
