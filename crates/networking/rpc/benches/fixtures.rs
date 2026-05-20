//! Deterministic synthetic fixture generators for the engine_transport bench.

use bytes::Bytes;
use ethrex_common::{
    Address, Bloom, H256,
    types::{Block, BlockBody, BlockHeader},
};
use ethrex_rpc::engine_rest::types::cancun::{
    ExecutionPayload as SszCancunPayload, ExecutionPayloadEnvelope as SszCancunEnvelope,
};
use ethrex_rpc::engine_rest::types::common::Bytes20;
use ethrex_rpc::types::payload::ExecutionPayload as JsonExecutionPayload;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;

/// Default workload sizes — used by both the criterion bench and the harness.
// These constants are forward-looking fixtures consumed in later bench tasks.
#[allow(dead_code)]
pub const DEFAULT_TX_COUNT: usize = 150;
#[allow(dead_code)]
pub const DEFAULT_BLOB_HASH_COUNT: usize = 6;
#[allow(dead_code)]
pub const DEFAULT_BLOB_REQUEST_COUNT: usize = 64;
#[allow(dead_code)]
pub const DEFAULT_BODIES_COUNT: u64 = 128;
#[allow(dead_code)]
pub const DEFAULT_SEED: u64 = 0xDEAD_BEEF_CAFE_BABE;

// ── Internal raw fields ──────────────────────────────────────────────────────

/// All the deterministic raw values needed to build both the JSON and SSZ
/// representations of a Cancun payload.  Fields use only public types so the
/// bench target (a separate compilation unit) can access them freely.
struct RawPayloadFields {
    parent_hash: [u8; 32],
    fee_recipient: [u8; 20],
    state_root: [u8; 32],
    receipts_root: [u8; 32],
    prev_randao: [u8; 32],
    block_number: u64,
    gas_limit: u64,
    gas_used: u64,
    timestamp: u64,
    base_fee_per_gas: u64,
    extra_data: Vec<u8>,
    /// Raw encoded tx bytes (not valid RLP — measuring serde, not validation).
    tx_bytes: Vec<Vec<u8>>,
}

fn build_raw_fields(seed: u64, tx_count: usize) -> RawPayloadFields {
    let mut rng = StdRng::seed_from_u64(seed);

    let tx_bytes: Vec<Vec<u8>> = (0..tx_count)
        .map(|_| {
            let mut buf = vec![0u8; 200];
            rng.fill(&mut buf[..]);
            buf[0] = 0x02; // EIP-1559 type byte
            buf
        })
        .collect();

    RawPayloadFields {
        parent_hash: rand_bytes32(&mut rng),
        fee_recipient: rand_bytes20(&mut rng),
        state_root: rand_bytes32(&mut rng),
        receipts_root: rand_bytes32(&mut rng),
        prev_randao: rand_bytes32(&mut rng),
        block_number: 1_000_000,
        gas_limit: 30_000_000,
        gas_used: (21_000 * tx_count as u64).min(30_000_000),
        timestamp: 1_700_000_000,
        base_fee_per_gas: 7,
        extra_data: b"ethrex-bench".to_vec(),
        tx_bytes,
    }
}

