/// Integration regression test for EIP-8070 builder over-pack fix.
///
/// eth/72 stores blob txs ELIDED: blobs vec is empty, commitments+proofs are
/// populated. The protocol blob cap is on commitment count, not on blob bytes.
///
/// Pre-patch, `fill_transactions` and `apply_blob_transaction` in payload.rs
/// counted the cap using `blobs.len()`, which is always 0 for elided bundles.
/// This caused the builder to include every blob tx regardless of the cap,
/// over-packing blocks (observed: 18-blob blocks on a cap-15 devnet; CL
/// rejected with `ExecutionInvalidBlobsLen { max: 15, actual: 18 }`).
///
/// Post-patch, both checks use `commitments.len()`, which is correct.
use std::{collections::BTreeMap, fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H256, U256,
    types::{
        AccessList, BYTES_PER_CELL, BlobsBundle, DEFAULT_BUILDER_GAS_CEIL, EIP4844Transaction,
        ELASTICITY_MULTIPLIER, GenesisAccount, MempoolTransaction, Transaction,
        kzg_commitment_to_versioned_hash,
    },
};
use ethrex_crypto::kzg::compute_cells;
use ethrex_storage::{EngineType, Store};
use secp256k1::{Message as SecpMessage, SECP256K1, SecretKey};

/// Max blobs per block for Prague in the fixture genesis (blobSchedule.prague.max = 9).
const PRAGUE_MAX_BLOBS: usize = 9;
/// Number of blob txs to inject — must exceed PRAGUE_MAX_BLOBS to trigger the bug.
const BLOB_TX_COUNT: usize = 15;

