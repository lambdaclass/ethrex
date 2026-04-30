//! End-to-end tests for EIP-7805 (FOCIL):
//!
//! - **6.2.3** locally-built block with IL satisfies validator on import
//! - **6.3.2** externally-built block that omits an IL tx whose sender retained
//!   nonce/balance/gas → `ChainError::IlUnsatisfied`
//! - **IL-first ordering** — IL txs appear at the front of the payload
//!
//! Tests build real blocks, sign real transactions, and run the full
//! `add_block_pipeline_with_il` integration including the satisfaction
//! validator. Helpers are imported from `batch_tests` where possible to keep
//! parity with existing patterns.

use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    error::ChainError,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER,
        GenesisAccount, Transaction, TxKind,
    },
    validation::BlockValidationContext,
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

const TEST_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
/// Second test key used to construct multi-sender ILs without nonce-collision.
const TEST_PRIVATE_KEY_2: &str = "94eb3102993b41ec55c241060f47daa0f6372e2e3ad7e91612ae36c364042e44";
const TEST_MAX_FEE_PER_GAS: u64 = 10_000_000_000;
const TEST_GAS_LIMIT: u64 = 100_000;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn key(hex_str: &str) -> SecretKey {
    SecretKey::from_slice(&hex::decode(hex_str).unwrap()).unwrap()
}

fn sender_from_key(sk: &SecretKey) -> Address {
    LocalSigner::new(*sk).address
}

/// Load the execution-api genesis with `senders` funded. Returns the store
/// and chain id. Senders get 100 ETH each.
async fn setup_store(senders: &[Address]) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let mut genesis: ethrex_common::types::Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");

    let chain_id = genesis.config.chain_id;
    for sender in senders {
        genesis.alloc.insert(
            *sender,
            GenesisAccount {
                balance: U256::from(10).pow(U256::from(20)), // 100 ETH
                code: Bytes::new(),
                storage: Default::default(),
                nonce: 0,
            },
        );
    }
    let mut store = Store::new("focil-store.db", EngineType::InMemory)
        .expect("Failed to build DB for testing");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    (store, chain_id)
}

/// Sign a 0-value EIP-1559 transfer to a deterministic recipient. Used to
/// populate the inclusion list with realistic, validator-friendly txs.
async fn make_transfer_tx(chain_id: u64, nonce: u64, signer: &Signer) -> Transaction {
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit: TEST_GAS_LIMIT,
        to: TxKind::Call(Address::from_low_u64_be(0xAAAA)),
        value: U256::zero(),
        data: Bytes::new(),
        ..Default::default()
    });
    tx.sign_inplace(signer).await.unwrap();
    tx
}

/// Build a block locally that honors the inclusion list (IL-first sequencing
/// per Decision 5).
async fn build_block_with_il(
    store: &Store,
    blockchain: &Blockchain,
    parent: &BlockHeader,
    il: &[Transaction],
) -> Block {
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 5,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        inclusion_list_transactions: Some(il.to_vec()),
    };
    let block = create_payload(&args, store, Bytes::new()).unwrap();
    blockchain.build_payload_with_il(block, il).unwrap().payload
}

/// Build a block locally that ignores the inclusion list (used to construct
/// the "external proposer that omitted an IL tx" scenario for 6.3.2).
async fn build_block_ignoring_il(
    store: &Store,
    blockchain: &Blockchain,
    parent: &BlockHeader,
) -> Block {
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
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
    let block = create_payload(&args, store, Bytes::new()).unwrap();
    blockchain.build_payload(block).unwrap().payload
}

/// 6.2.3 / 6.3.1 — locally-built block with IL: validator passes on import.
#[tokio::test]
async fn locally_built_block_with_il_satisfies_on_import() {
    let sk1 = key(TEST_PRIVATE_KEY);
    let sk2 = key(TEST_PRIVATE_KEY_2);
    let s1 = sender_from_key(&sk1);
    let s2 = sender_from_key(&sk2);
    let signer1: Signer = LocalSigner::new(sk1).into();
    let signer2: Signer = LocalSigner::new(sk2).into();

    let (mut store, chain_id) = setup_store(&[s1, s2]).await;
    let mut config = store.get_chain_config();
    // Activate Hegotá at genesis so the satisfaction check engages.
    config.hegota_time = Some(0);
    store.set_chain_config(&config).await.unwrap();

    let blockchain = Blockchain::default_with_store(store.clone());
    let genesis = store.get_block_header(0).unwrap().unwrap();

    // IL: one tx per sender — both should be includable.
    let il = vec![
        make_transfer_tx(chain_id, 0, &signer1).await,
        make_transfer_tx(chain_id, 0, &signer2).await,
    ];

    let block = build_block_with_il(&store, &blockchain, &genesis, &il).await;
    assert_eq!(
        block.body.transactions.len(),
        2,
        "block must include both IL txs (IL-first sequencing)"
    );
    // IL-first ordering: IL txs come first in the payload.
    assert_eq!(block.body.transactions[0].hash(), il[0].hash());
    assert_eq!(block.body.transactions[1].hash(), il[1].hash());

    // Import via the IL-aware pipeline. Should succeed (no IlUnsatisfied).
    let context = BlockValidationContext::with_inclusion_list(il);
    blockchain
        .add_block_pipeline_with_il(block, None, &context)
        .expect("locally-built block with IL must satisfy on import");
}

