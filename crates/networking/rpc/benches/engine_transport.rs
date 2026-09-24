//! Engine REST/SSZ vs JSON-RPC microbenchmarks — all fork eras
//! (Paris → Amsterdam) and all endpoint versions.
//!
//! Measures pure serde cost (encode/decode) of the engine API wire formats.
//! Transport-level comparison (HTTP, auth, server handling) lives in
//! `tooling/engine_bench`.
//!
//! Notes on coverage:
//! - Prague's newPayload wire shape is identical to Osaka's (`prague::Envelope`
//!   serves both), so only the Osaka group exists for newPayload; Prague gets
//!   its own getPayload group (V4: `BlobsBundleV1` + requests).
//! - blobs v2 and v3 share the hit-path shape; v3 additionally gets an
//!   all-miss group (its zero-padded miss entries are the pathological case).
//! - `/blobs/v4` has no JSON counterpart and production answers 204, so it is
//!   benchmarked SSZ-only for the spec's wire shape.

#![allow(clippy::unwrap_used)]

#[path = "fixtures.rs"]
mod fixtures;

use criterion::{
    BenchmarkId, Criterion, SamplingMode, Throughput, black_box, criterion_group, criterion_main,
};
use fixtures::PayloadEra;
use libssz::{SszDecode, SszEncode};

/// Tx counts for the newPayload scaling sweep (Osaka group only; other eras
/// use the 150-tx point). 150 × 200 B ≈ 30 KB of tx data is a small-to-mid
/// mainnet payload; 500 approximates a busy block.
const NEWPAYLOAD_TX_COUNTS: [usize; 3] = [10, 150, 500];

/// Hex-string → fixed byte array, shaped like the production hex deserializers
/// (allocate the string, strip `0x`, decode). Used by the JSON mirror types
/// below; production uses `hex_simd`, so treat the absolute numbers as an
/// upper bound on JSON decode cost.
fn hex_array<'de, D, const N: usize>(d: D) -> Result<[u8; N], D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    use serde::de::Error;
    let s = String::deserialize(d)?;
    let bytes = hex::decode(s.trim_start_matches("0x")).map_err(D::Error::custom)?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| D::Error::custom(format!("expected {N} bytes, got {}", v.len())))
}

/// Hex-string vector → fixed byte arrays (cell proofs).
fn hex_array_vec<'de, D, const N: usize>(d: D) -> Result<Vec<[u8; N]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    use serde::de::Error;
    let strings = Vec::<String>::deserialize(d)?;
    strings
        .into_iter()
        .map(|s| {
            let bytes = hex::decode(s.trim_start_matches("0x")).map_err(D::Error::custom)?;
            bytes.try_into().map_err(|v: Vec<u8>| {
                D::Error::custom(format!("expected {N} bytes, got {}", v.len()))
            })
        })
        .collect()
}

/// JSON-decode mirrors of the Serialize-only production blob types (the server
/// sends blobs, never parses them — a CL does). They give the blobs groups a
/// JSON decode baseline to compare against `decode_ssz`.
#[derive(serde::Deserialize)]
struct BlobAndProofV1Mirror {
    #[serde(deserialize_with = "hex_array")]
    #[allow(dead_code)]
    blob: [u8; ethrex_rpc::engine_rest::types::blobs::BYTES_PER_BLOB],
    #[serde(deserialize_with = "hex_array")]
    #[allow(dead_code)]
    proof: [u8; ethrex_rpc::engine_rest::types::blobs::BYTES_PER_PROOF],
}

#[derive(serde::Deserialize)]
struct BlobAndProofV2Mirror {
    #[serde(deserialize_with = "hex_array")]
    #[allow(dead_code)]
    blob: [u8; ethrex_rpc::engine_rest::types::blobs::BYTES_PER_BLOB],
    #[serde(deserialize_with = "hex_array_vec")]
    #[allow(dead_code)]
    proofs: Vec<[u8; ethrex_rpc::engine_rest::types::blobs::BYTES_PER_PROOF]>,
}

