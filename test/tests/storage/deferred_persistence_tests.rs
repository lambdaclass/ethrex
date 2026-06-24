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
            .block_data_buffer_get_header_for_test(hash)
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
            let hdr = store
                .block_data_buffer_get_header_for_test(*bh)
                .or_else(|| store.get_block_header_by_hash(*bh).expect("db"));
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

    // flushed_upto must reach N (the worker flushes after each insert).
    assert!(
        prev_flushed >= N,
        "flushed_upto={prev_flushed} expected >= {N}"
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

        // Build a canonical header at block 10 (the durable head) and persist it.
        let header = BlockHeader {
            number: 10,
            parent_hash: genesis_hash,
            ..Default::default()
        };
        let block = Block::new(header, BlockBody::default());
        let block_hash = block.hash();
        store.add_block(block).await.expect("add block 10");
        // Make 10 canonical and set LatestBlockNumber = 10.
        store
            .forkchoice_update(vec![(10, block_hash)], 10, block_hash, None, None)
            .await
            .expect("fcu to 10");

        // Simulate the flusher having durably persisted up to block 10.
        store.write_flushed_upto_for_test(10).expect("marker");
        // Simulate FCU having advanced LatestBlockNumber to 12 while the headers and
        // bodies for 11 and 12 were buffered and never reached disk (no header at 12).
        store.set_latest_block_number_for_test(12).expect("latest");
    } // drop = "crash"

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
    // The arithmetic helper agrees with the clamped head.
    assert_eq!(store.effective_durable_head().await.expect("durable"), 10);
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

// ── configurable backpressure cap ─────────────────────────────────────────────

/// The `StoreConfig` default must keep the production-tuned cap of 2.
#[test]
fn store_config_default_backpressure_cap_is_two() {
    assert_eq!(
        ethrex_storage::StoreConfig::default().block_flush_backpressure_cap,
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
            block_flush_backpressure_cap: 1,
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
        let hdr = store
            .block_data_buffer_get_header_for_test(*bh)
            .or_else(|| store.get_block_header_by_hash(*bh).expect("db"));
        assert_eq!(
            hdr.map(|h| h.number),
            Some(*bn),
            "block {bn} missing from buffer and disk"
        );
    }

    // flushed_upto must have reached N.
    let flushed = store.read_flushed_upto().expect("flushed_upto");
    assert!(flushed >= N, "flushed_upto={flushed} expected >= {N}");
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
        store.block_data_buffer_get_header_for_test(hash).is_some(),
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
