//! EIP-8141 §Networking: a frame transaction carrying blobs is submitted in the
//! EIP-7594 network wrapper,
//! `0x06 || rlp([tx_payload_body, wrapper_version, blobs, commitments, cell_proofs])`,
//! exactly as an EIP-4844 transaction is.

use std::{collections::BTreeMap, fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_common::{
    Address, U256,
    types::{
        APPROVE_EXECUTION_AND_PAYMENT, BYTES_PER_BLOB, BlobsBundle, FIELD_ELEMENTS_PER_BLOB,
        FRAME_TX_MAX_VERIFY_STATE_GAS, Frame, FrameMode, FrameTransaction, Genesis, GenesisAccount,
        Transaction, TxType, kzg_commitment_to_versioned_hash,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::structs::Encoder;
use ethrex_rpc::{
    engine::blobs::{BlobAndProofV2, BlobsV2Request},
    rpc::RpcHandler,
    test_utils::default_context_with_storage,
    types::transaction::SendRawTransactionRequest,
};
use ethrex_storage::{EngineType, Store};
use serde_json::json;

const SENDER: Address = Address::repeat_byte(0xAA);
/// `PUSH1 3 PUSH1 0 PUSH1 0 APPROVE`: the sender approves execution and payment.
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA];
const GWEI: u64 = 1_000_000_000;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// `l1-hegota.json` (every fork through Hegota active at genesis) with `SENDER`
/// funded and running `APPROVE_BOTH_CODE`.
async fn setup_store() -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-hegota.json"))
        .expect("open l1-hegota genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-hegota genesis");
    genesis.alloc.insert(
        SENDER,
        GenesisAccount {
            code: Bytes::from_static(APPROVE_BOTH_CODE),
            storage: BTreeMap::new(),
            balance: U256::from(10u64).pow(U256::from(18u64)),
            nonce: 0,
        },
    );
    let chain_id = genesis.config.chain_id;
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    (store, chain_id)
}

/// A blob whose field elements all stay below the BLS modulus.
fn sample_blob() -> [u8; BYTES_PER_BLOB] {
    let mut blob = [0u8; BYTES_PER_BLOB];
    for i in 0..FIELD_ELEMENTS_PER_BLOB {
        blob[i * 32 + 28] = (i & 0xFF) as u8;
        blob[i * 32 + 31] = ((i >> 8) & 0xFF) as u8;
    }
    blob
}

#[test]
fn a_frame_tx_sent_in_its_blob_network_wrapper_is_pooled_and_its_blobs_are_served() {
    // An unoptimized build copies each 128 KiB blob through the stack several
    // times while decoding the wrapper (as it does for an EIP-4844 wrapper), which
    // overflows a test thread's 2 MiB default stack.
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build the test runtime")
                .block_on(send_a_frame_tx_in_its_blob_network_wrapper())
        })
        .expect("spawn the test thread")
        .join()
        .expect("the test thread must not panic");
}

async fn send_a_frame_tx_in_its_blob_network_wrapper() {
    let (store, chain_id) = setup_store().await;
    let ctx = default_context_with_storage(store).await;

    let bundle = BlobsBundle::create_from_blobs(&vec![sample_blob()], Some(1))
        .expect("build the blob sidecar");
    let versioned_hash = kzg_commitment_to_versioned_hash(&bundle.commitments[0]);
    let tx = FrameTransaction {
        chain_id,
        nonce_keys: vec![],
        nonce_seq: 0,
        sender: SENDER,
        frames: vec![Frame {
            mode: FrameMode::Verify as u8,
            flags: APPROVE_EXECUTION_AND_PAYMENT,
            target: Some(SENDER),
            gas_limit: 10_000,
            state_gas_limit: FRAME_TX_MAX_VERIFY_STATE_GAS,
            value: U256::zero(),
            data: Bytes::new(),
        }],
        max_fee: U256::from(10_000_000 * GWEI),
        max_priority_fee_per_gas: U256::from(GWEI),
        blob_versioned_hashes: vec![versioned_hash],
        ..Default::default()
    };

    let mut wrapper_payload = Vec::new();
    Encoder::new(&mut wrapper_payload)
        .encode_field(&tx)
        .encode_field(&1u8)
        .encode_field(&bundle.blobs)
        .encode_field(&bundle.commitments)
        .encode_field(&bundle.proofs)
        .finish();
    let raw = [vec![TxType::Frame as u8], wrapper_payload].concat();

    let returned_hash =
        SendRawTransactionRequest::parse(&Some(vec![json!(format!("0x{}", hex::encode(&raw)))]))
            .expect("the network wrapper must decode")
            .handle(ctx.clone())
            .await
            .expect("the frame tx must enter the pool");
    let tx_hash = Transaction::FrameTransaction(tx).hash(&NativeCrypto);
    assert_eq!(
        returned_hash,
        json!(format!("{tx_hash:#x}")),
        "the tx hash covers the plain payload, not the wrapper"
    );

    let served_blobs = BlobsV2Request::parse(&Some(vec![json!([versioned_hash])]))
        .unwrap()
        .handle(ctx.clone())
        .await
        .unwrap();
    let expected_blobs = serde_json::to_value(vec![Some(BlobAndProofV2 {
        blob: bundle.blobs[0],
        proofs: bundle.proofs.clone(),
    })])
    .unwrap();
    assert_eq!(
        served_blobs, expected_blobs,
        "engine_getBlobsV2 serves the frame tx's blob and cell proofs"
    );

    let served_to_peers = ctx
        .blockchain
        .get_p2p_transaction_by_hash(&tx_hash)
        .expect("the pool serves the frame tx to peers");
    assert_eq!(
        served_to_peers.encode_canonical_to_vec(),
        raw,
        "peers receive the same network wrapper"
    );
}