/// Stamp a 4-way (encode/decode × json/ssz) comparison group. `$jty` is the
/// JSON decode target, `$sty` the SSZ decode target. Optional trailing
/// `cfg: method = value` pairs configure the criterion group (sampling).
macro_rules! transport_group {
    ($c:expr, $name:literal, $json:expr, $jty:ty, $ssz:expr, $sty:ty
     $(, cfg: $cfg:ident = $v:expr)*) => {{
        let json = $json;
        let ssz = $ssz;
        let json_bytes = serde_json::to_vec(&json).unwrap();
        let ssz_bytes = ssz.to_ssz();
        eprintln!(
            "{} wire size — json: {} bytes, ssz: {} bytes (ssz is {:.1}% of json)",
            $name,
            json_bytes.len(),
            ssz_bytes.len(),
            100.0 * ssz_bytes.len() as f64 / json_bytes.len() as f64
        );

        let mut g = $c.benchmark_group($name);
        $( g.$cfg($v); )*
        g.throughput(Throughput::Bytes(json_bytes.len() as u64));
        g.bench_function("encode_json", |b| {
            b.iter(|| black_box(serde_json::to_vec(&json).unwrap()))
        });
        g.throughput(Throughput::Bytes(ssz_bytes.len() as u64));
        g.bench_function("encode_ssz", |b| b.iter(|| black_box(ssz.to_ssz())));
        g.throughput(Throughput::Bytes(json_bytes.len() as u64));
        g.bench_function("decode_json", |b| {
            b.iter(|| {
                let p: $jty = serde_json::from_slice(&json_bytes).unwrap();
                black_box(p);
            })
        });
        g.throughput(Throughput::Bytes(ssz_bytes.len() as u64));
        g.bench_function("decode_ssz", |b| {
            b.iter(|| {
                let p = <$sty>::from_ssz_bytes(&ssz_bytes).unwrap();
                black_box(p);
            })
        });
        g.finish();
    }};
}

/// Like [`transport_group!`] plus `decode_json_via_value`, mirroring the
/// production newPayload parse path (body → `Value` → `params[i].clone()` →
/// `from_value`; see `RpcHandler::parse` in engine/payload.rs), which is
/// costlier than the direct `from_slice`.
macro_rules! newpayload_group {
    ($c:expr, $name:literal, $json:expr, $ssz:expr, $sty:ty) => {{
        let json = $json;
        let ssz = $ssz;
        let json_bytes = serde_json::to_vec(&json).unwrap();
        let ssz_bytes = ssz.to_ssz();
        eprintln!(
            "{} wire size — json: {} bytes, ssz: {} bytes (ssz is {:.1}% of json)",
            $name,
            json_bytes.len(),
            ssz_bytes.len(),
            100.0 * ssz_bytes.len() as f64 / json_bytes.len() as f64
        );

        let mut g = $c.benchmark_group($name);
        g.throughput(Throughput::Bytes(json_bytes.len() as u64));
        g.bench_function("encode_json", |b| {
            b.iter(|| black_box(serde_json::to_vec(&json).unwrap()))
        });
        g.throughput(Throughput::Bytes(ssz_bytes.len() as u64));
        g.bench_function("encode_ssz", |b| b.iter(|| black_box(ssz.to_ssz())));
        g.throughput(Throughput::Bytes(json_bytes.len() as u64));
        g.bench_function("decode_json", |b| {
            b.iter(|| {
                let p: ethrex_rpc::types::payload::ExecutionPayload =
                    serde_json::from_slice(&json_bytes).unwrap();
                black_box(p);
            })
        });
        g.bench_function("decode_json_via_value", |b| {
            b.iter(|| {
                let v: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();
                // The clone is intentional: production clones the param Value
                // out of the request before from_value.
                #[allow(clippy::redundant_clone)]
                let p: ethrex_rpc::types::payload::ExecutionPayload =
                    serde_json::from_value(v.clone()).unwrap();
                black_box(p);
            })
        });
        g.throughput(Throughput::Bytes(ssz_bytes.len() as u64));
        g.bench_function("decode_ssz", |b| {
            b.iter(|| {
                let p = <$sty>::from_ssz_bytes(&ssz_bytes).unwrap();
                black_box(p);
            })
        });
        g.finish();
    }};
}

