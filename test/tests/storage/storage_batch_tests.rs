//! Correctness parity between the batched storage-slot lookup
//! (`Store::get_storage_values_batch_by_root`) used by the BAL prefetch and the
//! per-slot single-get path (`Store::get_storage_at_root_with_known_storage_root`)
//! the executor reads through.
//!
//! The prefetch warms a cache that execution trusts, so the batched path must
//! return byte-identical key->value mappings (including "missing slot" -> None)
//! as the single-slot path, across both the FKV-swept and FKV-unswept states.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{Genesis, GenesisAccount},
};
use ethrex_storage::{EngineType, Store, hash_address};

/// Number of populated storage slots on the seeded contract.
///
/// Large enough that the FKV batch spans several sharded multi_get chunks
/// (KEYS_PER_SHARD = 256 in `get_storage_values_batch_by_root`), exercising the
/// parallel-shard path rather than only the single-shard fallback.
const POPULATED_SLOTS: u64 = 2048;

fn account_hash(address: &Address) -> H256 {
    H256::from_slice(&hash_address(address))
}

/// Slots to query: all populated slots, interleaved with absent slots so the
/// parity check covers the `None` (never-written) case too.
fn query_slots() -> Vec<H256> {
    let mut slots = Vec::new();
    for i in 0..POPULATED_SLOTS {
        slots.push(H256::from_low_u64_be(i));
        // An absent slot in the high range: never written at genesis.
        slots.push(H256::from_low_u64_be(1_000_000 + i));
    }
    slots
}

/// Build a genesis with a single contract account holding `POPULATED_SLOTS`
/// non-zero storage slots.
fn seed_genesis(contract: Address) -> Genesis {
    const GENESIS_EXECUTION_API: &str =
        include_str!("../../../fixtures/genesis/execution-api.json");
    let mut genesis: Genesis =
        serde_json::from_str(GENESIS_EXECUTION_API).expect("deserialize execution-api genesis");

    let mut storage: BTreeMap<U256, U256> = BTreeMap::new();
    for i in 0..POPULATED_SLOTS {
        // Non-zero value (genesis drops zero-valued slots). Use a value distinct
        // from the slot index to catch any key/value swap.
        storage.insert(U256::from(i), U256::from(i) + U256::from(7));
    }

    genesis.alloc.insert(
        contract,
        GenesisAccount {
            balance: U256::zero(),
            code: Bytes::from_static(&[0x60, 0x00]),
            storage,
            nonce: 1,
        },
    );
    genesis
}

/// Assert the batched helper matches the per-slot single-get path exactly.
fn assert_parity(store: &Store, state_root: H256, contract: Address) {
    let acct = store
        .get_account_state_by_root(state_root, contract)
        .expect("account lookup")
        .expect("contract account present");
    let acct_hash = account_hash(&contract);
    let storage_root = acct.storage_root;

    let slots = query_slots();

    // Reference: per-slot single-get path the executor actually reads through.
    let single: Vec<Option<U256>> = slots
        .iter()
        .map(|slot| {
            store
                .get_storage_at_root_with_known_storage_root(
                    state_root,
                    acct_hash,
                    storage_root,
                    *slot,
                )
                .expect("single-get storage")
        })
        .collect();

    // Batched path under test. Pass slots in a non-sorted order to exercise the
    // internal sort.
    let batch_input: Vec<(H256, H256, H256)> = slots
        .iter()
        .map(|slot| (acct_hash, storage_root, *slot))
        .collect();
    let batched = store
        .get_storage_values_batch_by_root(state_root, &batch_input)
        .expect("batched storage");

    assert_eq!(
        batched.len(),
        single.len(),
        "batched result length must match single-get length"
    );
    for (i, slot) in slots.iter().enumerate() {
        assert_eq!(
            batched[i], single[i],
            "slot {slot:?} (index {i}) value mismatch: batched={:?} single={:?}",
            batched[i], single[i]
        );
    }

    // Sanity: at least one populated slot returned a value and at least one
    // absent slot returned None, so we know both branches were exercised.
    assert!(
        batched.iter().any(|v| matches!(v, Some(v) if !v.is_zero())),
        "expected at least one populated slot value"
    );
    assert!(
        batched.iter().any(|v| v.is_none()),
        "expected at least one absent slot to be None"
    );
}

