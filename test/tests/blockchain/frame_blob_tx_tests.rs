//! EIP-8141 frame transactions that carry blobs: the pool admits them with their
//! sidecar, the builder accounts their blob gas and returns their sidecar in the
//! payload's blobs bundle, and the built block imports.

use std::{collections::BTreeMap, fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    error::MempoolError,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    constants::GAS_PER_BLOB,
    types::{
        APPROVE_EXECUTION_AND_PAYMENT, BYTES_PER_BLOB, BlobsBundle, BlobsBundleError,
        DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, FIELD_ELEMENTS_PER_BLOB,
        FRAME_TX_MAX_VERIFY_STATE_GAS, Frame, FrameMode, FrameTransaction, Genesis, GenesisAccount,
        MempoolTransaction, Transaction, frame_tx_nonce_manager, keyed_nonce_slot,
        kzg_commitment_to_versioned_hash,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{EngineType, Store};

const SENDER: Address = Address::repeat_byte(0xAA);
/// `PUSH1 3 PUSH1 0 PUSH1 0 APPROVE`: the sender approves execution and payment.
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA];
const GWEI: u64 = 1_000_000_000;
/// A non-zero EIP-8250 nonce key.
const NONCE_KEY: u64 = 0xBEEF;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// `l1-hegota.json` (every fork through Hegota active at genesis) with `SENDER`
/// funded and running `APPROVE_BOTH_CODE`.
async fn setup_store() -> (Store, u64) {
    setup_store_with(0, 0).await
}

/// `setup_store` with `SENDER`'s account nonce set to `sender_nonce` and its
/// `NONCE_KEY` sequence set to `key_sequence` in the EIP-8250 nonce manager.
async fn setup_store_with(sender_nonce: u64, key_sequence: u64) -> (Store, u64) {
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
            nonce: sender_nonce,
        },
    );
    if key_sequence > 0 {
        let slot = keyed_nonce_slot(SENDER, U256::from(NONCE_KEY));
        genesis
            .alloc
            .get_mut(&frame_tx_nonce_manager())
            .expect("the nonce manager is in the genesis")
            .storage
            .insert(
                U256::from_big_endian(slot.as_bytes()),
                U256::from(key_sequence),
            );
    }
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

/// A self-verifying frame transaction carrying one blob, and that blob's
/// EIP-7594 (cell-proof) sidecar.
fn frame_tx_with_one_blob(chain_id: u64) -> (FrameTransaction, BlobsBundle) {
    let bundle = BlobsBundle::create_from_blobs(&vec![sample_blob()], Some(1))
        .expect("build the blob sidecar");
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
        // Covers the execution, state and blob gas at any base fees this test sees.
        max_fee: U256::from(10_000_000 * GWEI),
        max_priority_fee_per_gas: U256::from(GWEI),
        blob_versioned_hashes: vec![kzg_commitment_to_versioned_hash(&bundle.commitments[0])],
        ..Default::default()
    };
    (tx, bundle)
}

#[tokio::test]
async fn a_frame_tx_with_blobs_is_built_with_its_blob_gas_and_sidecar_and_the_block_imports() {
    let (store, chain_id) = setup_store().await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let (tx, bundle) = frame_tx_with_one_blob(chain_id);
    let tx_hash = blockchain
        .add_blob_transaction_to_pool(Transaction::FrameTransaction(tx), bundle.clone())
        .await
        .expect("a frame tx with a valid sidecar must enter the pool");

    let parent = store.get_block_header(0).unwrap().unwrap();
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(1),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let result = blockchain.build_payload(payload).unwrap();
    let block = result.payload;

    let included: Vec<H256> = block
        .body
        .transactions
        .iter()
        .map(|tx| tx.hash(&NativeCrypto))
        .collect();
    assert_eq!(
        included,
        vec![tx_hash],
        "the block must include the frame tx"
    );
    assert!(
        result.receipts[0].succeeded,
        "the frame tx must succeed, got: {:?}",
        result.receipts
    );
    assert_eq!(
        block.header.blob_gas_used,
        Some(u64::from(GAS_PER_BLOB)),
        "the frame tx's blob counts toward the block's blob gas"
    );
    assert_eq!(
        result.blobs_bundle, bundle,
        "the payload's blobs bundle carries the frame tx's sidecar"
    );

    let (import_store, _) = setup_store().await;
    Blockchain::default_with_store(import_store)
        .add_block(block)
        .expect("a block carrying a frame tx with blobs must import");
}

