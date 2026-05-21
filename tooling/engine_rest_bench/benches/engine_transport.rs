//! Microbenchmarks comparing SSZ-REST (PR #764) wire codec to JSON-RPC for
//! the hottest Engine API messages. Run with:
//!
//!     cargo bench -p ethrex-rpc --bench engine_transport

#![allow(clippy::unwrap_used)]

use bytes::Bytes;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use ethrex_common::{Address, Bloom, H256};
use ethrex_rpc::engine_rest::types::common::{
    MAX_BYTES_PER_TRANSACTION, MAX_TRANSACTIONS_PER_PAYLOAD, MAX_WITHDRAWALS_PER_PAYLOAD,
    u64_to_uint256_le,
};
use ethrex_rpc::engine_rest::types::execution_payload::ExecutionPayloadV4;
use ethrex_rpc::engine_rest::types::new_payload::{
    BlobVersionedHashes, ExecutionRequests, NewPayloadV5Request,
};
use ethrex_rpc::engine_rest::types::withdrawal::WithdrawalV1;
use libssz::{SszDecode, SszEncode};
use libssz_types::SszList;

// ── Builders ──────────────────────────────────────────────────────────────────

/// Build a realistic NewPayloadV5Request (Amsterdam) with `n_txs` random-sized
/// transactions, full withdrawals slate, and `n_blobs` blob hashes.
fn build_new_payload_v5(n_txs: usize, n_blobs: usize) -> NewPayloadV5Request {
    let withdrawals: Vec<WithdrawalV1> = (0..MAX_WITHDRAWALS_PER_PAYLOAD)
        .map(|i| WithdrawalV1 {
            index: i as u64,
            validator_index: 1_000_000 + i as u64,
            address: [0x11; 20],
            amount: 32_000_000_000,
        })
        .collect();

    // Average post-Cancun L1 tx is ~120 bytes; use 256 to overshoot a bit.
    let tx_body = vec![0xab; 256];
    let txs: Vec<SszList<u8, MAX_BYTES_PER_TRANSACTION>> = (0..n_txs)
        .map(|_| {
            tx_body
                .clone()
                .try_into()
                .expect("tx body fits MAX_BYTES_PER_TRANSACTION")
        })
        .collect();
    let txs_ssz: SszList<SszList<u8, MAX_BYTES_PER_TRANSACTION>, MAX_TRANSACTIONS_PER_PAYLOAD> =
        txs.try_into()
            .expect("txs fit MAX_TRANSACTIONS_PER_PAYLOAD");

    let exec_payload = ExecutionPayloadV4 {
        parent_hash: [0xaa; 32],
        fee_recipient: [0x42; 20],
        state_root: [0xbb; 32],
        receipts_root: [0xcc; 32],
        logs_bloom: [0xdd; 256],
        prev_randao: [0xee; 32],
        block_number: 21_000_000,
        gas_limit: 30_000_000,
        gas_used: 28_500_000,
        timestamp: 1_730_000_000,
        extra_data: vec![0x65, 0x74, 0x68, 0x72, 0x65, 0x78]
            .try_into()
            .expect("extra_data ≤ 32"),
        base_fee_per_gas: u64_to_uint256_le(15_000_000_000),
        block_hash: [0xff; 32],
        transactions: txs_ssz,
        withdrawals: withdrawals.try_into().expect("withdrawals fit"),
        blob_gas_used: 786_432,
        excess_blob_gas: 196_608,
        block_access_list: vec![0u8; 4096].try_into().expect("BAL ≤ MAX_BYTES"),
        slot_number: 8_400_000,
    };

    let blob_hashes: Vec<[u8; 32]> = (0..n_blobs).map(|i| [i as u8; 32]).collect();
    let expected_blob_versioned_hashes: BlobVersionedHashes = blob_hashes
        .try_into()
        .expect("blob hashes ≤ MAX_BLOB_COMMITMENTS_PER_BLOCK");

    let execution_requests: ExecutionRequests =
        Vec::<SszList<u8, MAX_BYTES_PER_TRANSACTION>>::new()
            .try_into()
            .expect("empty execution_requests");

    NewPayloadV5Request {
        execution_payload: exec_payload,
        expected_blob_versioned_hashes,
        parent_beacon_block_root: [0x77; 32],
        execution_requests,
    }
}