/// 6.3.2 — externally-built block that omits an IL tx whose sender retained
/// nonce/balance/gas → import fails with `ChainError::IlUnsatisfied`.
#[tokio::test]
async fn externally_built_block_omitting_il_tx_fails_on_import() {
    let sk1 = key(TEST_PRIVATE_KEY);
    let sk2 = key(TEST_PRIVATE_KEY_2);
    let s1 = sender_from_key(&sk1);
    let s2 = sender_from_key(&sk2);
    let signer1: Signer = LocalSigner::new(sk1).into();
    let signer2: Signer = LocalSigner::new(sk2).into();

    let (mut store, chain_id) = setup_store(&[s1, s2]).await;
    let mut config = store.get_chain_config();
    config.hegota_time = Some(0);
    store.set_chain_config(&config).await.unwrap();

    let blockchain = Blockchain::default_with_store(store.clone());
    let genesis = store.get_block_header(0).unwrap().unwrap();

    let il_tx_dropped = make_transfer_tx(chain_id, 0, &signer1).await;
    let il = vec![il_tx_dropped.clone()];

    // Build a block WITHOUT the IL — the proposer is "external/adversarial":
    // it has the IL but chooses not to include it. The mempool is empty so
    // the resulting block is empty.
    let block = build_block_ignoring_il(&store, &blockchain, &genesis).await;
    assert!(
        block.body.transactions.is_empty(),
        "adversarial block omits the IL tx and has no other txs"
    );

    // Sender s1 has nonce=0, full balance, full gas — the IL tx is valid
    // against post-state. Validator must reject.
    let context = BlockValidationContext::with_inclusion_list(il);
    let err = blockchain
        .add_block_pipeline_with_il(block, None, &context)
        .expect_err("must reject IL-omitting block");

    match err {
        ChainError::IlUnsatisfied { tx_hash } => {
            assert_eq!(tx_hash, il_tx_dropped.hash());
        }
        other => panic!("expected ChainError::IlUnsatisfied, got {other:?}"),
    }

    // s2 is unused in this scenario; suppresses unused-binding lints.
    let _ = signer2;
}

/// IL-first ordering is preserved when the mempool also has txs available.
/// (Mempool ordering would otherwise interleave by tip.)
#[tokio::test]
async fn il_first_ordering_with_mempool_competition() {
    let sk1 = key(TEST_PRIVATE_KEY);
    let sk2 = key(TEST_PRIVATE_KEY_2);
    let s1 = sender_from_key(&sk1);
    let s2 = sender_from_key(&sk2);
    let signer1: Signer = LocalSigner::new(sk1).into();
    let signer2: Signer = LocalSigner::new(sk2).into();

    let (mut store, chain_id) = setup_store(&[s1, s2]).await;
    let mut config = store.get_chain_config();
    config.hegota_time = Some(0);
    store.set_chain_config(&config).await.unwrap();

    let blockchain = Blockchain::default_with_store(store.clone());
    let genesis = store.get_block_header(0).unwrap().unwrap();

    // The IL has s1's tx. The mempool has s2's tx (competing for inclusion).
    let il_tx = make_transfer_tx(chain_id, 0, &signer1).await;
    let mempool_tx = make_transfer_tx(chain_id, 0, &signer2).await;

    blockchain
        .add_transaction_to_pool(mempool_tx.clone())
        .await
        .expect("mempool tx should enter pool");

    let il = vec![il_tx.clone()];
    let block = build_block_with_il(&store, &blockchain, &genesis, &il).await;

    assert_eq!(
        block.body.transactions.len(),
        2,
        "block must include the IL tx and the mempool tx"
    );
    // IL tx is sequenced FIRST regardless of mempool tip ordering.
    assert_eq!(
        block.body.transactions[0].hash(),
        il_tx.hash(),
        "IL transaction must be first per Decision 5"
    );
    assert_eq!(block.body.transactions[1].hash(), mempool_tx.hash());

    // Sanity: importing this block via the IL-aware pipeline succeeds.
    let context = BlockValidationContext::with_inclusion_list(il);
    blockchain
        .add_block_pipeline_with_il(block, None, &context)
        .expect("IL-first locally-built block must satisfy on import");
}
