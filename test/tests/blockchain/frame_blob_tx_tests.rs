//! End-to-end coverage for blob-carrying EIP-8141 frame transactions.
//!
//! Two paths matter and neither is reachable from the unit tests on the wire
//! types alone:
//!
//!   1. **Building.** A frame transaction that carries blobs must contribute its
//!      blob gas to `header.blob_gas_used`, exactly as an EIP-4844 transaction
//!      does. This is the one place where a mistake produces an invalid block
//!      rather than a dropped transaction, so it is pinned here against the real
//!      payload builder.
//!   2. **Admission and serving.** Such a transaction must be admitted together
//!      with its sidecar, and then served back over p2p in the EIP-7594 wrapped
//!      form with that sidecar attached.

use std::collections::BTreeMap;

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    constants::GAS_PER_BLOB,
    types::{
        BlobsBundle, ChainConfig, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Frame,
        FrameMode, FrameTransaction, Genesis, GenesisAccount, P2PTransaction, Transaction,
        blobs_bundle::blob_from_bytes,
    },
};
use ethrex_storage::{EngineType, Store};

/// The frame transaction's sender, seeded with code that calls
/// `APPROVE(APPROVE_EXECUTION_AND_PAYMENT)`. With real APPROVE code on the sender
/// the transaction needs no outer signature at all: the VERIFY frame approves
/// both scopes itself, so these tests stay focused on blob handling.
const SENDER: u64 = 0xABCD;

/// PUSH1 scope; PUSH1 0; PUSH1 0; APPROVE; STOP
fn approve_both_code() -> Bytes {
    Bytes::from(vec![0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA, 0x00])
}

async fn hegota_store(store_name: &str) -> Store {
    let sender = Address::from_low_u64_be(SENDER);
    let genesis = Genesis {
        config: ChainConfig {
            chain_id: 0,
            shanghai_time: Some(0),
            cancun_time: Some(0),
            hegota_time: Some(0),
            ..Default::default()
        },
        gas_limit: 100_000_000,
        alloc: [(
            sender,
            GenesisAccount {
                code: approve_both_code(),
                storage: BTreeMap::new(),
                // Enough to front the transaction's maximum cost, blob fee included.
                balance: U256::from(10u64).pow(U256::from(20u64)),
                nonce: 0,
            },
        )]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let mut store = Store::new(store_name, EngineType::InMemory).expect("storage setup");
    store
        .add_initial_state(genesis)
        .await
        .expect("genesis setup");
    store
}

/// A KZG-valid single-blob sidecar and the versioned hashes that match it.
///
/// Version 1 (EIP-7594 cell proofs): frame transactions exist only from Hegota,
/// which is after Osaka, where a v0 sidecar is invalid.
fn valid_sidecar() -> (BlobsBundle, Vec<H256>) {
    let blobs = vec![blob_from_bytes("frame blobs".as_bytes().into()).unwrap()];
    let bundle = BlobsBundle::create_from_blobs(&blobs, Some(1)).unwrap();
    let versioned_hashes = bundle.generate_versioned_hashes();
    (bundle, versioned_hashes)
}

/// A minimal valid frame transaction: one `self_verify` frame targeting the
/// sender, whose APPROVE code sets both approvals. `blob_versioned_hashes` and
/// `max_fee_per_blob_gas` make it blob-carrying.
fn blob_frame_tx(versioned_hashes: Vec<H256>) -> FrameTransaction {
    let sender = Address::from_low_u64_be(SENDER);
    FrameTransaction {
        chain_id: 0,
        nonce: 0,
        sender,
        frames: vec![Frame {
            mode: FrameMode::Verify as u8,
            flags: 0x03, // APPROVE_EXECUTION_AND_PAYMENT
            target: Some(sender),
            gas_limit: 100_000,
            value: U256::zero(),
            data: Bytes::new(),
        }],
        signatures: vec![],
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 1_000_000_000,
        max_fee_per_blob_gas: U256::from(1_000u64),
        blob_versioned_hashes: versioned_hashes,
        ..Default::default()
    }
}

/// The builder must account a frame transaction's blob gas in the header, or it
/// would produce a block whose `blob_gas_used` disagrees with its contents.
#[tokio::test]
async fn builder_accounts_blob_gas_for_a_frame_transaction() {
    let store = hegota_store("frame-blob-build").await;
    let (_, versioned_hashes) = valid_sidecar();
    let tx = Transaction::FrameTransaction(blob_frame_tx(versioned_hashes));

    let parent = store.get_block_header(0).unwrap().expect("genesis header");
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
        .expect("a blob-carrying frame transaction must be buildable");

    assert_eq!(
        result.payload.body.transactions.len(),
        1,
        "the frame transaction must be included"
    );
    assert_eq!(
        result.payload.header.blob_gas_used,
        Some(u64::from(GAS_PER_BLOB)),
        "blob gas must be accounted from the frame transaction's versioned hashes"
    );
}

/// Admitted with its sidecar, then served back wrapped per EIP-7594.
#[tokio::test]
async fn blob_frame_transaction_is_admitted_and_served_wrapped() {
    let store = hegota_store("frame-blob-pool").await;
    let blockchain = Blockchain::default_with_store(store);
    let (bundle, versioned_hashes) = valid_sidecar();
    let tx = Transaction::FrameTransaction(blob_frame_tx(versioned_hashes.clone()));

    let hash = blockchain
        .add_blob_transaction_to_pool(tx, bundle.clone())
        .await
        .expect("a blob-carrying frame transaction with a valid sidecar must be admitted");

    // Served back in the wrapped form, sidecar intact: this is what a peer
    // receives in a PooledTransactions response.
    let served = blockchain
        .get_p2p_transaction_by_hash(&hash)
        .expect("the admitted transaction must be servable");
    let P2PTransaction::FrameTransactionWithBlobs(wrapped) = served else {
        panic!("a blob-carrying frame transaction must be served wrapped, got {served:?}");
    };
    assert_eq!(wrapped.blobs_bundle, bundle);
    assert_eq!(wrapped.tx.blob_versioned_hashes, versioned_hashes);
    // The sidecar is not part of the transaction's identity.
    assert_eq!(
        Transaction::FrameTransaction(wrapped.tx).hash(&ethrex_crypto::NativeCrypto),
        hash
    );
}

/// A sidecar whose commitments do not match the declared versioned hashes must
/// be refused, the same as for an EIP-4844 transaction.
#[tokio::test]
async fn blob_frame_transaction_with_mismatched_sidecar_is_rejected() {
    let store = hegota_store("frame-blob-mismatch").await;
    let blockchain = Blockchain::default_with_store(store);
    let (bundle, _) = valid_sidecar();
    // Declare a hash the bundle's commitment does not hash to.
    let mut wrong = [0xABu8; 32];
    wrong[0] = 0x01; // valid KZG version byte, so the correspondence check is what fires
    let tx = Transaction::FrameTransaction(blob_frame_tx(vec![H256(wrong)]));

    let result = blockchain.add_blob_transaction_to_pool(tx, bundle).await;
    assert!(
        result.is_err(),
        "a sidecar that does not match the declared versioned hashes must be rejected"
    );
}