/// Drive FKV generation to completion and wait until the cursor covers every
/// key (the generator writes `vec![0xff; 131]` on completion).
fn wait_for_full_fkv(store: &Store) {
    store
        .generate_flatkeyvalue()
        .expect("trigger FKV generation");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let last_written = store.last_written().expect("read FKV cursor");
        // Completion marker: a 131-byte all-0xff cursor that is >= any real key.
        if last_written.len() == 131 && last_written.iter().all(|b| *b == 0xff) {
            return;
        }
        if Instant::now() >= deadline {
            panic!("FKV generation did not finish within timeout (cursor={last_written:?})");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

async fn run_parity_test(engine_type: EngineType) {
    let nonce: u64 = H256::random().to_low_u64_be();
    let path = format!("storage-batch-test-db-{nonce}");
    if !matches!(engine_type, EngineType::InMemory) && std::path::Path::new(&path).exists() {
        std::fs::remove_dir_all(&path).expect("clean test db dir");
    }

    let contract = Address::from_low_u64_be(0xC0FFEE);
    let mut store = Store::new(&path, engine_type).expect("create test store");
    let genesis = seed_genesis(contract);
    let state_root = genesis.get_block().header.state_root;
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");

    // State A: FKV not yet generated -> the batched helper takes the per-slot
    // trie-walk fallback for every slot. Must match single-get exactly.
    assert_parity(&store, state_root, contract);

    // State B: FKV fully swept -> the batched helper takes the sorted multi_get
    // fast path for every slot. Must still match single-get exactly.
    wait_for_full_fkv(&store);
    assert_parity(&store, state_root, contract);

    drop(store);
    if !matches!(engine_type, EngineType::InMemory) && std::path::Path::new(&path).exists() {
        std::fs::remove_dir_all(&path).expect("clean test db dir");
    }
}

/// Effectiveness of the BAL-driven trie-node prefetch (`prefetch_trie_nodes`):
/// the speculative prefix probes must actually hit the real stored internal
/// nodes. A key-encoding mismatch (wrong nibble order, stray leaf flag, wrong
/// `apply_prefix` wrapping) would turn every probe into a bloom miss and warm
/// nothing, so we assert it hits a substantial node set for a storage-heavy
/// account plus its state-trie path. The slot keys used here match the genesis
/// seeding (`H256::from_low_u64_be(i)` == `U256::from(i)` big-endian).
async fn run_prefetch_effectiveness_test(engine_type: EngineType) {
    let nonce: u64 = H256::random().to_low_u64_be();
    let path = format!("trie-prefetch-test-db-{nonce}");
    if !matches!(engine_type, EngineType::InMemory) && std::path::Path::new(&path).exists() {
        std::fs::remove_dir_all(&path).expect("clean test db dir");
    }

    let contract = Address::from_low_u64_be(0xC0FFEE);
    let mut store = Store::new(&path, engine_type).expect("create test store");
    let genesis = seed_genesis(contract);
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");

    let slots: Vec<(Address, H256)> = (0..POPULATED_SLOTS)
        .map(|i| (contract, H256::from_low_u64_be(i)))
        .collect();
    let accounts = vec![contract];

    let hits = store
        .prefetch_trie_nodes(&slots, &accounts)
        .expect("prefetch trie nodes");

    // A correct encoding warms the storage trie's branch nodes (hundreds for
    // 2048 random-hashed slots) plus the contract's state-trie path. A broken
    // encoding hits at most the two root nodes (depth 0). 64 cleanly separates
    // the two outcomes.
    assert!(
        hits >= 64,
        "trie-node prefetch hit only {hits} real nodes for {POPULATED_SLOTS} slots; \
         expected hundreds. A near-zero count means the probe key encoding does \
         not match the stored node keys"
    );

    drop(store);
    if !matches!(engine_type, EngineType::InMemory) && std::path::Path::new(&path).exists() {
        std::fs::remove_dir_all(&path).expect("clean test db dir");
    }
}

#[tokio::test]
async fn trie_node_prefetch_effectiveness_in_memory() {
    run_prefetch_effectiveness_test(EngineType::InMemory).await;
}

#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn trie_node_prefetch_effectiveness_rocksdb() {
    run_prefetch_effectiveness_test(EngineType::RocksDB).await;
}

#[tokio::test]
async fn storage_batch_parity_in_memory() {
    run_parity_test(EngineType::InMemory).await;
}

#[cfg(feature = "rocksdb")]
#[tokio::test]
async fn storage_batch_parity_rocksdb() {
    run_parity_test(EngineType::RocksDB).await;
}