// ── newPayload ────────────────────────────────────────────────────────────────

fn newpayload_paris_bench(c: &mut Criterion) {
    newpayload_group!(
        c,
        "newPayload_paris",
        fixtures::payload_json(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            PayloadEra::Paris
        ),
        fixtures::paris_newpayload_ssz(fixtures::DEFAULT_SEED, fixtures::DEFAULT_TX_COUNT),
        ethrex_rpc::engine_rest::types::paris::ExecutionPayloadEnvelope
    );
}

fn newpayload_shanghai_bench(c: &mut Criterion) {
    newpayload_group!(
        c,
        "newPayload_shanghai",
        fixtures::payload_json(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            PayloadEra::Shanghai
        ),
        fixtures::shanghai_newpayload_ssz(fixtures::DEFAULT_SEED, fixtures::DEFAULT_TX_COUNT),
        ethrex_rpc::engine_rest::types::shanghai::ExecutionPayloadEnvelope
    );
}

fn newpayload_cancun_bench(c: &mut Criterion) {
    newpayload_group!(
        c,
        "newPayload_cancun",
        fixtures::payload_json(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            PayloadEra::Cancun
        ),
        fixtures::cancun_newpayload_ssz(fixtures::DEFAULT_SEED, fixtures::DEFAULT_TX_COUNT),
        ethrex_rpc::engine_rest::types::cancun::ExecutionPayloadEnvelope
    );
}

/// Osaka newPayload, swept over tx counts. The wire shape (`prague::Envelope`)
/// also serves Prague, so this group covers both eras.
fn newpayload_osaka_bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("newPayload_osaka");
    for tx_count in NEWPAYLOAD_TX_COUNTS {
        let json = fixtures::payload_json(fixtures::DEFAULT_SEED, tx_count, PayloadEra::Cancun);
        let ssz = fixtures::prague_newpayload_ssz(fixtures::DEFAULT_SEED, tx_count);

        let json_bytes = serde_json::to_vec(&json).unwrap();
        let ssz_bytes = ssz.to_ssz();
        eprintln!(
            "newPayload_osaka[{} txs] wire size — json: {} bytes, ssz: {} bytes (ssz is {:.1}% of json)",
            tx_count,
            json_bytes.len(),
            ssz_bytes.len(),
            100.0 * ssz_bytes.len() as f64 / json_bytes.len() as f64
        );

        g.throughput(Throughput::Bytes(json_bytes.len() as u64));
        g.bench_with_input(BenchmarkId::new("encode_json", tx_count), &json, |b, p| {
            b.iter(|| black_box(serde_json::to_vec(p).unwrap()))
        });
        g.throughput(Throughput::Bytes(ssz_bytes.len() as u64));
        g.bench_with_input(BenchmarkId::new("encode_ssz", tx_count), &ssz, |b, p| {
            b.iter(|| black_box(p.to_ssz()))
        });

        g.throughput(Throughput::Bytes(json_bytes.len() as u64));
        g.bench_function(BenchmarkId::new("decode_json", tx_count), |b| {
            b.iter(|| {
                let p: ethrex_rpc::types::payload::ExecutionPayload =
                    serde_json::from_slice(&json_bytes).unwrap();
                black_box(p);
            })
        });
        g.bench_function(BenchmarkId::new("decode_json_via_value", tx_count), |b| {
            b.iter(|| {
                let v: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();
                #[allow(clippy::redundant_clone)]
                let p: ethrex_rpc::types::payload::ExecutionPayload =
                    serde_json::from_value(v.clone()).unwrap();
                black_box(p);
            })
        });
        g.throughput(Throughput::Bytes(ssz_bytes.len() as u64));
        g.bench_function(BenchmarkId::new("decode_ssz", tx_count), |b| {
            b.iter(|| {
                let p =
                    ethrex_rpc::engine_rest::types::prague::ExecutionPayloadEnvelope::from_ssz_bytes(
                        &ssz_bytes,
                    )
                    .unwrap();
                black_box(p);
            })
        });
    }
    g.finish();
}

