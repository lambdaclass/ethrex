//! EIP-7805 (FOCIL) block production: the payload builder sequences the
//! inclusion list the consensus layer supplied before it fills from the mempool.
//!
//! These drive the real builder over a Bogotá-active chain with signed
//! transactions, so they cover the pre-pass in `apply_inclusion_list_transactions`
//! end to end: ordering against mempool competition, and the two shapes of
//! inclusion-list entry the builder drops instead of failing the whole build.

use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, EIP4844Transaction,
        ELASTICITY_MULTIPLIER, GenesisAccount, Transaction, TxKind, VERSIONED_HASH_VERSION_KZG,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

const TEST_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
const TEST_PRIVATE_KEY_2: &str = "94eb3102993b41ec55c241060f47daa0f6372e2e3ad7e91612ae36c364042e44";
const TEST_MAX_FEE_PER_GAS: u64 = 10_000_000_000;
const TEST_GAS_LIMIT: u64 = 100_000;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn key(hex_str: &str) -> SecretKey {
    SecretKey::from_slice(&hex::decode(hex_str).unwrap()).unwrap()
}

/// A Bogotá-active chain with `senders` funded with 100 ETH each.
async fn bogota_chain(senders: &[Address]) -> (Store, Blockchain, BlockHeader, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let mut genesis: ethrex_common::types::Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("Failed to deserialize genesis file");
    let chain_id = genesis.config.chain_id;
    for sender in senders {
        genesis.alloc.insert(
            *sender,
            GenesisAccount {
                balance: U256::from(10).pow(U256::from(20)),
                code: Bytes::new(),
                storage: Default::default(),
                nonce: 0,
            },
        );
    }

    let mut store = Store::new("", EngineType::InMemory).expect("Failed to build DB for testing");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    let mut config = store.get_chain_config();
    config.hegota_time = Some(0);
    store.set_chain_config(&config).await.unwrap();

    let blockchain = Blockchain::default_with_store(store.clone());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
    (store, blockchain, genesis_header, chain_id)
}

async fn transfer_tx(chain_id: u64, nonce: u64, signer: &Signer) -> Transaction {
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

async fn blob_tx(chain_id: u64, nonce: u64, signer: &Signer) -> Transaction {
    let mut versioned_hash = H256::random();
    versioned_hash.0[0] = VERSIONED_HASH_VERSION_KZG;
    let mut tx = Transaction::EIP4844Transaction(EIP4844Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        max_fee_per_blob_gas: U256::from(TEST_MAX_FEE_PER_GAS),
        gas: TEST_GAS_LIMIT,
        to: Address::from_low_u64_be(0xAAAA),
        value: U256::zero(),
        data: Bytes::new(),
        blob_versioned_hashes: vec![versioned_hash],
        ..Default::default()
    });
    tx.sign_inplace(signer).await.unwrap();
    tx
}

fn build_with_il(
    store: &Store,
    blockchain: &Blockchain,
    parent: &BlockHeader,
    inclusion_list: &[Transaction],
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
        inclusion_list_transactions: Some(inclusion_list.to_vec()),
    };
    let template = create_payload(&args, store, Bytes::new()).unwrap();
    blockchain
        .build_payload_with_il(template, inclusion_list)
        .unwrap()
        .payload
}

/// EIP-7805 has the proposer include the transactions from the inclusion lists
/// it collected, and says nothing about ordering them by fee, so an
/// inclusion-list transaction goes in ahead of a mempool transaction competing
/// for the same block.
#[tokio::test]
async fn inclusion_list_transactions_are_sequenced_before_the_mempool() {
    let signer_a: Signer = LocalSigner::new(key(TEST_PRIVATE_KEY)).into();
    let signer_b: Signer = LocalSigner::new(key(TEST_PRIVATE_KEY_2)).into();
    let (store, blockchain, genesis, chain_id) =
        bogota_chain(&[signer_a.address(), signer_b.address()]).await;

    let il_tx = transfer_tx(chain_id, 0, &signer_a).await;
    let mempool_tx = transfer_tx(chain_id, 0, &signer_b).await;
    blockchain
        .add_transaction_to_pool(mempool_tx.clone())
        .await
        .expect("mempool tx should enter the pool");

    let block = build_with_il(&store, &blockchain, &genesis, std::slice::from_ref(&il_tx));

    let included: Vec<H256> = block
        .body
        .transactions
        .iter()
        .map(|tx| tx.hash(&NativeCrypto))
        .collect();
    assert_eq!(
        included,
        vec![il_tx.hash(&NativeCrypto), mempool_tx.hash(&NativeCrypto)],
        "the inclusion-list transaction must lead the block"
    );
}

/// An inclusion-list transaction the builder cannot apply is skipped, not fatal:
/// the rest of the list and the mempool fill still make it into the block. The
/// receiving side's satisfaction check excuses exactly these transactions, so
/// skipping one cannot leave the block unsatisfied.
#[tokio::test]
async fn an_inapplicable_inclusion_list_transaction_does_not_abort_the_build() {
    let signer_a: Signer = LocalSigner::new(key(TEST_PRIVATE_KEY)).into();
    let signer_b: Signer = LocalSigner::new(key(TEST_PRIVATE_KEY_2)).into();
    let (store, blockchain, genesis, chain_id) =
        bogota_chain(&[signer_a.address(), signer_b.address()]).await;

    // Nonce 7 against a sender sitting at nonce 0: never applicable in this block.
    let unusable = transfer_tx(chain_id, 7, &signer_a).await;
    let usable = transfer_tx(chain_id, 0, &signer_b).await;

    let block = build_with_il(
        &store,
        &blockchain,
        &genesis,
        &[unusable.clone(), usable.clone()],
    );

    let included: Vec<H256> = block
        .body
        .transactions
        .iter()
        .map(|tx| tx.hash(&NativeCrypto))
        .collect();
    assert_eq!(included, vec![usable.hash(&NativeCrypto)]);
}

/// Blob transactions are dropped from the inclusion list. Client software MUST
/// NOT put one in an inclusion list it builds (execution-apis `bogota.md`,
/// `engine_getInclusionListV1`), and the engine API hands over the transaction
/// envelope alone, without the sidecar this node would need to publish a block
/// containing it.
#[tokio::test]
async fn a_blob_transaction_in_the_inclusion_list_is_dropped() {
    let signer_a: Signer = LocalSigner::new(key(TEST_PRIVATE_KEY)).into();
    let signer_b: Signer = LocalSigner::new(key(TEST_PRIVATE_KEY_2)).into();
    let (store, blockchain, genesis, chain_id) =
        bogota_chain(&[signer_a.address(), signer_b.address()]).await;

    let blob = blob_tx(chain_id, 0, &signer_a).await;
    let plain = transfer_tx(chain_id, 0, &signer_b).await;

    let block = build_with_il(
        &store,
        &blockchain,
        &genesis,
        &[blob.clone(), plain.clone()],
    );

    let included: Vec<H256> = block
        .body
        .transactions
        .iter()
        .map(|tx| tx.hash(&NativeCrypto))
        .collect();
    assert_eq!(included, vec![plain.hash(&NativeCrypto)]);
}