/// Build a Block from raw fields. Transactions are empty (the bench doesn't
/// need to RLP-decode them — `from_block` will re-encode them).
fn raw_to_block(f: &RawPayloadFields) -> Block {
    let header = BlockHeader {
        parent_hash: H256::from(f.parent_hash),
        coinbase: Address::from(f.fee_recipient),
        state_root: H256::from(f.state_root),
        receipts_root: H256::from(f.receipts_root),
        logs_bloom: Bloom::from([0u8; 256]),
        prev_randao: H256::from(f.prev_randao),
        number: f.block_number,
        gas_limit: f.gas_limit,
        gas_used: f.gas_used,
        timestamp: f.timestamp,
        extra_data: Bytes::from(f.extra_data.clone()),
        base_fee_per_gas: Some(f.base_fee_per_gas),
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        ..Default::default()
    };
    let body = BlockBody {
        transactions: vec![],
        ommers: vec![],
        withdrawals: Some(vec![]),
    };
    Block::new(header, body)
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Build a deterministic JSON-shape Cancun ExecutionPayload.
///
/// `tx_count`: number of random EIP-1559-shaped transactions (the bytes aren't
/// valid RLP txs — that's fine, we're measuring serde, not block validation).
/// `blob_hash_count`: not embedded in the payload directly; the count is kept
/// for parity with the harness signature and to optionally tweak the workload.
pub fn cancun_payload_json(
    seed: u64,
    tx_count: usize,
    _blob_hash_count: usize,
) -> JsonExecutionPayload {
    let f = build_raw_fields(seed, tx_count);
    // Build via the public constructor to avoid pub(crate) field access.
    let payload = JsonExecutionPayload::from_block(raw_to_block(&f), None);
    // Overwrite the (empty) transactions list with our synthetic encoded bytes.
    // `block_hash` and `transactions` are the only fields from_block can't set
    // correctly for us; we supply them directly via serde round-trip.
    let tx_json: Vec<serde_json::Value> = f
        .tx_bytes
        .iter()
        .map(|b| {
            let hex = format!("0x{}", hex::encode(b));
            serde_json::Value::String(hex)
        })
        .collect();
    let mut v = serde_json::to_value(&payload).expect("serialize payload");
    v["transactions"] = serde_json::Value::Array(tx_json);
    serde_json::from_value(v).expect("deserialize patched payload")
}

/// Build the matching SSZ envelope for the same seed (same field values).
#[allow(dead_code)]
pub fn cancun_payload_ssz(seed: u64, tx_count: usize, blob_hash_count: usize) -> SszCancunEnvelope {
    let _ = blob_hash_count;
    let f = build_raw_fields(seed, tx_count);
    raw_to_ssz_envelope(&f)
}

/// Return both views from a single seed — convenient when the bench needs both
/// for one (encode_json + encode_ssz) pair.
#[allow(dead_code)]
pub fn cancun_payload_pair(
    seed: u64,
    tx_count: usize,
    blob_hash_count: usize,
) -> (JsonExecutionPayload, SszCancunEnvelope) {
    let json = cancun_payload_json(seed, tx_count, blob_hash_count);
    let f = build_raw_fields(seed, tx_count);
    let ssz = raw_to_ssz_envelope(&f);
    (json, ssz)
}

/// Generate a deterministic list of `n` random KZG versioned hashes.
#[allow(dead_code)]
pub fn blob_versioned_hashes(seed: u64, n: usize) -> Vec<H256> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            let mut h = [0u8; 32];
            rng.fill(&mut h);
            h[0] = 0x01;
            H256::from(h)
        })
        .collect()
}

// ── Internal helpers ─────────────────────────────────────────────────────────