/// Valid sample blob: every 32-byte field element stays below the BLS modulus
/// (low bytes only).
fn sample_blob() -> [u8; ethrex_common::types::BYTES_PER_BLOB] {
    let mut blob = [0u8; ethrex_common::types::BYTES_PER_BLOB];
    for i in 0..ethrex_common::types::FIELD_ELEMENTS_PER_BLOB {
        blob[i * 32 + 28] = (i & 0xFF) as u8;
        blob[i * 32 + 31] = ((i >> 8) & 0xFF) as u8;
    }
    blob
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Derive the Ethereum address from a secp256k1 public key.
fn address_from_pubkey(pk: &secp256k1::PublicKey) -> Address {
    let bytes = pk.serialize_uncompressed();
    // Drop the 0x04 prefix, hash the remaining 64 bytes, take last 20.
    let hash = ethrex_crypto::keccak::keccak_hash(&bytes[1..]);
    Address::from_slice(&hash[12..])
}

/// Produce the signing payload for an EIP-4844 transaction (type byte + RLP
/// of unsigned fields), matching the `compute_sender` encoding in transaction.rs.
fn eip4844_signing_payload(tx: &EIP4844Transaction) -> Vec<u8> {
    let mut buf = vec![0x03u8]; // EIP-4844 type byte
    ethrex_rlp::structs::Encoder::new(&mut buf)
        .encode_field(&tx.chain_id)
        .encode_field(&tx.nonce)
        .encode_field(&tx.max_priority_fee_per_gas)
        .encode_field(&tx.max_fee_per_gas)
        .encode_field(&tx.gas)
        .encode_field(&tx.to)
        .encode_field(&tx.value)
        .encode_field(&tx.data)
        .encode_field(&tx.access_list)
        .encode_field(&tx.max_fee_per_blob_gas)
        .encode_field(&tx.blob_versioned_hashes)
        .finish();
    buf
}

/// Sign a 32-byte message hash with a secp256k1 secret key.
/// Returns (r, s, y_parity).
fn secp256k1_sign(sk: &SecretKey, msg_hash: &[u8; 32]) -> (U256, U256, bool) {
    let msg = SecpMessage::from_digest(*msg_hash);
    let (recid, compact) = SECP256K1
        .sign_ecdsa_recoverable(&msg, sk)
        .serialize_compact();
    let r = U256::from_big_endian(&compact[..32]);
    let s = U256::from_big_endian(&compact[32..]);
    let y_parity = i32::from(recid) != 0;
    (r, s, y_parity)
}

async fn setup_store(sender: Address) -> (Store, u64) {
    // Use the shared fixture genesis that already includes all Prague system contracts.
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis fixture");
    let reader = BufReader::new(file);
    let mut genesis: ethrex_common::types::Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis");

    let chain_id = genesis.config.chain_id;

    // Pre-fund the test sender.
    genesis.alloc.insert(
        sender,
        GenesisAccount {
            balance: U256::from(10u128.pow(24)), // 1_000_000 ETH
            nonce: 0,
            code: Bytes::new(),
            storage: BTreeMap::new(),
        },
    );

    let mut store = Store::new("test", EngineType::InMemory).expect("Failed to create store");
    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");
    (store, chain_id)
}

/// Regression test: the builder must cap elided blob bundles at the fork max.
///
/// With more blob txs than the Prague cap (9), only <=9 should appear in the
/// built block's blobs_bundle.commitments.
///
/// Pre-patch: `apply_blob_transaction` uses `blobs.len()` for the cap check.
/// For elided bundles blobs.len()==0, so 0 > max is always false, and ALL
/// BLOB_TX_COUNT (15) txs are included -- assertion fails: 15 > 9.
///
/// Post-patch: `commitments.len()` is used, cap fires at 9 -- assertion passes.
#[tokio::test]
async fn builder_caps_elided_blob_bundles_to_fork_max() {
    // Deterministic test keypair (constant seed for reproducibility).
    let sk_bytes = [0x42u8; 32];
    let sk = SecretKey::from_slice(&sk_bytes).expect("valid sk");
    let pk = sk.public_key(SECP256K1);
    let sender = address_from_pubkey(&pk);

    let (store, chain_id) = setup_store(sender).await;
    let blockchain = Blockchain::default_with_store(store.clone());

    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    // Build one real version-1 blob bundle and its 128 cells. All txs reuse the
    // same blob; what matters is that the elided bundle has a real commitment +
    // cell proofs and that the cells are stored, so the builder (Phase B) can
    // reconstruct the full blob and actually include the tx, exercising the cap
    // (Phase A) on genuinely included blobs rather than skipped ones.
    let blob = sample_blob();
    let full_bundle =
        BlobsBundle::create_from_blobs(&vec![blob], Some(1)).expect("create_from_blobs");
    let commitment = full_bundle.commitments[0];
    let versioned_hash = kzg_commitment_to_versioned_hash(&commitment);
    let cells = compute_cells(&blob).expect("compute_cells");

    // Insert BLOB_TX_COUNT elided blob txs (+ their cells) into the mempool.
    for nonce in 0..BLOB_TX_COUNT {
        let mut tx = EIP4844Transaction {
            chain_id,
            nonce: nonce as u64,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 1_000_000_000,
            gas: 21_000,
            to: Address::from_low_u64_be(1),
            value: U256::zero(),
            data: Bytes::default(),
            access_list: AccessList::default(),
            max_fee_per_blob_gas: U256::from(1_000_000_000u64),
            blob_versioned_hashes: vec![versioned_hash],
            signature_y_parity: false,
            signature_r: U256::zero(),
            signature_s: U256::zero(),
            ..Default::default()
        };

        let payload = eip4844_signing_payload(&tx);
        let hash = ethrex_crypto::keccak::keccak_hash(&payload);
        let (r, s, y_parity) = secp256k1_sign(&sk, &hash);
        tx.signature_r = r;
        tx.signature_s = s;
        tx.signature_y_parity = y_parity;

        let transaction = Transaction::EIP4844Transaction(tx);
        let tx_hash = transaction.hash();
        let mempool_tx = MempoolTransaction::new(transaction, sender);

        blockchain
            .mempool
            .add_transaction(tx_hash, sender, mempool_tx)
            .expect("add_transaction");

        // Elided bundle: blobs empty, real commitment + cell proofs.
        let elided_bundle = BlobsBundle {
            blobs: vec![],
            commitments: vec![commitment],
            proofs: full_bundle.proofs.clone(),
            version: 1,
        };
        blockchain
            .mempool
            .add_blobs_bundle(tx_hash, elided_bundle)
            .expect("add_blobs_bundle");

        // Store all 128 cells so the builder can reconstruct the full blob.
        let cell_entries: Vec<(usize, usize, Box<[u8; BYTES_PER_CELL]>)> = cells
            .iter()
            .enumerate()
            .map(|(col, c)| (0usize, col, Box::new(*c)))
            .collect();
        blockchain
            .mempool
            .store_cells(tx_hash, 1, cell_entries)
            .expect("store_cells");
    }

    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: Address::zero(),
        random: H256::zero(),
        withdrawals: Some(vec![]),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 3,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload_block = create_payload(&args, &store, Bytes::new()).expect("create_payload");
    let result = blockchain
        .build_payload(payload_block)
        .expect("build_payload");

    let included_commitments = result.blobs_bundle.commitments.len();

    // Lower bound guards against a false pass where all blob txs were dropped
    // for an unrelated reason (e.g. execution/fee failure), which would also
    // satisfy `<= max` while testing nothing.
    assert!(
        included_commitments > 0 && included_commitments <= PRAGUE_MAX_BLOBS,
        "expected the builder to include between 1 and {} blob commitments, \
         got {}. If 0: blob txs were dropped before the cap (check fees/exec). \
         If > {}: builder over-packed — pre-patch this happens because \
         apply_blob_transaction uses blobs.len() (always 0 for elided bundles) \
         instead of commitments.len().",
        PRAGUE_MAX_BLOBS,
        included_commitments,
        PRAGUE_MAX_BLOBS,
    );
}
