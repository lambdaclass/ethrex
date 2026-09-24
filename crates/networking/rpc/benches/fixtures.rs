//! Deterministic synthetic fixture generators for the engine transport
//! benchmarks. Covers every fork era of the engine API (Paris → Amsterdam).
//!
//! This file is the single canonical copy: the criterion bench
//! (`benches/engine_transport.rs`) includes it as a sibling module, and the
//! end-to-end harness (`tooling/engine_bench`) includes it via `#[path]`.
//! Keep it free of bench-target-specific code.
//!
//! Shape map (JSON-RPC method ↔ REST/SSZ type):
//! - newPayload:  V1 ↔ `paris::Envelope`, V2 ↔ `shanghai::Envelope`,
//!   V3 ↔ `cancun::Envelope`, V4 ↔ `prague::Envelope` (Prague AND Osaka),
//!   V5 ↔ `amsterdam::Envelope` (BAL + slot)
//! - getPayload:  V1 ↔ `BuiltPayloadParis` (JSON is a bare payload; SSZ adds
//!   block_value — a real spec asymmetry), V2 ↔ `BuiltPayloadShanghai`,
//!   V3 ↔ `BuiltPayloadCancun` (1 proof/blob), V4 ↔ `BuiltPayloadPrague`
//!   (+requests), V5 ↔ `BuiltPayloadOsaka` (cell proofs), V6 ↔ `BuiltPayloadAmsterdam`
//! - blobs:       `getBlobsV1` ↔ `/blobs/v1` (1 proof), `getBlobsV2/V3` ↔
//!   `/blobs/v2`,`/blobs/v3` (cell proofs), `/blobs/v4` is REST-only (no JSON method)
//! - bodies:      `…ByRangeV1` ↔ `BodiesResponseParis`/`BodiesResponseShanghai`
//!   (Shanghai shape serves the shanghai→osaka paths), `…ByRangeV2` ↔
//!   `BodiesResponseAmsterdam` (BAL per body)

use bytes::Bytes;
use ethrex_common::{
    Address, Bloom, H256, U256,
    types::{
        BlobsBundle, Block, BlockBody, BlockHeader,
        block_access_list::{
            AccountChanges, BalanceChange, BlockAccessList, SlotChange, StorageChange,
        },
        requests::EncodedRequests,
    },
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::engine_rest::types::amsterdam::{
    ExecutionPayload as SszAmsterdamPayload, ExecutionPayloadEnvelope as SszAmsterdamEnvelope,
};
use ethrex_rpc::engine_rest::types::built_payload::{
    BlobsBundleV1, BlobsBundleV2, BuiltPayloadAmsterdam, BuiltPayloadCancun, BuiltPayloadOsaka,
    BuiltPayloadParis, BuiltPayloadPrague, BuiltPayloadShanghai, ExecutionRequestsList,
};
use ethrex_rpc::engine_rest::types::cancun::{
    ExecutionPayload as SszCancunPayload, ExecutionPayloadEnvelope as SszCancunEnvelope,
};
use ethrex_rpc::engine_rest::types::common::Bytes20;
use ethrex_rpc::engine_rest::types::paris::{
    ExecutionPayload as SszParisPayload, ExecutionPayloadEnvelope as SszParisEnvelope,
};
use ethrex_rpc::engine_rest::types::prague::{
    ExecutionPayload as SszPraguePayload, ExecutionPayloadEnvelope as SszPragueEnvelope,
};
use ethrex_rpc::engine_rest::types::shanghai::{
    ExecutionPayload as SszShanghaiPayload, ExecutionPayloadEnvelope as SszShanghaiEnvelope,
};
use ethrex_rpc::types::payload::{
    ExecutionPayload as JsonExecutionPayload, ExecutionPayloadResponse,
};
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;

/// Default workload sizes — used by both the criterion bench and the harness.
// Not every constant is used by every includer of this file.
#[allow(dead_code)]
pub const DEFAULT_TX_COUNT: usize = 150;
/// Blobs carried by the synthetic getPayload response bundle (per-block max
/// varies by fork/BPO schedule; 6 is a representative count).
#[allow(dead_code)]
pub const DEFAULT_BUNDLE_BLOB_COUNT: usize = 6;
#[allow(dead_code)]
pub const DEFAULT_BLOB_REQUEST_COUNT: usize = 64;
/// Capped by the REST/SSZ `MAX_BODIES_PER_REQUEST` (32); the JSON side uses
/// the same count so the two transports move the same payload.
#[allow(dead_code)]
pub const DEFAULT_BODIES_COUNT: u64 = 32;
/// Accounts in the synthetic block-level BAL (Amsterdam payloads); ~430 B of
/// RLP per account puts the default around 65 KB, a plausible mainnet size.
#[allow(dead_code)]
pub const DEFAULT_BAL_ACCOUNTS: usize = 150;
/// Accounts in the per-body BAL for the Amsterdam bodies fixtures.
#[allow(dead_code)]
pub const DEFAULT_BODY_BAL_ACCOUNTS: usize = 20;
#[allow(dead_code)]
pub const DEFAULT_SEED: u64 = 0xDEAD_BEEF_CAFE_BABE;

/// Synthetic block value used by the getPayload response fixtures (wei).
#[allow(dead_code)]
const BLOCK_VALUE_WEI: u64 = 1_234_567_890_123_456;
/// Synthetic slot number for Amsterdam payloads.
#[allow(dead_code)]
pub const DEFAULT_SLOT_NUMBER: u64 = 4_242;

/// Which era's optional payload fields are present (the JSON struct is shared
/// across forks with `Option` fields; the SSZ structs are per-fork).
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PayloadEra {
    /// No withdrawals, no blob-gas fields.
    Paris,
    /// Withdrawals, no blob-gas fields.
    Shanghai,
    /// Withdrawals + blob-gas fields (Cancun, Prague, Osaka).
    Cancun,
}

// ── Internal raw fields ──────────────────────────────────────────────────────

/// All the deterministic raw values needed to build both the JSON and SSZ
/// representations of a payload.  Fields use only public types so the bench
/// target (a separate compilation unit) can access them freely.
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

/// Build a Block from raw fields with the era's optional fields. Transactions
/// are empty (the bench doesn't need to RLP-decode them — `from_block` will
/// re-encode them). `slot_number` is set for Amsterdam-shape payloads.
fn raw_to_block(f: &RawPayloadFields, era: PayloadEra, slot_number: Option<u64>) -> Block {
    let blob_fields = if era == PayloadEra::Cancun {
        Some(0)
    } else {
        None
    };
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
        blob_gas_used: blob_fields,
        excess_blob_gas: blob_fields,
        slot_number,
        ..Default::default()
    };
    let body = BlockBody {
        transactions: vec![],
        ommers: vec![],
        withdrawals: if era == PayloadEra::Paris {
            None
        } else {
            Some(vec![])
        },
    };
    Block::new(header, body)
}