#[allow(dead_code)]
fn raw_to_ssz_envelope(f: &RawPayloadFields) -> SszCancunEnvelope {
    use ethrex_rpc::engine_rest::types::common::{
        MAX_BYTES_PER_TRANSACTION, MAX_EXTRA_DATA_BYTES, MAX_TRANSACTIONS_PER_PAYLOAD,
        MAX_WITHDRAWALS_PER_PAYLOAD,
    };
    use libssz_types::SszList;

    let mut base_fee = [0u8; 32];
    base_fee[..8].copy_from_slice(&f.base_fee_per_gas.to_le_bytes());

    let extra_data: SszList<u8, MAX_EXTRA_DATA_BYTES> = f
        .extra_data
        .clone()
        .try_into()
        .expect("extra_data fits MAX_EXTRA_DATA_BYTES");

    let tx_lists: Vec<SszList<u8, MAX_BYTES_PER_TRANSACTION>> = f
        .tx_bytes
        .iter()
        .map(|b| {
            b.clone()
                .try_into()
                .expect("tx fits MAX_BYTES_PER_TRANSACTION")
        })
        .collect();
    let transactions: SszList<
        SszList<u8, MAX_BYTES_PER_TRANSACTION>,
        MAX_TRANSACTIONS_PER_PAYLOAD,
    > = tx_lists
        .try_into()
        .expect("transactions fit MAX_TRANSACTIONS_PER_PAYLOAD");

    let withdrawals: SszList<
        ethrex_rpc::engine_rest::types::shanghai::Withdrawal,
        MAX_WITHDRAWALS_PER_PAYLOAD,
    > = Vec::new().try_into().expect("empty withdrawals fits");

    let logs_bloom = vec![0u8; 256].try_into().expect("logs_bloom is 256 bytes");

    SszCancunEnvelope {
        execution_payload: SszCancunPayload {
            parent_hash: f.parent_hash,
            fee_recipient: Bytes20(f.fee_recipient),
            state_root: f.state_root,
            receipts_root: f.receipts_root,
            logs_bloom,
            prev_randao: f.prev_randao,
            block_number: f.block_number,
            gas_limit: f.gas_limit,
            gas_used: f.gas_used,
            timestamp: f.timestamp,
            extra_data,
            base_fee_per_gas: base_fee,
            block_hash: [0u8; 32],
            transactions,
            withdrawals,
            blob_gas_used: 0,
            excess_blob_gas: 0,
        },
        parent_beacon_block_root: [0u8; 32],
    }
}

fn rand_bytes20(rng: &mut StdRng) -> [u8; 20] {
    let mut b = [0u8; 20];
    rng.fill(&mut b);
    b
}

fn rand_bytes32(rng: &mut StdRng) -> [u8; 32] {
    let mut b = [0u8; 32];
    rng.fill(&mut b);
    b
}

// ── Response-side fixture generators ─────────────────────────────────────────

/// JSON-side bodies response: `Vec<Option<ExecutionPayloadBody>>`.
/// Generates `n` synthetic bodies with `tx_per_body` random EIP-1559 txs each.
#[allow(dead_code)]
pub fn bodies_range_json(
    seed: u64,
    n: usize,
    tx_per_body: usize,
) -> Vec<Option<ethrex_rpc::types::payload::ExecutionPayloadBody>> {
    use ethrex_rpc::types::payload::EncodedTransaction;

    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            let txs: Vec<EncodedTransaction> = (0..tx_per_body)
                .map(|_| {
                    let mut buf = vec![0u8; 200];
                    rng.fill(&mut buf[..]);
                    buf[0] = 0x02;
                    EncodedTransaction(Bytes::from(buf))
                })
                .collect();
            Some(ethrex_rpc::types::payload::ExecutionPayloadBody {
                transactions: txs,
                withdrawals: None,
            })
        })
        .collect()
}

/// SSZ-side bodies response wrapping `Vec<OptBodyShanghai>`.
#[allow(dead_code)]
pub fn bodies_range_ssz(
    seed: u64,
    n: usize,
    tx_per_body: usize,
) -> ethrex_rpc::engine_rest::types::bodies::BodiesByHashResponseShanghai {
    use ethrex_rpc::engine_rest::types::bodies::{
        BodiesByHashResponseShanghai, BodyShanghai, OptBodyShanghai,
    };
    use ethrex_rpc::engine_rest::types::common::{
        MAX_BYTES_PER_TRANSACTION, MAX_TRANSACTIONS_PER_PAYLOAD, MAX_WITHDRAWALS_PER_PAYLOAD,
    };
    use libssz_types::SszList;

    let mut rng = StdRng::seed_from_u64(seed);
    let bodies: Vec<OptBodyShanghai> = (0..n)
        .map(|_| {
            let tx_lists: Vec<SszList<u8, MAX_BYTES_PER_TRANSACTION>> = (0..tx_per_body)
                .map(|_| {
                    let mut buf = vec![0u8; 200];
                    rng.fill(&mut buf[..]);
                    buf[0] = 0x02;
                    buf.try_into().expect("tx fits MAX_BYTES_PER_TRANSACTION")
                })
                .collect();
            let transactions: SszList<
                SszList<u8, MAX_BYTES_PER_TRANSACTION>,
                MAX_TRANSACTIONS_PER_PAYLOAD,
            > = tx_lists
                .try_into()
                .expect("txs fit MAX_TRANSACTIONS_PER_PAYLOAD");
            let withdrawals: SszList<
                ethrex_rpc::engine_rest::types::shanghai::Withdrawal,
                MAX_WITHDRAWALS_PER_PAYLOAD,
            > = Vec::new().try_into().expect("empty withdrawals fits");
            OptBodyShanghai(Some(BodyShanghai {
                transactions,
                withdrawals,
            }))
        })
        .collect();
    BodiesByHashResponseShanghai { bodies }
}

