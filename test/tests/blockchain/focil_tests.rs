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
        Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, EIP4844Transaction,
        ELASTICITY_MULTIPLIER, GenesisAccount, Transaction, TxKind,
    },
    validation::BlockValidationContext,
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

const TEST_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
/// Second test key used to construct multi-sender ILs without nonce-collision.
const TEST_PRIVATE_KEY_2: &str = "94eb3102993b41ec55c241060f47daa0f6372e2e3ad7e91612ae36c364042e44";
/// Third key, deliberately left UNFUNDED in genesis, to exercise the
/// "IL tx sender cannot afford it" satisfaction path.
const TEST_PRIVATE_KEY_3: &str = "59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
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
    let mut store =
        Store::new("focil-store.db", EngineType::InMemory).expect("Failed to build DB for testing");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    (store, chain_id)
}

/// Sign a 0-value EIP-1559 transfer with a specific gas limit.
async fn make_transfer_tx_gas(
    chain_id: u64,
    nonce: u64,
    gas_limit: u64,
    signer: &Signer,
) -> Transaction {
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit,
        to: TxKind::Call(Address::from_low_u64_be(0xAAAA)),
        value: U256::zero(),
        data: Bytes::new(),
        ..Default::default()
    });
    tx.sign_inplace(signer).await.unwrap();
    tx
}

/// Sign a 0-value EIP-1559 transfer to a deterministic recipient. Used to
/// populate the inclusion list with realistic, validator-friendly txs.
async fn make_transfer_tx(chain_id: u64, nonce: u64, signer: &Signer) -> Transaction {
    make_transfer_tx_gas(chain_id, nonce, TEST_GAS_LIMIT, signer).await
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
    blockchain.build_payload(block, il).unwrap().payload
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
    blockchain.build_payload(block, &[]).unwrap().payload
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
    assert_eq!(
        block.body.transactions[0].hash(&NativeCrypto),
        il[0].hash(&NativeCrypto)
    );
    assert_eq!(
        block.body.transactions[1].hash(&NativeCrypto),
        il[1].hash(&NativeCrypto)
    );

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
            assert_eq!(tx_hash, il_tx_dropped.hash(&NativeCrypto));
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
        block.body.transactions[0].hash(&NativeCrypto),
        il_tx.hash(&NativeCrypto),
        "IL transaction must be first per Decision 5"
    );
    assert_eq!(
        block.body.transactions[1].hash(&NativeCrypto),
        mempool_tx.hash(&NativeCrypto)
    );

    // Sanity: importing this block via the IL-aware pipeline succeeds.
    let context = BlockValidationContext::with_inclusion_list(il);
    blockchain
        .add_block_pipeline_with_il(block, None, &context)
        .expect("IL-first locally-built block must satisfy on import");
}

// ─────────────────────────────────────────────────────────────────────────────
// Ported intent from execution-specs `tests/amsterdam/eip7805_focil/test_focil.py`
// (branch `eips/bogota/eip-7805`). EELS uses gas-headroom arithmetic to make a
// pending IL tx (un)appendable; here we drive the same satisfaction outcomes
// directly through the validator's per-tx appendability checks. An omitted IL
// tx makes the block INVALID only if it is still validly appendable; otherwise
// the block is valid. Mirrors `test_block_status_depends_on_pending_inclusion_list`
// and `test_block_with_pending_blob_il_tx_is_valid`.
// ─────────────────────────────────────────────────────────────────────────────

/// Hegotá-active chain with `senders` funded; returns store, blockchain,
/// genesis header, and chain id.
async fn hegota_chain(senders: &[Address]) -> (Store, Blockchain, BlockHeader, u64) {
    let (mut store, chain_id) = setup_store(senders).await;
    let mut config = store.get_chain_config();
    config.hegota_time = Some(0);
    store.set_chain_config(&config).await.unwrap();
    let blockchain = Blockchain::default_with_store(store.clone());
    let genesis = store.get_block_header(0).unwrap().unwrap();
    (store, blockchain, genesis, chain_id)
}

/// EELS `valid_with_pending_il_txs_that_do_not_fit`: an omitted IL tx whose gas
/// limit exceeds the block's remaining gas is not appendable → block is valid.
#[tokio::test]
async fn omitted_il_tx_exceeding_block_gas_is_valid() {
    let sk1 = key(TEST_PRIVATE_KEY);
    let s1 = sender_from_key(&sk1);
    let signer1: Signer = LocalSigner::new(sk1).into();

    let (store, blockchain, genesis, chain_id) = hegota_chain(&[s1]).await;

    // gas_limit far larger than any block gas limit → never fits the empty
    // block's headroom. Still affordable (1e9 gas * 10 gwei = 10 ETH < 100 ETH).
    let il = vec![make_transfer_tx_gas(chain_id, 0, 1_000_000_000, &signer1).await];

    let block = build_block_ignoring_il(&store, &blockchain, &genesis).await;
    let context = BlockValidationContext::with_inclusion_list(il);
    blockchain
        .add_block_pipeline_with_il(block, None, &context)
        .expect("omitted IL tx that cannot fit the block must be satisfied");
}

