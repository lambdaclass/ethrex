//! Engine REST/SSZ vs JSON-RPC microbenchmarks.

#[path = "fixtures.rs"]
mod fixtures;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use libssz::{SszDecode, SszEncode};

fn newpayload_bench(c: &mut Criterion) {
    let (json, ssz) = fixtures::cancun_payload_pair(
        fixtures::DEFAULT_SEED,
        fixtures::DEFAULT_TX_COUNT,
        fixtures::DEFAULT_BLOB_HASH_COUNT,
    );

    let json_bytes = serde_json::to_vec(&json).unwrap();
    let ssz_bytes = ssz.to_ssz();
    eprintln!(
        "newPayload wire size — json: {} bytes, ssz: {} bytes (ssz is {:.1}% of json)",
        json_bytes.len(),
        ssz_bytes.len(),
        100.0 * ssz_bytes.len() as f64 / json_bytes.len() as f64
    );

    let mut g = c.benchmark_group("newPayload");
    g.bench_function("encode_json", |b| {
        b.iter(|| black_box(serde_json::to_vec(&json).unwrap()))
    });
    g.bench_function("encode_ssz", |b| b.iter(|| black_box(ssz.to_ssz())));
    g.bench_function("decode_json", |b| {
        b.iter(|| {
            let p: ethrex_rpc::types::payload::ExecutionPayload =
                serde_json::from_slice(&json_bytes).unwrap();
            black_box(p);
        })
    });
    g.bench_function("decode_ssz", |b| {
        b.iter(|| {
            let p =
                ethrex_rpc::engine_rest::types::cancun::ExecutionPayloadEnvelope::from_ssz_bytes(
                    &ssz_bytes,
                )
                .unwrap();
            black_box(p);
        })
    });
    g.finish();
}

fn getpayload_bench(c: &mut Criterion) {
    let json = fixtures::getpayload_response_json(fixtures::DEFAULT_SEED);
    let ssz = fixtures::getpayload_response_ssz(fixtures::DEFAULT_SEED);

    let json_bytes = serde_json::to_vec(&json).unwrap();
    let ssz_bytes = ssz.to_ssz();
    eprintln!(
        "getPayload wire size — json: {} bytes, ssz: {} bytes",
        json_bytes.len(),
        ssz_bytes.len()
    );

    let mut g = c.benchmark_group("getPayload");
    g.bench_function("encode_json", |b| {
        b.iter(|| black_box(serde_json::to_vec(&json).unwrap()))
    });
    g.bench_function("encode_ssz", |b| b.iter(|| black_box(ssz.to_ssz())));
    g.bench_function("decode_json", |b| {
        b.iter(|| {
            let p: ethrex_rpc::types::payload::ExecutionPayload =
                serde_json::from_slice(&json_bytes).unwrap();
            black_box(p);
        })
    });
    g.bench_function("decode_ssz", |b| {
        b.iter(|| {
            let p =
                ethrex_rpc::engine_rest::types::cancun::ExecutionPayloadEnvelope::from_ssz_bytes(
                    &ssz_bytes,
                )
                .unwrap();
            black_box(p);
        })
    });
    g.finish();
}

fn blobs_bench(c: &mut Criterion) {
    let json = fixtures::blobs_v1_response_json(
        fixtures::DEFAULT_SEED,
        fixtures::DEFAULT_BLOB_REQUEST_COUNT,
    );
    let ssz = fixtures::blobs_v1_response_ssz(
        fixtures::DEFAULT_SEED,
        fixtures::DEFAULT_BLOB_REQUEST_COUNT,
    );

    let json_bytes = serde_json::to_vec(&json).unwrap();
    let ssz_bytes = ssz.to_ssz();
    eprintln!(
        "blobs wire size — json: {} bytes, ssz: {} bytes",
        json_bytes.len(),
        ssz_bytes.len()
    );

    let mut g = c.benchmark_group("blobs");
    g.bench_function("encode_json", |b| {
        b.iter(|| black_box(serde_json::to_vec(&json).unwrap()))
    });
    g.bench_function("encode_ssz", |b| b.iter(|| black_box(ssz.to_ssz())));
    // NOTE: ethrex_rpc::engine::blobs::BlobAndProofV1 only derives Serialize,
    // not Deserialize (the engine endpoint only sends blobs, never receives them).
    // decode_json is therefore omitted for this group.
    g.bench_function("decode_ssz", |b| {
        b.iter(|| {
            let r =
                ethrex_rpc::engine_rest::types::blobs::BlobsResponseV1::from_ssz_bytes(&ssz_bytes)
                    .unwrap();
            black_box(r);
        })
    });
    g.finish();
}

fn bodies_bench(c: &mut Criterion) {
    let n = fixtures::DEFAULT_BODIES_COUNT as usize;
    let json = fixtures::bodies_range_json(fixtures::DEFAULT_SEED, n, 5);
    let ssz = fixtures::bodies_range_ssz(fixtures::DEFAULT_SEED, n, 5);

    let json_bytes = serde_json::to_vec(&json).unwrap();
    let ssz_bytes = ssz.to_ssz();
    eprintln!(
        "bodies wire size — json: {} bytes, ssz: {} bytes",
        json_bytes.len(),
        ssz_bytes.len()
    );

    let mut g = c.benchmark_group("bodies");
    g.bench_function("encode_json", |b| {
        b.iter(|| black_box(serde_json::to_vec(&json).unwrap()))
    });
    g.bench_function("encode_ssz", |b| b.iter(|| black_box(ssz.to_ssz())));
    g.bench_function("decode_json", |b| {
        b.iter(|| {
            let r: Vec<Option<ethrex_rpc::types::payload::ExecutionPayloadBody>> =
                serde_json::from_slice(&json_bytes).unwrap();
            black_box(r);
        })
    });
    g.bench_function("decode_ssz", |b| {
        b.iter(|| {
            let r = ethrex_rpc::engine_rest::types::bodies::BodiesByHashResponseShanghai::from_ssz_bytes(
                &ssz_bytes,
            )
            .unwrap();
            black_box(r);
        })
    });
    g.finish();
}

criterion_group!(
    benches,
    newpayload_bench,
    getpayload_bench,
    blobs_bench,
    bodies_bench
);
criterion_main!(benches);