/// Amsterdam adds the block access list (EIP-7928): RLP hex inside JSON, the
/// same RLP bytes as an SSZ byte list. The BAL is the dominant new wire
/// element, so this group fixes tx_count and uses the default BAL.
fn newpayload_amsterdam_bench(c: &mut Criterion) {
    newpayload_group!(
        c,
        "newPayload_amsterdam",
        fixtures::amsterdam_payload_json(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            fixtures::DEFAULT_BAL_ACCOUNTS
        ),
        fixtures::amsterdam_newpayload_ssz(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            fixtures::DEFAULT_BAL_ACCOUNTS
        ),
        ethrex_rpc::engine_rest::types::amsterdam::ExecutionPayloadEnvelope
    );
}

// ── getPayload ────────────────────────────────────────────────────────────────

fn getpayload_paris_bench(c: &mut Criterion) {
    transport_group!(
        c,
        "getPayload_paris",
        fixtures::getpayload_response_json_paris(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT
        ),
        ethrex_rpc::types::payload::ExecutionPayload,
        fixtures::getpayload_response_ssz_paris(fixtures::DEFAULT_SEED, fixtures::DEFAULT_TX_COUNT),
        ethrex_rpc::engine_rest::types::built_payload::BuiltPayloadParis
    );
}

fn getpayload_shanghai_bench(c: &mut Criterion) {
    transport_group!(
        c,
        "getPayload_shanghai",
        fixtures::getpayload_response_json_shanghai(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT
        ),
        ethrex_rpc::types::payload::ExecutionPayloadResponse,
        fixtures::getpayload_response_ssz_shanghai(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT
        ),
        ethrex_rpc::engine_rest::types::built_payload::BuiltPayloadShanghai
    );
}

fn getpayload_cancun_bench(c: &mut Criterion) {
    transport_group!(
        c,
        "getPayload_cancun",
        fixtures::getpayload_response_json_cancun(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            fixtures::DEFAULT_BUNDLE_BLOB_COUNT
        ),
        ethrex_rpc::types::payload::ExecutionPayloadResponse,
        fixtures::getpayload_response_ssz_cancun(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            fixtures::DEFAULT_BUNDLE_BLOB_COUNT
        ),
        ethrex_rpc::engine_rest::types::built_payload::BuiltPayloadCancun,
        cfg: sampling_mode = SamplingMode::Flat,
        cfg: sample_size = 30
    );
}

fn getpayload_prague_bench(c: &mut Criterion) {
    transport_group!(
        c,
        "getPayload_prague",
        fixtures::getpayload_response_json_prague(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            fixtures::DEFAULT_BUNDLE_BLOB_COUNT
        ),
        ethrex_rpc::types::payload::ExecutionPayloadResponse,
        fixtures::getpayload_response_ssz_prague(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            fixtures::DEFAULT_BUNDLE_BLOB_COUNT
        ),
        ethrex_rpc::engine_rest::types::built_payload::BuiltPayloadPrague,
        cfg: sampling_mode = SamplingMode::Flat,
        cfg: sample_size = 30
    );
}

fn getpayload_osaka_bench(c: &mut Criterion) {
    transport_group!(
        c,
        "getPayload_osaka",
        fixtures::getpayload_response_json_osaka(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            fixtures::DEFAULT_BUNDLE_BLOB_COUNT
        ),
        ethrex_rpc::types::payload::ExecutionPayloadResponse,
        fixtures::getpayload_response_ssz_osaka(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            fixtures::DEFAULT_BUNDLE_BLOB_COUNT
        ),
        ethrex_rpc::engine_rest::types::built_payload::BuiltPayloadOsaka,
        cfg: sampling_mode = SamplingMode::Flat,
        cfg: sample_size = 30
    );
}