// ── JSON shadow type for fair comparison ──────────────────────────────────────
//
// We bench against the production `ethrex_rpc::types::payload::ExecutionPayload`
// via `from_block` would require building a Block. To keep the comparison
// scoped to wire codec cost, we use a serde_json round-trip on a struct that
// mirrors the SSZ container's fields with hex-encoded bytes (matching the
// JSON-RPC Engine API wire format).

#[derive(serde::Serialize, serde::Deserialize)]
struct JsonExecutionPayloadV4 {
    #[serde(rename = "parentHash")]
    parent_hash: H256,
    #[serde(rename = "feeRecipient")]
    fee_recipient: Address,
    #[serde(rename = "stateRoot")]
    state_root: H256,
    #[serde(rename = "receiptsRoot")]
    receipts_root: H256,
    #[serde(rename = "logsBloom")]
    logs_bloom: Bloom,
    #[serde(rename = "prevRandao")]
    prev_randao: H256,
    #[serde(
        rename = "blockNumber",
        with = "ethrex_common::serde_utils::u64::hex_str"
    )]
    block_number: u64,
    #[serde(rename = "gasLimit", with = "ethrex_common::serde_utils::u64::hex_str")]
    gas_limit: u64,
    #[serde(rename = "gasUsed", with = "ethrex_common::serde_utils::u64::hex_str")]
    gas_used: u64,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    timestamp: u64,
    #[serde(rename = "extraData", with = "ethrex_common::serde_utils::bytes")]
    extra_data: Bytes,
    #[serde(
        rename = "baseFeePerGas",
        with = "ethrex_common::serde_utils::u64::hex_str"
    )]
    base_fee_per_gas: u64,
    #[serde(rename = "blockHash")]
    block_hash: H256,
    transactions: Vec<JsonBytes>,
    withdrawals: Vec<JsonWithdrawal>,
    #[serde(
        rename = "blobGasUsed",
        with = "ethrex_common::serde_utils::u64::hex_str"
    )]
    blob_gas_used: u64,
    #[serde(
        rename = "excessBlobGas",
        with = "ethrex_common::serde_utils::u64::hex_str"
    )]
    excess_blob_gas: u64,
    // V4 (Amsterdam) additions:
    #[serde(rename = "blockAccessList", with = "ethrex_common::serde_utils::bytes")]
    block_access_list: Bytes,
    #[serde(
        rename = "slotNumber",
        with = "ethrex_common::serde_utils::u64::hex_str"
    )]
    slot_number: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct JsonWithdrawal {
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    index: u64,
    #[serde(
        rename = "validatorIndex",
        with = "ethrex_common::serde_utils::u64::hex_str"
    )]
    validator_index: u64,
    address: Address,
    #[serde(with = "ethrex_common::serde_utils::u64::hex_str")]
    amount: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct JsonBytes(#[serde(with = "ethrex_common::serde_utils::bytes")] Bytes);

/// JSON-RPC `engine_newPayloadV5` params are a 4-element array:
/// [executionPayloadV4, expectedBlobVersionedHashes, parentBeaconBlockRoot, executionRequests].
/// Serializing a tuple yields exactly that JSON array, matching what an EL receives.
type JsonNewPayloadV5Request = (JsonExecutionPayloadV4, Vec<H256>, H256, Vec<JsonBytes>);

fn build_json_request(req: &NewPayloadV5Request) -> JsonNewPayloadV5Request {
    let p = &req.execution_payload;
    let payload = JsonExecutionPayloadV4 {
        parent_hash: H256(p.parent_hash),
        fee_recipient: Address::from_slice(&p.fee_recipient),
        state_root: H256(p.state_root),
        receipts_root: H256(p.receipts_root),
        logs_bloom: Bloom(p.logs_bloom),
        prev_randao: H256(p.prev_randao),
        block_number: p.block_number,
        gas_limit: p.gas_limit,
        gas_used: p.gas_used,
        timestamp: p.timestamp,
        extra_data: Bytes::copy_from_slice(&p.extra_data),
        base_fee_per_gas: 15_000_000_000,
        block_hash: H256(p.block_hash),
        transactions: p
            .transactions
            .iter()
            .map(|raw| JsonBytes(Bytes::copy_from_slice(raw)))
            .collect(),
        withdrawals: p
            .withdrawals
            .iter()
            .map(|w| JsonWithdrawal {
                index: w.index,
                validator_index: w.validator_index,
                address: Address::from_slice(&w.address),
                amount: w.amount,
            })
            .collect(),
        blob_gas_used: p.blob_gas_used,
        excess_blob_gas: p.excess_blob_gas,
        block_access_list: Bytes::copy_from_slice(&p.block_access_list),
        slot_number: p.slot_number,
    };

    let expected_blob_versioned_hashes: Vec<H256> = req
        .expected_blob_versioned_hashes
        .iter()
        .map(|h| H256(*h))
        .collect();
    let parent_beacon_block_root = H256(req.parent_beacon_block_root);
    let execution_requests: Vec<JsonBytes> = req
        .execution_requests
        .iter()
        .map(|raw| JsonBytes(Bytes::copy_from_slice(raw)))
        .collect();

    (
        payload,
        expected_blob_versioned_hashes,
        parent_beacon_block_root,
        execution_requests,
    )
}

// ── Benches ───────────────────────────────────────────────────────────────────

fn bench_codec(c: &mut Criterion) {
    // Realistic Amsterdam newPayload: ~150 txs, 6 blob hashes.
    let req = build_new_payload_v5(150, 6);
    let json = build_json_request(&req);

    let ssz_bytes = {
        let mut buf = Vec::with_capacity(req.encoded_len());
        req.ssz_append(&mut buf);
        buf
    };
    let json_bytes = serde_json::to_vec(&json).unwrap();

    eprintln!(
        "wire size: SSZ = {} B, JSON = {} B  (SSZ is {:.0}% of JSON)",
        ssz_bytes.len(),
        json_bytes.len(),
        100.0 * ssz_bytes.len() as f64 / json_bytes.len() as f64,
    );

    let mut group = c.benchmark_group("new_payload_v5_encode_150tx_6blob");
    group.throughput(Throughput::Bytes(ssz_bytes.len() as u64));
    group.bench_function("ssz_encode", |b| {
        b.iter(|| {
            let r = std::hint::black_box(&req);
            let mut buf = Vec::with_capacity(r.encoded_len());
            r.ssz_append(&mut buf);
            std::hint::black_box(buf);
        })
    });
    group.throughput(Throughput::Bytes(json_bytes.len() as u64));
    group.bench_function("json_encode", |b| {
        b.iter(|| {
            let v = serde_json::to_vec(std::hint::black_box(&json)).unwrap();
            std::hint::black_box(v);
        })
    });
    group.finish();

    let mut group = c.benchmark_group("new_payload_v5_decode_150tx_6blob");
    group.throughput(Throughput::Bytes(ssz_bytes.len() as u64));
    group.bench_function("ssz_decode", |b| {
        b.iter(|| {
            let v = NewPayloadV5Request::from_ssz_bytes(std::hint::black_box(&ssz_bytes)).unwrap();
            std::hint::black_box(v);
        })
    });
    group.throughput(Throughput::Bytes(json_bytes.len() as u64));
    group.bench_function("json_decode", |b| {
        b.iter(|| {
            let v: JsonNewPayloadV5Request =
                serde_json::from_slice(std::hint::black_box(&json_bytes)).unwrap();
            std::hint::black_box(v);
        })
    });
    group.finish();
}

// Blob-heavy variant: GetPayload V6 carries 6× 131KB blobs in real Amsterdam
// blocks. Bench just the blob list to isolate the hex-encoding overhead.
fn bench_blob_list(c: &mut Criterion) {
    use ethrex_rpc::engine_rest::types::blobs::{BlobAndProofV2, GetBlobsV2Response};
    use ethrex_rpc::engine_rest::types::common::{
        BLOB_SIZE, CELLS_PER_EXT_BLOB, MAX_BLOB_HASHES_REQUEST,
    };

    let blob = [0xcd; BLOB_SIZE];
    let proofs: SszList<[u8; 48], CELLS_PER_EXT_BLOB> =
        vec![[0xef; 48]; CELLS_PER_EXT_BLOB].try_into().unwrap();

    let items: Vec<BlobAndProofV2> = (0..6)
        .map(|_| BlobAndProofV2 {
            blob,
            proofs: proofs.clone(),
        })
        .collect();
    let resp = GetBlobsV2Response {
        blobs_and_proofs: items.try_into().unwrap(),
    };
    let _ = MAX_BLOB_HASHES_REQUEST;

    let ssz_bytes = {
        let mut buf = Vec::with_capacity(resp.encoded_len());
        resp.ssz_append(&mut buf);
        buf
    };

    // JSON shadow: hex-encoded blob bytes per spec.
    #[derive(serde::Serialize, serde::Deserialize)]
    struct JsonBlob(#[serde(with = "ethrex_common::serde_utils::bytes")] Bytes);
    #[derive(serde::Serialize, serde::Deserialize)]
    struct JsonBlobAndProofs {
        blob: JsonBlob,
        proofs: Vec<JsonBlob>,
    }
    let json_items: Vec<JsonBlobAndProofs> = resp
        .blobs_and_proofs
        .iter()
        .map(|i| JsonBlobAndProofs {
            blob: JsonBlob(Bytes::copy_from_slice(&i.blob)),
            proofs: i
                .proofs
                .iter()
                .map(|p| JsonBlob(Bytes::copy_from_slice(p)))
                .collect(),
        })
        .collect();
    let json_bytes = serde_json::to_vec(&json_items).unwrap();

    eprintln!(
        "blob bundle wire size: SSZ = {:.2} MB, JSON = {:.2} MB  (SSZ {:.0}%)",
        ssz_bytes.len() as f64 / 1e6,
        json_bytes.len() as f64 / 1e6,
        100.0 * ssz_bytes.len() as f64 / json_bytes.len() as f64,
    );

    let mut group = c.benchmark_group("blobs_v2_response_6_blobs");
    group.sample_size(20);
    group.throughput(Throughput::Bytes(ssz_bytes.len() as u64));
    group.bench_function("ssz_encode", |b| {
        b.iter(|| {
            let r = std::hint::black_box(&resp);
            let mut buf = Vec::with_capacity(r.encoded_len());
            r.ssz_append(&mut buf);
            std::hint::black_box(buf);
        })
    });
    group.throughput(Throughput::Bytes(json_bytes.len() as u64));
    group.bench_function("json_encode", |b| {
        b.iter(|| {
            let v = serde_json::to_vec(std::hint::black_box(&json_items)).unwrap();
            std::hint::black_box(v);
        })
    });
    group.throughput(Throughput::Bytes(ssz_bytes.len() as u64));
    group.bench_function("ssz_decode", |b| {
        b.iter(|| {
            let v = GetBlobsV2Response::from_ssz_bytes(std::hint::black_box(&ssz_bytes)).unwrap();
            std::hint::black_box(v);
        })
    });
    group.throughput(Throughput::Bytes(json_bytes.len() as u64));
    group.bench_function("json_decode", |b| {
        b.iter(|| {
            let v: Vec<JsonBlobAndProofs> =
                serde_json::from_slice(std::hint::black_box(&json_bytes)).unwrap();
            std::hint::black_box(v);
        })
    });
    group.finish();
}

// `blobs_bundle_to_ssz_v2` by-value (move) vs the previous &-borrow that copied
// each blob byte-by-byte. Fresh clone per iter so the clone cost is excluded.
fn bench_blobs_bundle_conversion(c: &mut Criterion) {
    use ethrex_common::types::{BYTES_PER_BLOB, BlobsBundle};
    use ethrex_rpc::engine_rest::conversions::blobs_bundle_to_ssz_v2;
    use ethrex_rpc::engine_rest::types::common::CELLS_PER_EXT_BLOB;

    const N_BLOBS: usize = 6; // Cancun blob target per block.
    let bundle = BlobsBundle {
        blobs: vec![[0xab; BYTES_PER_BLOB]; N_BLOBS],
        commitments: vec![[0xcd; 48]; N_BLOBS],
        proofs: vec![[0xef; 48]; N_BLOBS * CELLS_PER_EXT_BLOB],
        version: 1,
    };

    let mut group = c.benchmark_group("blobs_bundle_to_ssz_v2_6blob");
    group.sample_size(50);
    group.throughput(Throughput::Bytes((N_BLOBS * BYTES_PER_BLOB) as u64));

    group.bench_function("by_value_move", |b| {
        b.iter_batched(
            || bundle.clone(),
            |bnd| std::hint::black_box(blobs_bundle_to_ssz_v2(bnd).unwrap()),
            criterion::BatchSize::SmallInput,
        )
    });

    // Old &-borrow path: materialise fresh Vecs + per-blob copy_from_slice.
    group.bench_function("by_ref_copy", |b| {
        b.iter(|| {
            let r = std::hint::black_box(&bundle);
            let commitments: Vec<[u8; 48]> = r.commitments.to_vec();
            let proofs: Vec<[u8; 48]> = r.proofs.to_vec();
            let blobs: Vec<[u8; BYTES_PER_BLOB]> = r
                .blobs
                .iter()
                .map(|b| {
                    let mut arr = [0u8; BYTES_PER_BLOB];
                    arr.copy_from_slice(b.as_ref());
                    arr
                })
                .collect();
            std::hint::black_box((commitments, proofs, blobs));
        })
    });
    group.finish();
}

// Sequential vs `try_join_all` for `bodies_by_hash_v1`'s 32-key storage fan-out.
// Real win depends on whether the underlying KV layer parallelises read txns.
fn bench_body_lookups(c: &mut Criterion) {
    use ethrex_common::H256;
    use ethrex_common::types::{Block, BlockBody, BlockHeader};
    use ethrex_storage::{EngineType, Store};

    let rt = tokio::runtime::Runtime::new().unwrap();
    let storage = Store::new("", EngineType::InMemory).unwrap();
    let hashes: Vec<H256> = rt.block_on(async {
        let mut hashes = Vec::with_capacity(32);
        for i in 0..32u64 {
            let block = Block {
                header: BlockHeader {
                    number: i,
                    timestamp: 1_700_000_000 + i,
                    ..Default::default()
                },
                body: BlockBody::default(),
            };
            hashes.push(block.hash());
            storage.add_block(block).await.unwrap();
        }
        hashes
    });

    let mut group = c.benchmark_group("bodies_by_hash_v1_32_blocks");
    group.sample_size(20);

    group.bench_function("sequential", |b| {
        b.iter(|| {
            rt.block_on(async {
                let mut out = Vec::with_capacity(hashes.len());
                for h in &hashes {
                    out.push(storage.get_block_body_by_hash(*h).await.unwrap());
                }
                std::hint::black_box(out);
            })
        })
    });

    group.bench_function("join_all", |b| {
        b.iter(|| {
            rt.block_on(async {
                let futs = hashes.iter().map(|h| storage.get_block_body_by_hash(*h));
                let out = futures::future::try_join_all(futs).await.unwrap();
                std::hint::black_box(out);
            })
        })
    });
    group.finish();
}

// SSZ-tx → `Transaction` direct decode vs the previous `Bytes::copy_from_slice`
// + decode chain. Both arms produce `Vec<Transaction>` from the same input.
fn bench_ssz_tx_decoding(c: &mut Criterion) {
    use ethrex_common::types::{EIP1559Transaction, Transaction};
    use ethrex_rpc::engine_rest::types::common::MAX_BYTES_PER_TRANSACTION;
    use ethrex_rpc::engine_rest::types::execution_payload::Transactions;

    // A realistic-ish tx: EIP-1559 with ~512 bytes of calldata, encoding to
    // ~530 bytes. Matches the rough average of post-Cancun L1 transactions.
    let sample_tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        data: vec![0xab; 512].into(),
        ..Default::default()
    });
    let tx_bytes = sample_tx.encode_canonical_to_vec();
    let txs_vec: Vec<SszList<u8, MAX_BYTES_PER_TRANSACTION>> = (0..150)
        .map(|_| tx_bytes.clone().try_into().unwrap())
        .collect();
    let transactions: Transactions = txs_vec.try_into().unwrap();

    let mut group = c.benchmark_group("ssz_tx_decoding_150tx");
    group.sample_size(20);

    // New direct path — decode straight from each SSZ slice.
    group.bench_function("direct", |b| {
        b.iter(|| {
            let txs: Vec<Transaction> = transactions
                .iter()
                .map(|raw| Transaction::decode_canonical(raw).unwrap())
                .collect();
            std::hint::black_box(txs);
        })
    });

    // Old path emulation — per-tx Bytes::copy_from_slice into an intermediate
    // Vec<Bytes> before decoding.
    group.bench_function("via_bytes_copy", |b| {
        b.iter(|| {
            let copied: Vec<Bytes> = transactions
                .iter()
                .map(|raw| Bytes::copy_from_slice(raw))
                .collect();
            let txs: Vec<Transaction> = copied
                .iter()
                .map(|b| Transaction::decode_canonical(b.as_ref()).unwrap())
                .collect();
            std::hint::black_box(txs);
        })
    });
    group.finish();
}