/// JSON-side blobs response: `Vec<Option<BlobAndProofV1>>`.
#[allow(dead_code)]
pub fn blobs_v1_response_json(
    seed: u64,
    n: usize,
) -> Vec<Option<ethrex_rpc::engine::blobs::BlobAndProofV1>> {
    use ethrex_rpc::engine_rest::types::blobs::{BYTES_PER_BLOB, BYTES_PER_PROOF};

    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            let mut blob = [0u8; BYTES_PER_BLOB];
            rng.fill(&mut blob[..]);
            let mut proof = [0u8; BYTES_PER_PROOF];
            rng.fill(&mut proof[..]);
            Some(ethrex_rpc::engine::blobs::BlobAndProofV1 { blob, proof })
        })
        .collect()
}

/// SSZ-side blobs response wrapping `Vec<OptBlobAndProofV1>`.
#[allow(dead_code)]
pub fn blobs_v1_response_ssz(
    seed: u64,
    n: usize,
) -> ethrex_rpc::engine_rest::types::blobs::BlobsResponseV1 {
    use ethrex_rpc::engine_rest::types::blobs::{
        BYTES_PER_BLOB, BYTES_PER_PROOF, BlobAndProofV1 as SszBlobAndProofV1, BlobsResponseV1,
        OptBlobAndProofV1,
    };
    use libssz_types::SszList;

    let mut rng = StdRng::seed_from_u64(seed);
    let items: Vec<OptBlobAndProofV1> = (0..n)
        .map(|_| {
            let mut blob = vec![0u8; BYTES_PER_BLOB];
            rng.fill(&mut blob[..]);
            let mut proof = [0u8; BYTES_PER_PROOF];
            rng.fill(&mut proof[..]);
            let blob_ssz: SszList<u8, BYTES_PER_BLOB> =
                blob.try_into().expect("blob fits BYTES_PER_BLOB");
            OptBlobAndProofV1::some(SszBlobAndProofV1 {
                blob: blob_ssz,
                proof,
            })
        })
        .collect();
    BlobsResponseV1 { items }
}

/// JSON-side getPayload response — same `ExecutionPayload` shape as newPayload.
/// The V3 response embeds the same payload shape; measuring encode/decode is representative.
#[allow(dead_code)]
pub fn getpayload_response_json(seed: u64) -> ethrex_rpc::types::payload::ExecutionPayload {
    cancun_payload_json(seed, DEFAULT_TX_COUNT, DEFAULT_BLOB_HASH_COUNT)
}

/// SSZ-side getPayload response — same `ExecutionPayloadEnvelope` shape as newPayload.
#[allow(dead_code)]
pub fn getpayload_response_ssz(
    seed: u64,
) -> ethrex_rpc::engine_rest::types::cancun::ExecutionPayloadEnvelope {
    cancun_payload_ssz(seed, DEFAULT_TX_COUNT, DEFAULT_BLOB_HASH_COUNT)
}
