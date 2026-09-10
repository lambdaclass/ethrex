//! Regression test for the blob (type-3) tx behaviour of `testing_buildBlockV1`
//! (`build_payload_with_transactions`).
//!
//! The canonical encoding accepted on the explicit-transaction path carries no
//! blob sidecar, so a blob tx is *accepted*: its blob gas is accounted from the
//! tx's versioned hashes and the resulting `blobsBundle` is left empty. Callers
//! needing the sidecars must produce them out-of-band. This test pins that
//! contract: a built block containing a blob tx returns an empty `blobs_bundle`
//! while still accounting blob gas in the header.
use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    constants::GAS_PER_BLOB,
    types::{
        DEFAULT_BUILDER_GAS_CEIL, EIP4844Transaction, ELASTICITY_MULTIPLIER, Genesis,
        GenesisAccount, Transaction, TxKind,
    },
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

/// Test private key from fixtures/keys/private_keys_tests.txt.
const TEST_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
const TEST_MAX_FEE_PER_GAS: u64 = 10_000_000_000;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Load the execution-api genesis (Cancun+Prague active from genesis), inject
/// `sender` with a large balance, and return the store plus the chain id.
async fn setup_store(sender: Address) -> (Store, u64) {
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

    let mut store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");

    (store, chain_id)
}

#[tokio::test]
async fn explicit_blob_tx_is_accepted_with_empty_bundle() {
    let sk = SecretKey::from_slice(&hex::decode(TEST_PRIVATE_KEY).unwrap()).unwrap();
    let signer: Signer = LocalSigner::new(sk).into();
    let sender = LocalSigner::new(sk).address;

    let (store, chain_id) = setup_store(sender).await;
    let parent = store.get_block_header(0).unwrap().unwrap();

    // A single versioned hash with the EIP-4844 KZG version byte (0x01). The
    // recipient is an EOA, so no BLOBHASH opcode runs and the hash content is
    // never inspected; only the count drives blob-gas accounting.
    let mut versioned_hash = [0u8; 32];
    versioned_hash[0] = 0x01;

    let mut tx = Transaction::EIP4844Transaction(EIP4844Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas: 21_000,
        to: H160::from_low_u64_be(0xabcd),
        value: U256::zero(),
        data: Bytes::new(),
        access_list: Vec::new(),
        max_fee_per_blob_gas: U256::from(1_000_000_000u64),
        blob_versioned_hashes: vec![H256(versioned_hash)],
        ..Default::default()
    });
    tx.sign_inplace(&signer).await.unwrap();
    assert!(matches!(tx.to(), TxKind::Call(_)));

    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 3,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let blockchain = Blockchain::default_with_store(store);

    let result = blockchain
        .build_payload_with_transactions(payload, vec![tx])
        .expect("blob tx must be accepted on the explicit-transaction path");

    // The blob tx is included...
    assert_eq!(
        result.payload.body.transactions.len(),
        1,
        "the blob tx must be included in the built block"
    );
    // ...blob gas is accounted from the single versioned hash...
    assert_eq!(
        result.payload.header.blob_gas_used,
        Some(u64::from(GAS_PER_BLOB)),
        "blob gas must be accounted from the tx's versioned hashes"
    );
    // ...but the returned bundle is empty (no sidecar in the canonical encoding).
    assert!(
        result.blobs_bundle.blobs.is_empty()
            && result.blobs_bundle.commitments.is_empty()
            && result.blobs_bundle.proofs.is_empty(),
        "blobsBundle must be empty: the explicit list carries no sidecar"
    );
}