#[tokio::test]
async fn the_pool_rejects_a_frame_tx_whose_sidecar_does_not_match_its_versioned_hashes() {
    let (store, chain_id) = setup_store().await;
    let blockchain = Blockchain::default_with_store(store);
    let (mut tx, bundle) = frame_tx_with_one_blob(chain_id);
    tx.blob_versioned_hashes[0].0[31] ^= 1;

    let result = blockchain
        .add_blob_transaction_to_pool(Transaction::FrameTransaction(tx), bundle)
        .await;
    assert!(
        matches!(
            result,
            Err(MempoolError::BlobsBundleError(
                BlobsBundleError::BlobVersionedHashesError
            ))
        ),
        "got {result:?}"
    );
}

/// eth/72 (EIP-8070) defines blob elision for EIP-4844 transactions only: a frame
/// tx always travels with its blobs, so a sidecar without them is rejected
/// rather than stored for cell reconstruction.
#[tokio::test]
async fn the_pool_rejects_a_frame_tx_whose_sidecar_elides_its_blobs() {
    let (store, chain_id) = setup_store().await;
    let blockchain = Blockchain::default_with_store(store);
    let (tx, mut bundle) = frame_tx_with_one_blob(chain_id);
    bundle.blobs.clear();

    let result = blockchain
        .add_blob_transaction_to_pool(Transaction::FrameTransaction(tx), bundle)
        .await;
    assert!(
        matches!(
            result,
            Err(MempoolError::BlobsBundleError(
                BlobsBundleError::BlobBundleEmptyError
            ))
        ),
        "got {result:?}"
    );
}

/// The frame tx of `frame_tx_with_one_blob` on nonce key `NONCE_KEY` at
/// sequence 0.
fn keyed_frame_tx_with_one_blob(chain_id: u64) -> (FrameTransaction, BlobsBundle) {
    let (mut tx, bundle) = frame_tx_with_one_blob(chain_id);
    tx.nonce_keys = vec![U256::from(NONCE_KEY)];
    (tx, bundle)
}

/// EIP-8250 ties a keyed frame tx's sequence to its keys, not to the account
/// nonce: a sender whose account nonce is ahead of an unused key's sequence still
/// has a valid tx on that key, and the stale-blob sweep after a new head keeps it.
#[tokio::test]
async fn the_stale_blob_sweep_keeps_a_keyed_frame_tx_whose_key_is_unused() {
    let (store, chain_id) = setup_store_with(1, 0).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let (tx, bundle) = keyed_frame_tx_with_one_blob(chain_id);
    let tx_hash = blockchain
        .add_blob_transaction_to_pool(Transaction::FrameTransaction(tx), bundle)
        .await
        .expect("a keyed frame tx with blobs must enter the pool");

    let head = store.get_block_header(0).unwrap().unwrap().hash();
    blockchain.remove_stale_blob_txs(head).await.unwrap();
    assert!(
        blockchain.mempool.contains_tx(tx_hash).unwrap(),
        "the account nonce does not make a keyed frame tx stale"
    );
}

/// Once a block has moved the key past the tx's sequence, the tx can never be
/// included, and the stale-blob sweep drops it.
#[tokio::test]
async fn the_stale_blob_sweep_drops_a_keyed_frame_tx_whose_key_moved_past_it() {
    let (store, chain_id) = setup_store_with(0, 1).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let (tx, bundle) = keyed_frame_tx_with_one_blob(chain_id);
    let tx = Transaction::FrameTransaction(tx);
    let tx_hash = tx.hash(&NativeCrypto);
    // Admission rejects a consumed sequence, so place the tx as a pool that
    // admitted it before the key was consumed holds it.
    blockchain
        .mempool
        .add_blobs_bundle(tx_hash, bundle)
        .expect("add the blobs bundle");
    blockchain
        .mempool
        .add_transaction(
            tx_hash,
            SENDER,
            MempoolTransaction::new(tx, SENDER),
            None,
            None,
        )
        .expect("add the transaction");

    let head = store.get_block_header(0).unwrap().unwrap().hash();
    blockchain.remove_stale_blob_txs(head).await.unwrap();
    assert!(
        !blockchain.mempool.contains_tx(tx_hash).unwrap(),
        "a frame tx whose key moved past its sequence is stale"
    );
}