/// EELS `valid_with_pending_il_txs_that_are_invalid`: an omitted IL tx with a
/// future nonce is not appendable against post-state → block is valid.
#[tokio::test]
async fn omitted_il_tx_wrong_nonce_is_valid() {
    let sk1 = key(TEST_PRIVATE_KEY);
    let s1 = sender_from_key(&sk1);
    let signer1: Signer = LocalSigner::new(sk1).into();

    let (store, blockchain, genesis, chain_id) = hegota_chain(&[s1]).await;

    // Sender is at nonce 0; an IL tx at nonce 7 can never be appended now.
    let il = vec![make_transfer_tx(chain_id, 7, &signer1).await];

    let block = build_block_ignoring_il(&store, &blockchain, &genesis).await;
    let context = BlockValidationContext::with_inclusion_list(il);
    blockchain
        .add_block_pipeline_with_il(block, None, &context)
        .expect("omitted IL tx with wrong nonce must be satisfied");
}

/// EELS `valid_with_pending_il_txs_that_fit_but_sender_cannot_afford`: an
/// omitted IL tx whose sender has no balance is not appendable → block valid.
#[tokio::test]
async fn omitted_il_tx_unaffordable_is_valid() {
    let sk1 = key(TEST_PRIVATE_KEY);
    let s1 = sender_from_key(&sk1);
    // sk_poor is NOT funded in genesis (balance 0).
    let sk_poor = key(TEST_PRIVATE_KEY_3);
    let signer_poor: Signer = LocalSigner::new(sk_poor).into();

    // Only s1 is funded; the poor sender is intentionally absent.
    let (store, blockchain, genesis, chain_id) = hegota_chain(&[s1]).await;

    // Correct nonce (0) and fits gas, but the sender cannot pay for it.
    let il = vec![make_transfer_tx(chain_id, 0, &signer_poor).await];

    let block = build_block_ignoring_il(&store, &blockchain, &genesis).await;
    let context = BlockValidationContext::with_inclusion_list(il);
    blockchain
        .add_block_pipeline_with_il(block, None, &context)
        .expect("omitted IL tx whose sender cannot afford it must be satisfied");
}

/// EELS `test_block_with_pending_blob_il_tx_is_valid`: blob (EIP-4844) txs are
/// excluded from the EL IL-satisfaction pass, so omitting one keeps the block
/// valid even though the sender is funded and the tx is otherwise appendable.
#[tokio::test]
async fn omitted_blob_il_tx_is_valid() {
    let sk1 = key(TEST_PRIVATE_KEY);
    let s1 = sender_from_key(&sk1);
    let signer1: Signer = LocalSigner::new(sk1).into();

    let (store, blockchain, genesis, chain_id) = hegota_chain(&[s1]).await;

    // A signed blob tx from a funded sender at the correct nonce: without the
    // blob-skip it would be appendable → unsatisfied. The skip keeps it valid.
    let mut blob_tx = Transaction::EIP4844Transaction(EIP4844Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas: TEST_GAS_LIMIT,
        to: Address::from_low_u64_be(0xAAAA),
        value: U256::zero(),
        data: Bytes::new(),
        max_fee_per_blob_gas: U256::from(1u64),
        blob_versioned_hashes: vec![H256::zero()],
        ..Default::default()
    });
    blob_tx.sign_inplace(&signer1).await.unwrap();
    let il = vec![blob_tx];

    let block = build_block_ignoring_il(&store, &blockchain, &genesis).await;
    let context = BlockValidationContext::with_inclusion_list(il);
    blockchain
        .add_block_pipeline_with_il(block, None, &context)
        .expect("omitted blob IL tx must be satisfied (excluded from EL IL check)");
}

/// EELS `unsatisfied_with_mixed_valid_and_invalid_pending_il_txs`: an IL with
/// one un-appendable tx (wrong nonce) and one appendable tx, both omitted, is
/// UNSATISFIED — the appendable one must trigger the rejection.
#[tokio::test]
async fn mixed_valid_and_invalid_omitted_il_is_unsatisfied() {
    let sk1 = key(TEST_PRIVATE_KEY);
    let sk2 = key(TEST_PRIVATE_KEY_2);
    let s1 = sender_from_key(&sk1);
    let s2 = sender_from_key(&sk2);
    let signer1: Signer = LocalSigner::new(sk1).into();
    let signer2: Signer = LocalSigner::new(sk2).into();

    let (store, blockchain, genesis, chain_id) = hegota_chain(&[s1, s2]).await;

    // s2's tx has a future nonce (not appendable); s1's tx is appendable.
    let invalid = make_transfer_tx(chain_id, 9, &signer2).await;
    let appendable = make_transfer_tx(chain_id, 0, &signer1).await;
    let il = vec![invalid, appendable.clone()];

    let block = build_block_ignoring_il(&store, &blockchain, &genesis).await;
    let context = BlockValidationContext::with_inclusion_list(il);
    let err = blockchain
        .add_block_pipeline_with_il(block, None, &context)
        .expect_err("an appendable omitted IL tx must make the block unsatisfied");
    match err {
        ChainError::IlUnsatisfied { tx_hash } => {
            assert_eq!(tx_hash, appendable.hash(&NativeCrypto))
        }
        other => panic!("expected ChainError::IlUnsatisfied, got {other:?}"),
    }
}