// ── Synthetic BAL (EIP-7928, Amsterdam) ──────────────────────────────────────

/// Deterministic synthetic block access list: each account gets 3 storage
/// writes, 5 storage reads, and one balance change.
#[allow(dead_code)]
pub fn build_synthetic_bal(seed: u64, n_accounts: usize) -> BlockAccessList {
    let mut rng = StdRng::seed_from_u64(seed ^ 0xBA1);
    let accounts: Vec<AccountChanges> = (0..n_accounts)
        .map(|_| AccountChanges {
            address: Address::from(rand_bytes20(&mut rng)),
            storage_changes: (0..3)
                .map(|i| SlotChange {
                    slot: U256::from(rng.r#gen::<u64>()),
                    slot_changes: vec![StorageChange {
                        block_access_index: i,
                        post_value: U256::from(rng.r#gen::<u64>()),
                    }],
                })
                .collect(),
            storage_reads: (0..5).map(|_| U256::from(rng.r#gen::<u64>())).collect(),
            balance_changes: vec![BalanceChange {
                block_access_index: 0,
                post_balance: U256::from(rng.r#gen::<u64>()),
            }],
            nonce_changes: Vec::new(),
            code_changes: Vec::new(),
        })
        .collect();
    BlockAccessList::from_accounts(accounts)
}

/// The exact RLP bytes the JSON `blockAccessList` hex field carries; the SSZ
/// `block_access_list` byte list holds the same bytes, so both transports move
/// identical content.
#[allow(dead_code)]
fn bal_rlp(bal: &BlockAccessList) -> Vec<u8> {
    let mut buf = Vec::new();
    bal.encode(&mut buf);
    buf
}

// ── Execution requests (EIP-7685) ────────────────────────────────────────────

/// Two type-prefixed requests: one deposit-shaped (0x00 + 2×192 B) and one
/// withdrawal-shaped (0x01 + 76 B).
#[allow(dead_code)]
fn build_raw_requests(seed: u64) -> Vec<Vec<u8>> {
    let mut rng = StdRng::seed_from_u64(seed ^ 0x7685);
    let mut deposit = vec![0u8; 1 + 2 * 192];
    rng.fill(&mut deposit[..]);
    deposit[0] = 0x00;
    let mut withdrawal = vec![0u8; 1 + 76];
    rng.fill(&mut withdrawal[..]);
    withdrawal[0] = 0x01;
    vec![deposit, withdrawal]
}

#[allow(dead_code)]
fn requests_json(raw: &[Vec<u8>]) -> Vec<EncodedRequests> {
    raw.iter()
        .map(|r| EncodedRequests(Bytes::from(r.clone())))
        .collect()
}

#[allow(dead_code)]
fn requests_ssz(raw: &[Vec<u8>]) -> ExecutionRequestsList {
    let lists: Vec<_> = raw
        .iter()
        .map(|r| {
            r.clone()
                .try_into()
                .expect("request fits MAX_REQUEST_BYTES")
        })
        .collect();
    lists
        .try_into()
        .expect("requests fit MAX_EXECUTION_REQUESTS_PER_PAYLOAD")
}

// ── newPayload fixtures (JSON side) ──────────────────────────────────────────

/// JSON-shape ExecutionPayload for the given era (`engine_newPayloadV1..V4`
/// param 0). Requests (V4) ride as a separate param and are empty in the
/// benchmarks, matching the SSZ envelopes.
#[allow(dead_code)]
pub fn payload_json(seed: u64, tx_count: usize, era: PayloadEra) -> JsonExecutionPayload {
    let f = build_raw_fields(seed, tx_count);
    json_payload_from_raw(&f, era, None, None)
}

/// JSON-shape Amsterdam ExecutionPayload (`engine_newPayloadV5` param 0):
/// Cancun-era payload + `blockAccessList` (RLP hex) + `slotNumber`.
#[allow(dead_code)]
pub fn amsterdam_payload_json(
    seed: u64,
    tx_count: usize,
    bal_accounts: usize,
) -> JsonExecutionPayload {
    let f = build_raw_fields(seed, tx_count);
    let bal = build_synthetic_bal(seed, bal_accounts);
    json_payload_from_raw(&f, PayloadEra::Cancun, Some(bal), Some(DEFAULT_SLOT_NUMBER))
}

// ── newPayload fixtures (SSZ side) ───────────────────────────────────────────

/// `POST /engine/v1/payloads` body (Eth-Execution-Version: paris).
#[allow(dead_code)]
pub fn paris_newpayload_ssz(seed: u64, tx_count: usize) -> SszParisEnvelope {
    let f = build_raw_fields(seed, tx_count);
    SszParisEnvelope {
        execution_payload: raw_to_ssz_payload_paris(&f),
    }
}

/// `POST /engine/v1/payloads` body (Eth-Execution-Version: shanghai).
#[allow(dead_code)]
pub fn shanghai_newpayload_ssz(seed: u64, tx_count: usize) -> SszShanghaiEnvelope {
    let f = build_raw_fields(seed, tx_count);
    SszShanghaiEnvelope {
        execution_payload: raw_to_ssz_payload_shanghai(&f),
    }
}

/// `POST /engine/v1/payloads` body (Eth-Execution-Version: cancun).
#[allow(dead_code)]
pub fn cancun_newpayload_ssz(seed: u64, tx_count: usize) -> SszCancunEnvelope {
    let f = build_raw_fields(seed, tx_count);
    SszCancunEnvelope {
        execution_payload: raw_to_ssz_payload_cancun(&f),
        parent_beacon_block_root: [0u8; 32],
    }
}

/// `POST /engine/v1/payloads` body for Eth-Execution-Version prague and osaka (same shape;
/// Osaka re-exports the Prague envelope).
#[allow(dead_code)]
pub fn prague_newpayload_ssz(seed: u64, tx_count: usize) -> SszPragueEnvelope {
    let f = build_raw_fields(seed, tx_count);
    SszPragueEnvelope {
        execution_payload: raw_to_ssz_payload_prague(&f),
        parent_beacon_block_root: [0u8; 32],
        execution_requests: Vec::new().try_into().expect("empty requests fit"),
    }
}

/// `POST /engine/v1/payloads` body (Eth-Execution-Version: amsterdam). The BAL rides as the same RLP
/// bytes the JSON hex field carries.
#[allow(dead_code)]
pub fn amsterdam_newpayload_ssz(
    seed: u64,
    tx_count: usize,
    bal_accounts: usize,
) -> SszAmsterdamEnvelope {
    let f = build_raw_fields(seed, tx_count);
    let bal = build_synthetic_bal(seed, bal_accounts);
    SszAmsterdamEnvelope {
        execution_payload: raw_to_ssz_payload_amsterdam(&f, bal_rlp(&bal), DEFAULT_SLOT_NUMBER),
        parent_beacon_block_root: [0u8; 32],
        execution_requests: Vec::new().try_into().expect("empty requests fit"),
    }
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

// ── Internal payload builders ────────────────────────────────────────────────

/// Build the JSON payload via the public `from_block` constructor (struct
/// fields are pub(crate)), then patch in the synthetic tx bytes via a serde
/// round-trip — `block_hash` and `transactions` are the only fields
/// `from_block` can't set correctly for us.
fn json_payload_from_raw(
    f: &RawPayloadFields,
    era: PayloadEra,
    bal: Option<BlockAccessList>,
    slot_number: Option<u64>,
) -> JsonExecutionPayload {
    let payload = JsonExecutionPayload::from_block(raw_to_block(f, era, slot_number), bal);
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

#[allow(dead_code)]
fn raw_to_ssz_payload_paris(f: &RawPayloadFields) -> SszParisPayload {
    let p = ssz_payload_parts(f);
    SszParisPayload {
        parent_hash: f.parent_hash,
        fee_recipient: Bytes20(f.fee_recipient),
        state_root: f.state_root,
        receipts_root: f.receipts_root,
        logs_bloom: p.logs_bloom,
        prev_randao: f.prev_randao,
        block_number: f.block_number,
        gas_limit: f.gas_limit,
        gas_used: f.gas_used,
        timestamp: f.timestamp,
        extra_data: p.extra_data,
        base_fee_per_gas: p.base_fee,
        block_hash: [0u8; 32],
        transactions: p.transactions,
    }
}

#[allow(dead_code)]
fn raw_to_ssz_payload_shanghai(f: &RawPayloadFields) -> SszShanghaiPayload {
    let p = ssz_payload_parts(f);
    SszShanghaiPayload {
        parent_hash: f.parent_hash,
        fee_recipient: Bytes20(f.fee_recipient),
        state_root: f.state_root,
        receipts_root: f.receipts_root,
        logs_bloom: p.logs_bloom,
        prev_randao: f.prev_randao,
        block_number: f.block_number,
        gas_limit: f.gas_limit,
        gas_used: f.gas_used,
        timestamp: f.timestamp,
        extra_data: p.extra_data,
        base_fee_per_gas: p.base_fee,
        block_hash: [0u8; 32],
        transactions: p.transactions,
        withdrawals: p.withdrawals,
    }
}

#[allow(dead_code)]
fn raw_to_ssz_payload_cancun(f: &RawPayloadFields) -> SszCancunPayload {
    let p = ssz_payload_parts(f);
    SszCancunPayload {
        parent_hash: f.parent_hash,
        fee_recipient: Bytes20(f.fee_recipient),
        state_root: f.state_root,
        receipts_root: f.receipts_root,
        logs_bloom: p.logs_bloom,
        prev_randao: f.prev_randao,
        block_number: f.block_number,
        gas_limit: f.gas_limit,
        gas_used: f.gas_used,
        timestamp: f.timestamp,
        extra_data: p.extra_data,
        base_fee_per_gas: p.base_fee,
        block_hash: [0u8; 32],
        transactions: p.transactions,
        withdrawals: p.withdrawals,
        blob_gas_used: 0,
        excess_blob_gas: 0,
    }
}

#[allow(dead_code)]
fn raw_to_ssz_payload_prague(f: &RawPayloadFields) -> SszPraguePayload {
    let p = ssz_payload_parts(f);
    SszPraguePayload {
        parent_hash: f.parent_hash,
        fee_recipient: Bytes20(f.fee_recipient),
        state_root: f.state_root,
        receipts_root: f.receipts_root,
        logs_bloom: p.logs_bloom,
        prev_randao: f.prev_randao,
        block_number: f.block_number,
        gas_limit: f.gas_limit,
        gas_used: f.gas_used,
        timestamp: f.timestamp,
        extra_data: p.extra_data,
        base_fee_per_gas: p.base_fee,
        block_hash: [0u8; 32],
        transactions: p.transactions,
        withdrawals: p.withdrawals,
        blob_gas_used: 0,
        excess_blob_gas: 0,
    }
}

#[allow(dead_code)]
fn raw_to_ssz_payload_amsterdam(
    f: &RawPayloadFields,
    bal_bytes: Vec<u8>,
    slot_number: u64,
) -> SszAmsterdamPayload {
    use ethrex_rpc::engine_rest::types::common::MAX_BLOCK_ACCESS_LIST_BYTES;
    use libssz_types::SszList;

    let p = ssz_payload_parts(f);
    let block_access_list: SszList<u8, MAX_BLOCK_ACCESS_LIST_BYTES> = bal_bytes
        .try_into()
        .expect("BAL fits MAX_BLOCK_ACCESS_LIST_BYTES");
    SszAmsterdamPayload {
        parent_hash: f.parent_hash,
        fee_recipient: Bytes20(f.fee_recipient),
        state_root: f.state_root,
        receipts_root: f.receipts_root,
        logs_bloom: p.logs_bloom,
        prev_randao: f.prev_randao,
        block_number: f.block_number,
        gas_limit: f.gas_limit,
        gas_used: f.gas_used,
        timestamp: f.timestamp,
        extra_data: p.extra_data,
        base_fee_per_gas: p.base_fee,
        block_hash: [0u8; 32],
        transactions: p.transactions,
        withdrawals: p.withdrawals,
        blob_gas_used: 0,
        excess_blob_gas: 0,
        block_access_list,
        slot_number,
    }
}

/// The SSZ sub-fields shared by every fork's payload shape.
struct SszPayloadParts {
    logs_bloom: libssz_types::SszVector<u8, 256>,
    extra_data:
        libssz_types::SszList<u8, { ethrex_rpc::engine_rest::types::common::MAX_EXTRA_DATA_BYTES }>,
    base_fee: [u8; 32],
    transactions: libssz_types::SszList<
        libssz_types::SszList<
            u8,
            { ethrex_rpc::engine_rest::types::common::MAX_BYTES_PER_TRANSACTION },
        >,
        { ethrex_rpc::engine_rest::types::common::MAX_TRANSACTIONS_PER_PAYLOAD },
    >,
    withdrawals: libssz_types::SszList<
        ethrex_rpc::engine_rest::types::shanghai::Withdrawal,
        { ethrex_rpc::engine_rest::types::common::MAX_WITHDRAWALS_PER_PAYLOAD },
    >,
}

fn ssz_payload_parts(f: &RawPayloadFields) -> SszPayloadParts {
    use ethrex_rpc::engine_rest::types::common::MAX_BYTES_PER_TRANSACTION;
    use libssz_types::SszList;

    let mut base_fee = [0u8; 32];
    base_fee[..8].copy_from_slice(&f.base_fee_per_gas.to_le_bytes());

    let tx_lists: Vec<SszList<u8, MAX_BYTES_PER_TRANSACTION>> = f
        .tx_bytes
        .iter()
        .map(|b| {
            b.clone()
                .try_into()
                .expect("tx fits MAX_BYTES_PER_TRANSACTION")
        })
        .collect();

    SszPayloadParts {
        logs_bloom: vec![0u8; 256].try_into().expect("logs_bloom is 256 bytes"),
        extra_data: f
            .extra_data
            .clone()
            .try_into()
            .expect("extra_data fits MAX_EXTRA_DATA_BYTES"),
        base_fee,
        transactions: tx_lists
            .try_into()
            .expect("transactions fit MAX_TRANSACTIONS_PER_PAYLOAD"),
        withdrawals: Vec::new().try_into().expect("empty withdrawals fits"),
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

// ── Blobs-bundle raw data (shared by the getPayload response fixtures) ───────

/// Raw bundle bytes used by both the JSON and SSZ getPayload responses, so the
/// two sides carry identical logical content. `proofs_per_blob` is 1 for the
/// Cancun/Prague `BlobsBundleV1` and `CELLS_PER_EXT_BLOB` for the
/// Osaka/Amsterdam `BlobsBundleV2`.
#[allow(dead_code)]
struct RawBlobsBundle {
    /// Each `BYTES_PER_BLOB` long.
    blobs: Vec<Vec<u8>>,
    commitments: Vec<[u8; 48]>,
    proofs: Vec<[u8; 48]>,
}

#[allow(dead_code)]
fn build_raw_bundle(seed: u64, blob_count: usize, proofs_per_blob: usize) -> RawBlobsBundle {
    use ethrex_rpc::engine_rest::types::blobs::BYTES_PER_BLOB;

    // Offset the seed so the bundle bytes differ from the payload fields.
    let mut rng = StdRng::seed_from_u64(seed ^ 0xB10B);
    let blobs: Vec<Vec<u8>> = (0..blob_count)
        .map(|_| {
            let mut b = vec![0u8; BYTES_PER_BLOB];
            rng.fill(&mut b[..]);
            b
        })
        .collect();
    let commitments: Vec<[u8; 48]> = (0..blob_count)
        .map(|_| {
            let mut c = [0u8; 48];
            rng.fill(&mut c[..]);
            c
        })
        .collect();
    let proofs: Vec<[u8; 48]> = (0..blob_count * proofs_per_blob)
        .map(|_| {
            let mut p = [0u8; 48];
            rng.fill(&mut p[..]);
            p
        })
        .collect();
    RawBlobsBundle {
        blobs,
        commitments,
        proofs,
    }
}

#[allow(dead_code)]
fn json_blobs_bundle(bundle: &RawBlobsBundle) -> BlobsBundle {
    let blobs: Vec<ethrex_common::types::Blob> = bundle
        .blobs
        .iter()
        .map(|b| b.as_slice().try_into().expect("blob is BYTES_PER_BLOB"))
        .collect();
    BlobsBundle {
        blobs,
        commitments: bundle.commitments.clone(),
        proofs: bundle.proofs.clone(),
        version: 0,
    }
}

#[allow(dead_code)]
fn ssz_blobs_bundle_v1(bundle: RawBlobsBundle) -> BlobsBundleV1 {
    use ethrex_rpc::engine_rest::types::blobs::BYTES_PER_BLOB;
    use libssz_types::SszVector;

    let blobs: Vec<SszVector<u8, BYTES_PER_BLOB>> = bundle
        .blobs
        .into_iter()
        .map(|b| b.try_into().expect("blob fits BYTES_PER_BLOB"))
        .collect();
    BlobsBundleV1 {
        commitments: bundle
            .commitments
            .try_into()
            .expect("commitments fit MAX_BLOB_COMMITMENTS_PER_BLOCK"),
        proofs: bundle
            .proofs
            .try_into()
            .expect("proofs fit MAX_BLOB_COMMITMENTS_PER_BLOCK"),
        blobs: blobs
            .try_into()
            .expect("blobs fit MAX_BLOB_COMMITMENTS_PER_BLOCK"),
    }
}

#[allow(dead_code)]
fn ssz_blobs_bundle_v2(bundle: RawBlobsBundle) -> BlobsBundleV2 {
    use ethrex_rpc::engine_rest::types::blobs::BYTES_PER_BLOB;
    use libssz_types::SszVector;

    let blobs: Vec<SszVector<u8, BYTES_PER_BLOB>> = bundle
        .blobs
        .into_iter()
        .map(|b| b.try_into().expect("blob fits BYTES_PER_BLOB"))
        .collect();
    BlobsBundleV2 {
        commitments: bundle
            .commitments
            .try_into()
            .expect("commitments fit MAX_BLOB_COMMITMENTS_PER_BLOCK"),
        proofs: bundle
            .proofs
            .try_into()
            .expect("cell proofs fit MAX_CELL_PROOFS"),
        blobs: blobs
            .try_into()
            .expect("blobs fit MAX_BLOB_COMMITMENTS_PER_BLOCK"),
    }
}

// ── getPayload response fixtures ─────────────────────────────────────────────

/// JSON-side `engine_getPayloadV1` response: a bare ExecutionPayload (the
/// JSON V1 result carries no block value — the SSZ `BuiltPayloadParis` does;
/// this asymmetry is faithful to the wire).
#[allow(dead_code)]
pub fn getpayload_response_json_paris(seed: u64, tx_count: usize) -> JsonExecutionPayload {
    payload_json(seed, tx_count, PayloadEra::Paris)
}

/// SSZ-side `GET /engine/v1/payloads/{id}` response (Eth-Execution-Version: paris).
#[allow(dead_code)]
pub fn getpayload_response_ssz_paris(seed: u64, tx_count: usize) -> BuiltPayloadParis {
    let f = build_raw_fields(seed, tx_count);
    BuiltPayloadParis {
        payload: raw_to_ssz_payload_paris(&f),
        block_value: block_value_le(),
    }
}

/// JSON-side `engine_getPayloadV2` response: payload + block value (the
/// implementation serializes the full response struct with null optionals).
#[allow(dead_code)]
pub fn getpayload_response_json_shanghai(seed: u64, tx_count: usize) -> ExecutionPayloadResponse {
    let f = build_raw_fields(seed, tx_count);
    ExecutionPayloadResponse {
        execution_payload: json_payload_from_raw(&f, PayloadEra::Shanghai, None, None),
        block_value: U256::from(BLOCK_VALUE_WEI),
        blobs_bundle: None,
        should_override_builder: None,
        execution_requests: None,
    }
}

/// SSZ-side `GET /engine/v1/payloads/{id}` response (Eth-Execution-Version: shanghai).
#[allow(dead_code)]
pub fn getpayload_response_ssz_shanghai(seed: u64, tx_count: usize) -> BuiltPayloadShanghai {
    let f = build_raw_fields(seed, tx_count);
    BuiltPayloadShanghai {
        payload: raw_to_ssz_payload_shanghai(&f),
        block_value: block_value_le(),
    }
}

/// JSON-side `engine_getPayloadV3` response: + `BlobsBundleV1` (1 proof/blob).
#[allow(dead_code)]
pub fn getpayload_response_json_cancun(
    seed: u64,
    tx_count: usize,
    blob_count: usize,
) -> ExecutionPayloadResponse {
    let f = build_raw_fields(seed, tx_count);
    ExecutionPayloadResponse {
        execution_payload: json_payload_from_raw(&f, PayloadEra::Cancun, None, None),
        block_value: U256::from(BLOCK_VALUE_WEI),
        blobs_bundle: Some(json_blobs_bundle(&build_raw_bundle(seed, blob_count, 1))),
        should_override_builder: Some(false),
        execution_requests: None,
    }
}

/// SSZ-side `GET /engine/v1/payloads/{id}` response (Eth-Execution-Version: cancun).
#[allow(dead_code)]
pub fn getpayload_response_ssz_cancun(
    seed: u64,
    tx_count: usize,
    blob_count: usize,
) -> BuiltPayloadCancun {
    let f = build_raw_fields(seed, tx_count);
    BuiltPayloadCancun {
        payload: raw_to_ssz_payload_cancun(&f),
        block_value: block_value_le(),
        blobs_bundle: ssz_blobs_bundle_v1(build_raw_bundle(seed, blob_count, 1)),
        should_override_builder: false,
    }
}

/// JSON-side `engine_getPayloadV4` response: V3 + execution requests.
#[allow(dead_code)]
pub fn getpayload_response_json_prague(
    seed: u64,
    tx_count: usize,
    blob_count: usize,
) -> ExecutionPayloadResponse {
    let f = build_raw_fields(seed, tx_count);
    ExecutionPayloadResponse {
        execution_payload: json_payload_from_raw(&f, PayloadEra::Cancun, None, None),
        block_value: U256::from(BLOCK_VALUE_WEI),
        blobs_bundle: Some(json_blobs_bundle(&build_raw_bundle(seed, blob_count, 1))),
        should_override_builder: Some(false),
        execution_requests: Some(requests_json(&build_raw_requests(seed))),
    }
}

/// SSZ-side `GET /engine/v1/payloads/{id}` response (Eth-Execution-Version: prague).
#[allow(dead_code)]
pub fn getpayload_response_ssz_prague(
    seed: u64,
    tx_count: usize,
    blob_count: usize,
) -> BuiltPayloadPrague {
    let f = build_raw_fields(seed, tx_count);
    BuiltPayloadPrague {
        payload: raw_to_ssz_payload_prague(&f),
        block_value: block_value_le(),
        blobs_bundle: ssz_blobs_bundle_v1(build_raw_bundle(seed, blob_count, 1)),
        execution_requests: requests_ssz(&build_raw_requests(seed)),
        should_override_builder: false,
    }
}

/// JSON-side `engine_getPayloadV5` (Osaka) response: cell-proof bundle.
#[allow(dead_code)]
pub fn getpayload_response_json_osaka(
    seed: u64,
    tx_count: usize,
    blob_count: usize,
) -> ExecutionPayloadResponse {
    use ethrex_rpc::engine_rest::types::blobs::CELLS_PER_EXT_BLOB;
    let f = build_raw_fields(seed, tx_count);
    ExecutionPayloadResponse {
        execution_payload: json_payload_from_raw(&f, PayloadEra::Cancun, None, None),
        block_value: U256::from(BLOCK_VALUE_WEI),
        blobs_bundle: Some(json_blobs_bundle(&build_raw_bundle(
            seed,
            blob_count,
            CELLS_PER_EXT_BLOB,
        ))),
        should_override_builder: Some(false),
        execution_requests: Some(requests_json(&build_raw_requests(seed))),
    }
}

/// SSZ-side `GET /engine/v1/payloads/{id}` response (Eth-Execution-Version: osaka).
#[allow(dead_code)]
pub fn getpayload_response_ssz_osaka(
    seed: u64,
    tx_count: usize,
    blob_count: usize,
) -> BuiltPayloadOsaka {
    use ethrex_rpc::engine_rest::types::blobs::CELLS_PER_EXT_BLOB;
    let f = build_raw_fields(seed, tx_count);
    BuiltPayloadOsaka {
        payload: raw_to_ssz_payload_prague(&f),
        block_value: block_value_le(),
        blobs_bundle: ssz_blobs_bundle_v2(build_raw_bundle(seed, blob_count, CELLS_PER_EXT_BLOB)),
        execution_requests: requests_ssz(&build_raw_requests(seed)),
        should_override_builder: false,
    }
}

/// JSON-side `engine_getPayloadV6` (Amsterdam) response: V5 plus the BAL
/// inside the execution payload.
#[allow(dead_code)]
pub fn getpayload_response_json_amsterdam(
    seed: u64,
    tx_count: usize,
    blob_count: usize,
    bal_accounts: usize,
) -> ExecutionPayloadResponse {
    use ethrex_rpc::engine_rest::types::blobs::CELLS_PER_EXT_BLOB;
    let f = build_raw_fields(seed, tx_count);
    let bal = build_synthetic_bal(seed, bal_accounts);
    ExecutionPayloadResponse {
        execution_payload: json_payload_from_raw(
            &f,
            PayloadEra::Cancun,
            Some(bal),
            Some(DEFAULT_SLOT_NUMBER),
        ),
        block_value: U256::from(BLOCK_VALUE_WEI),
        blobs_bundle: Some(json_blobs_bundle(&build_raw_bundle(
            seed,
            blob_count,
            CELLS_PER_EXT_BLOB,
        ))),
        should_override_builder: Some(false),
        execution_requests: Some(requests_json(&build_raw_requests(seed))),
    }
}

/// SSZ-side `GET /engine/v1/payloads/{id}` response (Eth-Execution-Version: amsterdam).
#[allow(dead_code)]
pub fn getpayload_response_ssz_amsterdam(
    seed: u64,
    tx_count: usize,
    blob_count: usize,
    bal_accounts: usize,
) -> BuiltPayloadAmsterdam {
    use ethrex_rpc::engine_rest::types::blobs::CELLS_PER_EXT_BLOB;
    let f = build_raw_fields(seed, tx_count);
    let bal = build_synthetic_bal(seed, bal_accounts);
    BuiltPayloadAmsterdam {
        payload: raw_to_ssz_payload_amsterdam(&f, bal_rlp(&bal), DEFAULT_SLOT_NUMBER),
        block_value: block_value_le(),
        blobs_bundle: ssz_blobs_bundle_v2(build_raw_bundle(seed, blob_count, CELLS_PER_EXT_BLOB)),
        execution_requests: requests_ssz(&build_raw_requests(seed)),
        should_override_builder: false,
    }
}

#[allow(dead_code)]
fn block_value_le() -> [u8; 32] {
    let mut v = [0u8; 32];
    v[..8].copy_from_slice(&BLOCK_VALUE_WEI.to_le_bytes());
    v
}

// ── Bodies response fixtures ─────────────────────────────────────────────────

/// JSON-side bodies response (`…BodiesByRangeV1`):
/// `Vec<Option<ExecutionPayloadBody>>` with `tx_per_body` random txs each.
/// `with_withdrawals` distinguishes the Paris era (`null`) from Shanghai+
/// (empty list).
#[allow(dead_code)]
pub fn bodies_range_json(
    seed: u64,
    n: usize,
    tx_per_body: usize,
    with_withdrawals: bool,
) -> Vec<Option<ethrex_rpc::types::payload::ExecutionPayloadBody>> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            Some(ethrex_rpc::types::payload::ExecutionPayloadBody {
                transactions: body_txs(&mut rng, tx_per_body),
                withdrawals: if with_withdrawals { Some(vec![]) } else { None },
            })
        })
        .collect()
}

/// SSZ-side `GET /engine/v1/bodies` response (Eth-Execution-Version: paris).
#[allow(dead_code)]
pub fn bodies_range_ssz_paris(
    seed: u64,
    n: usize,
    tx_per_body: usize,
) -> ethrex_rpc::engine_rest::types::bodies::BodiesResponseParis {
    use ethrex_rpc::engine_rest::types::bodies::{BodyEntryParis, BodyParis};

    let mut rng = StdRng::seed_from_u64(seed);
    let entries_vec: Vec<BodyEntryParis> = (0..n)
        .map(|_| {
            BodyEntryParis::available(BodyParis {
                transactions: body_txs_ssz(&mut rng, tx_per_body),
            })
        })
        .collect();
    ethrex_rpc::engine_rest::types::bodies::BodiesResponseParis {
        entries: entries_vec
            .try_into()
            .expect("bodies fit MAX_BODIES_PER_REQUEST"),
    }
}

/// SSZ-side bodies response for the shanghai → osaka paths (Shanghai shape).
#[allow(dead_code)]
pub fn bodies_range_ssz_shanghai(
    seed: u64,
    n: usize,
    tx_per_body: usize,
) -> ethrex_rpc::engine_rest::types::bodies::BodiesResponseShanghai {
    use ethrex_rpc::engine_rest::types::bodies::{BodyEntryShanghai, BodyShanghai};

    let mut rng = StdRng::seed_from_u64(seed);
    let entries_vec: Vec<BodyEntryShanghai> = (0..n)
        .map(|_| {
            BodyEntryShanghai::available(BodyShanghai {
                transactions: body_txs_ssz(&mut rng, tx_per_body),
                withdrawals: Vec::new().try_into().expect("empty withdrawals fits"),
            })
        })
        .collect();
    ethrex_rpc::engine_rest::types::bodies::BodiesResponseShanghai {
        entries: entries_vec
            .try_into()
            .expect("bodies fit MAX_BODIES_PER_REQUEST"),
    }
}

/// JSON-side Amsterdam bodies response (`…BodiesByRangeV2`): adds a per-body
/// BAL.
#[allow(dead_code)]
pub fn bodies_range_json_amsterdam(
    seed: u64,
    n: usize,
    tx_per_body: usize,
    bal_accounts: usize,
) -> Vec<Option<ethrex_rpc::types::payload::ExecutionPayloadBodyV2>> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|i| {
            Some(ethrex_rpc::types::payload::ExecutionPayloadBodyV2 {
                transactions: body_txs(&mut rng, tx_per_body),
                withdrawals: Some(vec![]),
                block_access_list: Some(build_synthetic_bal(
                    seed.wrapping_add(i as u64),
                    bal_accounts,
                )),
            })
        })
        .collect()
}

/// SSZ-side Amsterdam bodies response (`GET /engine/v1/bodies`, Eth-Execution-Version: amsterdam).
#[allow(dead_code)]
pub fn bodies_range_ssz_amsterdam(
    seed: u64,
    n: usize,
    tx_per_body: usize,
    bal_accounts: usize,
) -> ethrex_rpc::engine_rest::types::bodies::BodiesResponseAmsterdam {
    use ethrex_rpc::engine_rest::types::bodies::{BodyAmsterdam, BodyEntryAmsterdam};

    let mut rng = StdRng::seed_from_u64(seed);
    let entries_vec: Vec<BodyEntryAmsterdam> = (0..n)
        .map(|i| {
            let bal = build_synthetic_bal(seed.wrapping_add(i as u64), bal_accounts);
            BodyEntryAmsterdam::available(BodyAmsterdam {
                transactions: body_txs_ssz(&mut rng, tx_per_body),
                withdrawals: Vec::new().try_into().expect("empty withdrawals fits"),
                block_access_list: bal_rlp(&bal)
                    .try_into()
                    .expect("BAL fits MAX_BLOCK_ACCESS_LIST_BYTES"),
            })
        })
        .collect();
    ethrex_rpc::engine_rest::types::bodies::BodiesResponseAmsterdam {
        entries: entries_vec
            .try_into()
            .expect("bodies fit MAX_BODIES_PER_REQUEST"),
    }
}

#[allow(dead_code)]
fn body_txs(
    rng: &mut StdRng,
    tx_per_body: usize,
) -> Vec<ethrex_rpc::types::payload::EncodedTransaction> {
    use ethrex_rpc::types::payload::EncodedTransaction;
    (0..tx_per_body)
        .map(|_| {
            let mut buf = vec![0u8; 200];
            rng.fill(&mut buf[..]);
            buf[0] = 0x02;
            EncodedTransaction(Bytes::from(buf))
        })
        .collect()
}

#[allow(dead_code)]
fn body_txs_ssz(
    rng: &mut StdRng,
    tx_per_body: usize,
) -> libssz_types::SszList<
    libssz_types::SszList<
        u8,
        { ethrex_rpc::engine_rest::types::common::MAX_BYTES_PER_TRANSACTION },
    >,
    { ethrex_rpc::engine_rest::types::common::MAX_TRANSACTIONS_PER_PAYLOAD },
> {
    use ethrex_rpc::engine_rest::types::common::MAX_BYTES_PER_TRANSACTION;
    use libssz_types::SszList;

    let tx_lists: Vec<SszList<u8, MAX_BYTES_PER_TRANSACTION>> = (0..tx_per_body)
        .map(|_| {
            let mut buf = vec![0u8; 200];
            rng.fill(&mut buf[..]);
            buf[0] = 0x02;
            buf.try_into().expect("tx fits MAX_BYTES_PER_TRANSACTION")
        })
        .collect();
    tx_lists
        .try_into()
        .expect("txs fit MAX_TRANSACTIONS_PER_PAYLOAD")
}

// ── Blobs response fixtures ──────────────────────────────────────────────────

/// JSON-side `engine_getBlobsV1` hit-path response: blob + single proof.
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

/// SSZ-side `/blobs/v1` hit-path response.
#[allow(dead_code)]
pub fn blobs_v1_response_ssz(
    seed: u64,
    n: usize,
) -> ethrex_rpc::engine_rest::types::blobs::BlobsV1Response {
    use ethrex_rpc::engine_rest::types::blobs::{
        BYTES_PER_BLOB, BYTES_PER_PROOF, BlobAndProofV1 as SszBlobAndProofV1, BlobV1Entry,
    };
    use libssz_types::SszVector;

    let mut rng = StdRng::seed_from_u64(seed);
    let entries: Vec<BlobV1Entry> = (0..n)
        .map(|_| {
            let mut blob = vec![0u8; BYTES_PER_BLOB];
            rng.fill(&mut blob[..]);
            let mut proof = [0u8; BYTES_PER_PROOF];
            rng.fill(&mut proof[..]);
            let blob_ssz: SszVector<u8, BYTES_PER_BLOB> =
                blob.try_into().expect("blob fits BYTES_PER_BLOB");
            BlobV1Entry::available(SszBlobAndProofV1 {
                blob: blob_ssz,
                proof,
            })
        })
        .collect();
    ethrex_rpc::engine_rest::types::blobs::BlobsV1Response {
        entries: entries.try_into().expect("n <= MAX_BLOBS_REQUEST"),
    }
}

/// SSZ-side `/blobs/v1` all-miss response: zero-padded full-size entries.
#[allow(dead_code)]
pub fn blobs_v1_response_ssz_allmiss(
    n: usize,
) -> ethrex_rpc::engine_rest::types::blobs::BlobsV1Response {
    use ethrex_rpc::engine_rest::types::blobs::BlobV1Entry;

    let entries: Vec<BlobV1Entry> = (0..n).map(|_| BlobV1Entry::unavailable()).collect();
    ethrex_rpc::engine_rest::types::blobs::BlobsV1Response {
        entries: entries.try_into().expect("n <= MAX_BLOBS_REQUEST"),
    }
}

/// JSON-side v1 all-miss response: `n` nulls.
#[allow(dead_code)]
pub fn blobs_v1_response_json_allmiss(
    n: usize,
) -> Vec<Option<ethrex_rpc::engine::blobs::BlobAndProofV1>> {
    (0..n).map(|_| None).collect()
}

/// JSON-side blobs response (`engine_getBlobsV2`/`V3` hit path):
/// `Vec<Option<BlobAndProofV2>>`, each entry a blob + `CELLS_PER_EXT_BLOB`
/// cell proofs. The hit-path shape is identical for v2 and v3.
#[allow(dead_code)]
pub fn blobs_v2_response_json(
    seed: u64,
    n: usize,
) -> Vec<Option<ethrex_rpc::engine::blobs::BlobAndProofV2>> {
    use ethrex_rpc::engine_rest::types::blobs::{
        BYTES_PER_BLOB, BYTES_PER_PROOF, CELLS_PER_EXT_BLOB,
    };

    let mut rng = StdRng::seed_from_u64(seed);
    (0..n)
        .map(|_| {
            let mut blob = [0u8; BYTES_PER_BLOB];
            rng.fill(&mut blob[..]);
            let proofs: Vec<[u8; BYTES_PER_PROOF]> = (0..CELLS_PER_EXT_BLOB)
                .map(|_| {
                    let mut p = [0u8; BYTES_PER_PROOF];
                    rng.fill(&mut p[..]);
                    p
                })
                .collect();
            Some(ethrex_rpc::engine::blobs::BlobAndProofV2 { blob, proofs })
        })
        .collect()
}

/// SSZ-side blobs v2/v3 response, all entries available. (`BlobsV2Response`
/// and `BlobsV3Response` are the same SSZ type.)
#[allow(dead_code)]
pub fn blobs_v2_response_ssz(
    seed: u64,
    n: usize,
) -> ethrex_rpc::engine_rest::types::blobs::BlobsV2Response {
    use ethrex_rpc::engine_rest::types::blobs::{
        BYTES_PER_BLOB, BYTES_PER_PROOF, BlobAndProofV2 as SszBlobAndProofV2, BlobV2Entry,
        CELLS_PER_EXT_BLOB,
    };
    use libssz_types::SszVector;

    let mut rng = StdRng::seed_from_u64(seed);
    let entries: Vec<BlobV2Entry> = (0..n)
        .map(|_| {
            let mut blob = vec![0u8; BYTES_PER_BLOB];
            rng.fill(&mut blob[..]);
            let proofs: Vec<[u8; BYTES_PER_PROOF]> = (0..CELLS_PER_EXT_BLOB)
                .map(|_| {
                    let mut p = [0u8; BYTES_PER_PROOF];
                    rng.fill(&mut p[..]);
                    p
                })
                .collect();
            let blob_ssz: SszVector<u8, BYTES_PER_BLOB> =
                blob.try_into().expect("blob fits BYTES_PER_BLOB");
            BlobV2Entry::available(SszBlobAndProofV2 {
                blob: blob_ssz,
                proofs: proofs.try_into().expect("proofs fit CELLS_PER_EXT_BLOB"),
            })
        })
        .collect();
    ethrex_rpc::engine_rest::types::blobs::BlobsV2Response {
        entries: entries.try_into().expect("n <= MAX_BLOBS_REQUEST"),
    }
}

/// SSZ-side `/blobs/v3` all-miss response: every entry `available == false`
/// with zero-valued contents. This is the v3 miss-path pathology — each missed
/// entry still ships a zeroed full-size blob. (JSON's all-miss is `n` nulls;
/// v2's all-miss is `null`/`204 No Content` on JSON/REST respectively.)
#[allow(dead_code)]
pub fn blobs_v3_response_ssz_allmiss(
    n: usize,
) -> ethrex_rpc::engine_rest::types::blobs::BlobsV3Response {
    use ethrex_rpc::engine_rest::types::blobs::BlobV2Entry;

    let entries: Vec<BlobV2Entry> = (0..n).map(|_| BlobV2Entry::unavailable()).collect();
    ethrex_rpc::engine_rest::types::blobs::BlobsV3Response {
        entries: entries.try_into().expect("n <= MAX_BLOBS_REQUEST"),
    }
}

/// JSON-side v3 all-miss response: `n` nulls.
#[allow(dead_code)]
pub fn blobs_v3_response_json_allmiss(
    n: usize,
) -> Vec<Option<ethrex_rpc::engine::blobs::BlobAndProofV2>> {
    (0..n).map(|_| None).collect()
}

/// SSZ-side `/blobs/v4` hit-path response: per-cell nullable cells + proofs,
/// all present. REST-only — there is no JSON `engine_getBlobsV4`, and the
/// production handler currently answers 204 (no per-cell mempool storage), so
/// this measures the spec's wire shape, not a live server path.
#[allow(dead_code)]
pub fn blobs_v4_response_ssz(
    seed: u64,
    n: usize,
) -> ethrex_rpc::engine_rest::types::blobs::BlobsV4Response {
    use ethrex_rpc::engine_rest::types::blobs::{
        BYTES_PER_CELL, BYTES_PER_PROOF, BlobCellsAndProofs, BlobV4Entry, CELLS_PER_EXT_BLOB,
    };
    use libssz_types::{SszList, SszVector};

    let mut rng = StdRng::seed_from_u64(seed);
    let entries: Vec<BlobV4Entry> = (0..n)
        .map(|_| {
            let cells: Vec<SszList<SszVector<u8, BYTES_PER_CELL>, 1>> = (0..CELLS_PER_EXT_BLOB)
                .map(|_| {
                    let mut cell = vec![0u8; BYTES_PER_CELL];
                    rng.fill(&mut cell[..]);
                    let cell: SszVector<u8, BYTES_PER_CELL> =
                        cell.try_into().expect("cell fits BYTES_PER_CELL");
                    vec![cell].try_into().expect("one cell fits List[_, 1]")
                })
                .collect();
            let proofs: Vec<SszList<[u8; BYTES_PER_PROOF], 1>> = (0..CELLS_PER_EXT_BLOB)
                .map(|_| {
                    let mut p = [0u8; BYTES_PER_PROOF];
                    rng.fill(&mut p[..]);
                    vec![p].try_into().expect("one proof fits List[_, 1]")
                })
                .collect();
            BlobV4Entry {
                available: true,
                contents: BlobCellsAndProofs {
                    blob_cells: cells.try_into().expect("cells fit CELLS_PER_EXT_BLOB"),
                    proofs: proofs.try_into().expect("proofs fit CELLS_PER_EXT_BLOB"),
                },
            }
        })
        .collect();
    ethrex_rpc::engine_rest::types::blobs::BlobsV4Response {
        entries: entries.try_into().expect("n <= MAX_BLOBS_REQUEST"),
    }
}