fn getpayload_amsterdam_bench(c: &mut Criterion) {
    transport_group!(
        c,
        "getPayload_amsterdam",
        fixtures::getpayload_response_json_amsterdam(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            fixtures::DEFAULT_BUNDLE_BLOB_COUNT,
            fixtures::DEFAULT_BAL_ACCOUNTS
        ),
        ethrex_rpc::types::payload::ExecutionPayloadResponse,
        fixtures::getpayload_response_ssz_amsterdam(
            fixtures::DEFAULT_SEED,
            fixtures::DEFAULT_TX_COUNT,
            fixtures::DEFAULT_BUNDLE_BLOB_COUNT,
            fixtures::DEFAULT_BAL_ACCOUNTS
        ),
        ethrex_rpc::engine_rest::types::built_payload::BuiltPayloadAmsterdam,
        cfg: sampling_mode = SamplingMode::Flat,
        cfg: sample_size = 30
    );
}

// ── blobs ─────────────────────────────────────────────────────────────────────

fn blobs_v1_bench(c: &mut Criterion) {
    transport_group!(
        c,
        "blobs_v1",
        fixtures::blobs_v1_response_json(fixtures::DEFAULT_SEED, fixtures::DEFAULT_BLOB_REQUEST_COUNT),
        Vec<Option<BlobAndProofV1Mirror>>,
        fixtures::blobs_v1_response_ssz(fixtures::DEFAULT_SEED, fixtures::DEFAULT_BLOB_REQUEST_COUNT),
        ethrex_rpc::engine_rest::types::blobs::BlobsV1Response,
        cfg: sampling_mode = SamplingMode::Flat,
        cfg: sample_size = 10
    );
}

/// v1 all-miss: like v3, missed entries are zero-padded to full blob size.
fn blobs_v1_miss_bench(c: &mut Criterion) {
    transport_group!(
        c,
        "blobs_v1_miss",
        fixtures::blobs_v1_response_json_allmiss(fixtures::DEFAULT_BLOB_REQUEST_COUNT),
        Vec<Option<BlobAndProofV1Mirror>>,
        fixtures::blobs_v1_response_ssz_allmiss(fixtures::DEFAULT_BLOB_REQUEST_COUNT),
        ethrex_rpc::engine_rest::types::blobs::BlobsV1Response,
        cfg: sampling_mode = SamplingMode::Flat,
        cfg: sample_size = 10
    );
}

/// Hit path for v2 AND v3 (identical shape: blob + 128 cell proofs).
fn blobs_v2_bench(c: &mut Criterion) {
    transport_group!(
        c,
        "blobs_v2",
        fixtures::blobs_v2_response_json(fixtures::DEFAULT_SEED, fixtures::DEFAULT_BLOB_REQUEST_COUNT),
        Vec<Option<BlobAndProofV2Mirror>>,
        fixtures::blobs_v2_response_ssz(fixtures::DEFAULT_SEED, fixtures::DEFAULT_BLOB_REQUEST_COUNT),
        ethrex_rpc::engine_rest::types::blobs::BlobsV2Response,
        cfg: sampling_mode = SamplingMode::Flat,
        cfg: sample_size = 10
    );
}

/// All-miss path. JSON answers `n` nulls; SSZ v3 answers `n` zero-padded
/// full-size entries — the miss-path pathology. (v2's all-miss is
/// `null`/`204 No Content` and costs nothing on either transport.)
fn blobs_v3_miss_bench(c: &mut Criterion) {
    transport_group!(
        c,
        "blobs_v3_miss",
        fixtures::blobs_v3_response_json_allmiss(fixtures::DEFAULT_BLOB_REQUEST_COUNT),
        Vec<Option<BlobAndProofV2Mirror>>,
        fixtures::blobs_v3_response_ssz_allmiss(fixtures::DEFAULT_BLOB_REQUEST_COUNT),
        ethrex_rpc::engine_rest::types::blobs::BlobsV3Response,
        cfg: sampling_mode = SamplingMode::Flat,
        cfg: sample_size = 10
    );
}

