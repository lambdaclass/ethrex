//! `Store::get_storage_at_root` on a just-imported, NOT-YET-canonical block's
//! own `state_root` must honour that exact root — never a value the flat-KV
//! shortcut computed against the durable/canonical head — even once flat-KV
//! generation has fully swept past the queried account.
//!
//! `get_storage_at_root` skips the state-trie account lookup (and its
//! `storage_root` field) once
//! `Store::flatkeyvalue_computed_with_last_written` says the FKV generator has
//! passed the account's hash, reading the on-disk flat-KV table instead. That
//! table reflects only the durable head, so a caller judging a block that is
//! executed but not yet canonical (`newPayload`, before the matching
//! `forkchoice_update`) depends on the in-memory trie diff-layer cache
//! (`TrieLayerCache`) being consulted FIRST — for both the state-trie AND
//! storage-trie opens — so the just-imported block's own diff is found before
//! ever falling through to flat-KV. This is exactly the call site EIP-8369
//! Profile 2 replay (`BlockchainProfile2Evaluator`) and
//! `check_recent_root_references_at_root` both read through.

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER, TxKind},
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;
use std::time::{Duration, Instant};

const TEST_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
const TEST_MAX_FEE_PER_GAS: u64 = 10_000_000_000;

/// Runtime bytecode that unconditionally writes `99` to storage slot `0` on
/// every call: `PUSH1 99; PUSH1 0; SSTORE; STOP`.
const SETTER_CODE: [u8; 6] = [0x60, 0x63, 0x60, 0x00, 0x55, 0x00];

/// Drive FKV generation to completion (mirrors
/// `storage_batch_tests::wait_for_full_fkv`).
async fn wait_for_full_fkv(store: &Store) {
    store
        .generate_flatkeyvalue()
        .expect("trigger FKV generation");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !store
        .flatkeyvalue_fully_generated()
        .expect("read FKV completion marker")
    {
        assert!(
            Instant::now() < deadline,
            "FKV generation did not finish within 30s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn noncanonical_block_storage_write_is_visible_after_fkv_sweeps_the_account() {
    let file = std::fs::File::open(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures/genesis/execution-api.json"),
    )
    .expect("open execution-api genesis fixture");
    let mut genesis: ethrex_common::types::Genesis =
        serde_json::from_reader(std::io::BufReader::new(file)).expect("parse genesis fixture");

    let sender_key = SecretKey::from_slice(&hex::decode(TEST_PRIVATE_KEY).unwrap()).unwrap();
    let signer: Signer = LocalSigner::new(sender_key).into();
    let sender = signer.address();

    let contract = Address::from_low_u64_be(0xC0FFEE);
    genesis.alloc.insert(
        contract,
        ethrex_common::types::GenesisAccount {
            balance: U256::zero(),
            code: Bytes::from(SETTER_CODE.to_vec()),
            storage: Default::default(),
            nonce: 0,
        },
    );
    genesis.alloc.insert(
        sender,
        ethrex_common::types::GenesisAccount {
            balance: U256::from(10u64).pow(U256::from(20u64)),
            code: Bytes::new(),
            storage: Default::default(),
            nonce: 0,
        },
    );
    let chain_id = genesis.config.chain_id;

    let mut store =
        Store::new("fkv-noncanonical-store.db", EngineType::InMemory).expect("in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    // Sweep FKV over genesis BEFORE any further block executes. Slot 0 of
    // `contract` is absent (never written at genesis), so `use_fkv` for this
    // account resolves via the empty-storage-trie fast path once the sweep
    // passes its hash.
    wait_for_full_fkv(&store).await;
    let baseline = store
        .get_storage_at_root(genesis_header.state_root, contract, H256::zero())
        .expect("read genesis storage")
        .unwrap_or_default();
    assert!(
        baseline.is_zero(),
        "precondition: slot 0 must be unwritten at genesis"
    );

    let blockchain = Blockchain::default_with_store(store.clone());
    let tx = EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit: 100_000,
        to: TxKind::Call(contract),
        value: U256::zero(),
        data: Bytes::new(),
        ..Default::default()
    };
    let mut tx = ethrex_common::types::Transaction::EIP1559Transaction(tx);
    tx.sign_inplace(&signer).await.unwrap();

    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 5,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        inclusion_list_transactions: None,
    };
    let payload = create_payload(&args, &store, Bytes::new()).expect("create payload");
    let block = blockchain
        .build_payload_with_transactions(payload, vec![tx])
        .expect("build payload")
        .payload;
    assert_eq!(
        block.body.transactions.len(),
        1,
        "the setter call must have been included"
    );

    // Execute WITHOUT canonicalizing: no `forkchoice_update` call. This is the
    // `newPayload`-before-`forkchoice_update` window Phase 5's Profile 2
    // replay and `check_recent_root_references_at_root` read through.
    blockchain
        .add_block(block.clone())
        .expect("execute the block");
    assert!(
        !store.is_canonical_sync(block.hash()).unwrap(),
        "precondition: the block must not be canonical yet"
    );
    assert_ne!(
        block.header.state_root, genesis_header.state_root,
        "the setter call must have changed the state root"
    );

    // The FKV sweep above never observed this write (it ran before the block
    // executed) and stays fully-generated afterwards: FKV generation is not
    // re-triggered by ordinary block execution. If `get_storage_at_root`
    // wrongly took the flat-KV shortcut for this not-yet-canonical root, this
    // read would return the stale genesis value (0) instead of 99.
    assert!(
        store
            .flatkeyvalue_fully_generated()
            .expect("read FKV completion marker"),
        "precondition: FKV must still read as fully generated"
    );
    let after_write = store
        .get_storage_at_root(block.header.state_root, contract, H256::zero())
        .expect("read non-canonical block storage");
    assert_eq!(
        after_write,
        Some(U256::from(99u64)),
        "a not-yet-canonical block's own state_root must be honoured over the FKV shortcut"
    );
}
