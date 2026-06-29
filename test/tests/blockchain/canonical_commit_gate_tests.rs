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
        GenesisAccount, Transaction, TxKind,
    },
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
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

/// Load the execution-api genesis, fund `sender`, and return an in-memory store + chain id.
async fn setup_store(sender: Address) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let mut genesis: ethrex_common::types::Genesis =
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