// JSON-RPC newPayload `payload.clone() → into_block` vs the new
// `to_block(&self)` that skips the upfront clone.
fn bench_json_payload_to_block(c: &mut Criterion) {
    use ethrex_common::H256;
    use ethrex_common::types::{
        Block, BlockBody, BlockHeader, EIP1559Transaction, Transaction, Withdrawal,
    };
    use ethrex_rpc::types::payload::ExecutionPayload;

    // `ExecutionPayload` has `pub(crate)` fields, so construct via from_block.
    let sample_tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        data: vec![0xab; 512].into(),
        ..Default::default()
    });
    let body = BlockBody {
        transactions: vec![sample_tx; 150],
        ommers: Vec::new(),
        withdrawals: Some(Vec::<Withdrawal>::new()),
    };
    let header = BlockHeader {
        number: 1,
        gas_limit: 30_000_000,
        timestamp: 1_700_000_000,
        base_fee_per_gas: Some(0),
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        ..Default::default()
    };
    let block = Block::new(header, body);
    let payload = ExecutionPayload::from_block(block, None);

    let mut group = c.benchmark_group("json_payload_to_block_150tx");
    group.sample_size(20);

    // New path — no upfront clone.
    group.bench_function("to_block_ref", |b| {
        b.iter(|| {
            let block = payload.to_block(Some(H256::zero()), None, None).unwrap();
            std::hint::black_box(block);
        })
    });

    // Old path — payload.clone() then into_block(self).
    group.bench_function("clone_then_into_block", |b| {
        b.iter(|| {
            let cloned = payload.clone();
            let block = cloned.to_block(Some(H256::zero()), None, None).unwrap();
            std::hint::black_box(block);
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_codec,
    bench_blob_list,
    bench_blobs_bundle_conversion,
    bench_body_lookups,
    bench_ssz_tx_decoding,
    bench_json_payload_to_block
);
criterion_main!(benches);