/// `/blobs/v4` is REST-only (no JSON method; production answers 204 today), so
/// only the SSZ encode/decode of the spec's wire shape is measured.
fn blobs_v4_ssz_bench(c: &mut Criterion) {
    let ssz = fixtures::blobs_v4_response_ssz(
        fixtures::DEFAULT_SEED,
        fixtures::DEFAULT_BLOB_REQUEST_COUNT,
    );
    let ssz_bytes = ssz.to_ssz();
    eprintln!(
        "blobs_v4 wire size — ssz: {} bytes (no JSON counterpart)",
        ssz_bytes.len()
    );

    let mut g = c.benchmark_group("blobs_v4_ssz_only");
    g.sampling_mode(SamplingMode::Flat);
    g.sample_size(10);
    g.throughput(Throughput::Bytes(ssz_bytes.len() as u64));
    g.bench_function("encode_ssz", |b| b.iter(|| black_box(ssz.to_ssz())));
    g.bench_function("decode_ssz", |b| {
        b.iter(|| {
            let r =
                ethrex_rpc::engine_rest::types::blobs::BlobsV4Response::from_ssz_bytes(&ssz_bytes)
                    .unwrap();
            black_box(r);
        })
    });
    g.finish();
}

// ── bodies ────────────────────────────────────────────────────────────────────

fn bodies_paris_bench(c: &mut Criterion) {
    let n = fixtures::DEFAULT_BODIES_COUNT as usize;
    transport_group!(
        c,
        "bodies_paris",
        fixtures::bodies_range_json(fixtures::DEFAULT_SEED, n, 5, false),
        Vec<Option<ethrex_rpc::types::payload::ExecutionPayloadBody>>,
        fixtures::bodies_range_ssz_paris(fixtures::DEFAULT_SEED, n, 5),
        ethrex_rpc::engine_rest::types::bodies::BodiesResponseParis
    );
}

/// The Shanghai body shape serves the shanghai → osaka REST paths and the
/// JSON `…BodiesByRangeV1` method.
fn bodies_shanghai_bench(c: &mut Criterion) {
    let n = fixtures::DEFAULT_BODIES_COUNT as usize;
    transport_group!(
        c,
        "bodies_shanghai",
        fixtures::bodies_range_json(fixtures::DEFAULT_SEED, n, 5, true),
        Vec<Option<ethrex_rpc::types::payload::ExecutionPayloadBody>>,
        fixtures::bodies_range_ssz_shanghai(fixtures::DEFAULT_SEED, n, 5),
        ethrex_rpc::engine_rest::types::bodies::BodiesResponseShanghai
    );
}

fn bodies_amsterdam_bench(c: &mut Criterion) {
    let n = fixtures::DEFAULT_BODIES_COUNT as usize;
    transport_group!(
        c,
        "bodies_amsterdam",
        fixtures::bodies_range_json_amsterdam(
            fixtures::DEFAULT_SEED,
            n,
            5,
            fixtures::DEFAULT_BODY_BAL_ACCOUNTS
        ),
        Vec<Option<ethrex_rpc::types::payload::ExecutionPayloadBodyV2>>,
        fixtures::bodies_range_ssz_amsterdam(
            fixtures::DEFAULT_SEED,
            n,
            5,
            fixtures::DEFAULT_BODY_BAL_ACCOUNTS
        ),
        ethrex_rpc::engine_rest::types::bodies::BodiesResponseAmsterdam
    );
}

criterion_group!(
    benches,
    newpayload_paris_bench,
    newpayload_shanghai_bench,
    newpayload_cancun_bench,
    newpayload_osaka_bench,
    newpayload_amsterdam_bench,
    getpayload_paris_bench,
    getpayload_shanghai_bench,
    getpayload_cancun_bench,
    getpayload_prague_bench,
    getpayload_osaka_bench,
    getpayload_amsterdam_bench,
    blobs_v1_bench,
    blobs_v1_miss_bench,
    blobs_v2_bench,
    blobs_v3_miss_bench,
    blobs_v4_ssz_bench,
    bodies_paris_bench,
    bodies_shanghai_bench,
    bodies_amsterdam_bench,
);
criterion_main!(benches);
