use ethrex_common::{
    Address, H256,
    types::{
        BYTES_PER_BLOB, BYTES_PER_CELL, BlobsBundle, CELLS_PER_EXT_BLOB, ChainConfig,
        kzg_commitment_to_versioned_hash,
    },
};
use ethrex_rpc::{
    engine::{blobs::BlobsV4Request, fork_choice::ForkChoiceUpdatedV4},
    rpc::{RpcApiContext, RpcHandler},
    test_utils::default_context_with_storage,
    utils::RpcErr,
};
use ethrex_storage::{EngineType, Store};
use serde_json::{Value, json};

// ── helpers ───────────────────────────────────────────────────────────────────

fn zero_fcs() -> Value {
    json!({
        "headBlockHash": H256::zero(),
        "safeBlockHash": H256::zero(),
        "finalizedBlockHash": H256::zero(),
    })
}

fn hex_mask(mask: u128) -> String {
    format!("0x{}", hex::encode(mask.to_le_bytes()))
}

async fn fresh_context() -> RpcApiContext {
    let store = Store::new("test", EngineType::InMemory).expect("store");
    default_context_with_storage(store).await
}

// getBlobsV4 (EIP-8070) is an Amsterdam Engine API method, so the serving-path
// tests activate Amsterdam (which implies Osaka, where cell proofs first exist).
async fn amsterdam_context() -> RpcApiContext {
    let mut store = Store::new("test-amsterdam", EngineType::InMemory).expect("store");
    let config = ChainConfig {
        chain_id: 1,
        shanghai_time: Some(0),
        cancun_time: Some(0),
        prague_time: Some(0),
        osaka_time: Some(0),
        amsterdam_time: Some(0),
        deposit_contract_address: Address::zero(),
        ..Default::default()
    };
    store.set_chain_config(&config).await.expect("set config");
    default_context_with_storage(store).await
}

// ── engine_getBlobsV4 ─────────────────────────────────────────────────────────

#[tokio::test]
async fn blobs_v4_pre_amsterdam_returns_null() {
    // amsterdam.md getBlobsV4 §6: while unable to serve (here pre-Amsterdam /
    // syncing) the method MUST return `null` per hash, not a bespoke -38005.
    let ctx = fresh_context().await; // no fork times configured
    let req =
        BlobsV4Request::parse(&Some(vec![json!([H256::zero()]), json!(hex_mask(1u128))])).unwrap();
    let result = req.handle(ctx).await.unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert!(arr[0].is_null(), "pre-Amsterdam must return null entry");
}

#[tokio::test]
async fn blobs_v4_rejects_over_cap() {
    let ctx = amsterdam_context().await;
    let hashes: Vec<H256> = (0..=128).map(H256::from_low_u64_be).collect();
    let req = BlobsV4Request::parse(&Some(vec![
        serde_json::to_value(&hashes).unwrap(),
        json!(hex_mask(1u128)),
    ]))
    .unwrap();
    let err = req.handle(ctx).await.unwrap_err();
    assert!(
        matches!(err, RpcErr::TooLargeRequest),
        "over-cap must return TooLargeRequest, got {err:?}"
    );
}

#[tokio::test]
async fn blobs_v4_unknown_hash_returns_null_entry() {
    let ctx = amsterdam_context().await;
    let req = BlobsV4Request::parse(&Some(vec![
        json!([H256::from_low_u64_be(999)]),
        json!(hex_mask(u128::MAX)),
    ]))
    .unwrap();
    let result = req.handle(ctx).await.unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert!(arr[0].is_null(), "unknown hash must return null entry");
}

#[tokio::test]
async fn blobs_v4_sparse_mask_returns_length_128_matrix() {
    let ctx = amsterdam_context().await;

    // Build a synthetic bundle with 1 blob (version=1 for Osaka).
    let commitment = [0x01u8; 48];
    let cell_proofs: Vec<[u8; 48]> = (0..CELLS_PER_EXT_BLOB).map(|i| [i as u8; 48]).collect();
    let bundle = BlobsBundle {
        blobs: vec![[0u8; BYTES_PER_BLOB]],
        commitments: vec![commitment],
        proofs: cell_proofs,
        version: 1,
    };
    let vh = kzg_commitment_to_versioned_hash(&commitment);
    let tx_hash = H256::from_low_u64_be(1);
    ctx.blockchain
        .mempool
        .add_blobs_bundle(tx_hash, bundle)
        .unwrap();

    // Store a recognizable cell for column 2 only.
    let cell_bytes = Box::new([0xCCu8; BYTES_PER_CELL]);
    ctx.blockchain
        .mempool
        .store_cells(tx_hash, 1, vec![(0, 2, cell_bytes)])
        .unwrap();

    // Request only column 2 (bit 2 → mask = 0b100 = 4).
    let mask: u128 = 1 << 2;
    let req = BlobsV4Request::parse(&Some(vec![
        serde_json::to_value(vec![vh]).unwrap(),
        json!(hex_mask(mask)),
    ]))
    .unwrap();
    let result = req.handle(ctx).await.unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    let entry = &arr[0];
    assert!(!entry.is_null(), "known hash must return non-null entry");
    let blob_cells = entry["blobCells"].as_array().unwrap();
    let proofs = entry["proofs"].as_array().unwrap();
    // Sparse length-128 matrices (EIP-8070 / execution-specs PR #2948): the value
    // sits at requested+held column 2, with null at every other index, for both
    // cells and proofs.
    assert_eq!(blob_cells.len(), CELLS_PER_EXT_BLOB);
    assert_eq!(proofs.len(), CELLS_PER_EXT_BLOB);
    assert!(
        !blob_cells[2].is_null(),
        "requested column 2 cell must not be null"
    );
    assert!(
        !proofs[2].is_null(),
        "requested column 2 proof must not be null"
    );
    let hex = blob_cells[2].as_str().unwrap();
    let decoded = hex::decode(&hex[2..]).unwrap();
    assert!(
        decoded.iter().all(|&b| b == 0xCC),
        "stored cell value preserved"
    );
    for i in 0..CELLS_PER_EXT_BLOB {
        if i == 2 {
            continue;
        }
        assert!(
            blob_cells[i].is_null(),
            "non-requested cell {i} must be null"
        );
        assert!(proofs[i].is_null(), "non-requested proof {i} must be null");
    }
}

#[tokio::test]
async fn blobs_v4_version_zero_bundle_returns_null_entry() {
    let ctx = amsterdam_context().await;
    // version=0 bundle cannot supply per-cell proofs → null entry.
    let commitment = [0x02u8; 48];
    let bundle = BlobsBundle {
        blobs: vec![[0u8; BYTES_PER_BLOB]],
        commitments: vec![commitment],
        proofs: vec![[0u8; 48]], // single KZG proof, not cell proofs
        version: 0,
    };
    let vh = kzg_commitment_to_versioned_hash(&commitment);
    let tx_hash = H256::from_low_u64_be(2);
    ctx.blockchain
        .mempool
        .add_blobs_bundle(tx_hash, bundle)
        .unwrap();

    let req = BlobsV4Request::parse(&Some(vec![
        serde_json::to_value(vec![vh]).unwrap(),
        json!(hex_mask(u128::MAX)),
    ]))
    .unwrap();
    let result = req.handle(ctx).await.unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert!(
        arr[0].is_null(),
        "version-0 bundle must return null entry (no cell proofs)"
    );
}

// ── forkchoiceUpdatedV4 custodyColumns (parse path) ───────────────────────────

#[test]
fn fcu_v4_parse_custody_absent_is_none() {
    // Only 1 param → no custodyColumns.
    let parsed = ForkChoiceUpdatedV4::parse(&Some(vec![zero_fcs()])).unwrap();
    assert_eq!(parsed.custody_columns, None);
}

#[test]
fn fcu_v4_parse_custody_null_is_none() {
    let parsed =
        ForkChoiceUpdatedV4::parse(&Some(vec![zero_fcs(), json!(null), json!(null)])).unwrap();
    assert_eq!(parsed.custody_columns, None);
}

#[test]
fn fcu_v4_parse_custody_valid_16_bytes() {
    let mask: u128 = 0x0000_0000_0000_0000_0000_0000_0000_0001;
    let params = Some(vec![zero_fcs(), json!(null), json!(hex_mask(mask))]);
    let parsed = ForkChoiceUpdatedV4::parse(&params).unwrap();
    assert_eq!(parsed.custody_columns, Some(mask));
}

#[test]
fn fcu_v4_parse_custody_wrong_byte_length_is_bad_params() {
    // 8 bytes instead of 16.
    let params = Some(vec![zero_fcs(), json!(null), json!("0x0000000000000001")]);
    let err = ForkChoiceUpdatedV4::parse(&params).unwrap_err();
    assert!(
        matches!(err, RpcErr::BadParams(_)),
        "wrong byte length must be BadParams, got {err:?}"
    );
}

// ── forkchoiceUpdatedV4 custodyColumns (handle path — mempool effect) ─────────
//
// These tests exercise apply_custody_update through the public handle() path.
// The FCU head is H256::zero() (unknown), so apply_fork_choice returns Syncing,
// and the RPC layer calls apply_custody_update before returning the SYNCING response.

#[tokio::test]
async fn fcu_v4_null_custody_does_not_change_mempool() {
    let ctx = fresh_context().await;
    ctx.blockchain.mempool.set_custody_columns(0xFF).unwrap();

    let req = ForkChoiceUpdatedV4::parse(&Some(vec![
        zero_fcs(),
        json!(null),
        json!(null), // null custody
    ]))
    .unwrap();
    req.handle(ctx.clone()).await.unwrap();

    assert_eq!(
        ctx.blockchain.mempool.get_custody_columns().unwrap(),
        0xFF,
        "null custody must leave mempool unchanged"
    );
}

#[tokio::test]
async fn fcu_v4_identical_custody_is_noop() {
    let ctx = fresh_context().await;
    ctx.blockchain.mempool.set_custody_columns(0b1010).unwrap();
    let gen_before = ctx.blockchain.mempool.custody_generation();

    let req = ForkChoiceUpdatedV4::parse(&Some(vec![
        zero_fcs(),
        json!(null),
        json!(hex_mask(0b1010u128)),
    ]))
    .unwrap();
    req.handle(ctx.clone()).await.unwrap();

    assert_eq!(
        ctx.blockchain.mempool.get_custody_columns().unwrap(),
        0b1010
    );
    assert_eq!(
        ctx.blockchain.mempool.custody_generation(),
        gen_before,
        "identical custody must not bump generation"
    );
}

#[tokio::test]
async fn fcu_v4_expansion_sets_custody_and_bumps_generation() {
    let ctx = fresh_context().await;
    ctx.blockchain.mempool.set_custody_columns(0b0001).unwrap();
    let gen_before = ctx.blockchain.mempool.custody_generation();

    let req = ForkChoiceUpdatedV4::parse(&Some(vec![
        zero_fcs(),
        json!(null),
        json!(hex_mask(0b0011u128)), // add column 1
    ]))
    .unwrap();
    req.handle(ctx.clone()).await.unwrap();

    assert_eq!(
        ctx.blockchain.mempool.get_custody_columns().unwrap(),
        0b0011,
        "custody must expand to 0b0011"
    );
    assert!(
        ctx.blockchain.mempool.custody_generation() > gen_before,
        "generation must bump on expansion"
    );
}

#[tokio::test]
async fn fcu_v4_contraction_sets_reduced_custody() {
    let ctx = fresh_context().await;
    ctx.blockchain.mempool.set_custody_columns(0b1111).unwrap();

    let req = ForkChoiceUpdatedV4::parse(&Some(vec![
        zero_fcs(),
        json!(null),
        json!(hex_mask(0b0011u128)), // remove columns 2 and 3
    ]))
    .unwrap();
    req.handle(ctx.clone()).await.unwrap();

    assert_eq!(
        ctx.blockchain.mempool.get_custody_columns().unwrap(),
        0b0011,
        "custody must contract to 0b0011"
    );
}

// ── engine_getBlobsV4 unit tests (moved from engine/blobs.rs) ─────────────────
//
// These were in-crate unit tests; they drive BlobsV4Request through the public
// `handle()` path. The request is built via the `test_utils::blobs_v4_request`
// shim (its fields are crate-private) and the bitarray-parse tests call the
// `test_utils::parse_indices_bitarray` shim.
use ethrex_common::types::{Commitment, Proof};
use ethrex_rpc::test_utils::{
    GET_BLOBS_V1_REQUEST_MAX_SIZE, blobs_v4_request, parse_indices_bitarray,
};

fn chain_config(active: bool) -> ChainConfig {
    ChainConfig {
        chain_id: 1,
        shanghai_time: Some(0),
        cancun_time: Some(0),
        prague_time: Some(0),
        osaka_time: active.then_some(0),
        amsterdam_time: active.then_some(0),
        deposit_contract_address: Address::zero(),
        ..Default::default()
    }
}

async fn context_with_chain_config(osaka_active: bool) -> RpcApiContext {
    let mut storage =
        Store::new("test-blobs", EngineType::InMemory).expect("Failed to create test store");
    storage
        .set_chain_config(&chain_config(osaka_active))
        .await
        .expect("Failed to set chain config");
    default_context_with_storage(storage).await
}

#[tokio::test]
async fn blobs_v4_accepts_exactly_max_size() {
    // Spec §5: clients MUST support at least MAX hashes, so exactly MAX must not
    // be rejected as too large (regression guard for the `>=` vs `>` off-by-one).
    let context = context_with_chain_config(true).await;
    let request = blobs_v4_request(vec![H256::zero(); GET_BLOBS_V1_REQUEST_MAX_SIZE], u128::MAX);
    let result = request.handle(context).await;
    assert!(!matches!(result, Err(RpcErr::TooLargeRequest)));
}

#[tokio::test]
async fn blobs_v4_parse_wrong_bitarray_length_rejected() {
    let err = parse_indices_bitarray(&serde_json::json!("0xdeadbeef")).unwrap_err();
    assert!(matches!(err, RpcErr::BadParams(_)));
}

#[tokio::test]
async fn blobs_v4_parse_valid_bitarray() {
    let mask: u128 = 0x0000_0000_0000_0000_0000_0000_0000_0001;
    let hex = format!("0x{}", hex::encode(mask.to_le_bytes()));
    let parsed = parse_indices_bitarray(&serde_json::json!(hex)).unwrap();
    assert_eq!(parsed, mask);
}

#[tokio::test]
async fn blobs_v4_response_is_sparse_length_128() {
    // EIP-8070 / execution-specs PR #2948: getBlobsV4 returns length-128
    // matrices with the value at requested indices and null elsewhere.
    let context = context_with_chain_config(true).await;
    let (bundle, hashes) = sample_bundle(1);
    let tx_hash = H256::from_low_u64_be(1);
    context
        .blockchain
        .mempool
        .add_blobs_bundle(tx_hash, bundle)
        .unwrap();

    // Request columns 0 and 5 only; store cells for both.
    for col in [0usize, 5] {
        let cell_bytes = Box::new([0xCDu8; BYTES_PER_CELL]);
        context
            .blockchain
            .mempool
            .store_cells(tx_hash, 1, vec![(0, col, cell_bytes)])
            .unwrap();
    }
    let mask: u128 = (1 << 0) | (1 << 5);
    let request = blobs_v4_request(vec![hashes[0]], mask);
    let result = request.handle(context).await.unwrap();
    let entry = &result.as_array().unwrap()[0];
    let blob_cells = entry["blobCells"].as_array().unwrap();
    let proofs = entry["proofs"].as_array().unwrap();
    assert_eq!(blob_cells.len(), CELLS_PER_EXT_BLOB);
    assert_eq!(proofs.len(), CELLS_PER_EXT_BLOB);
    for i in 0..CELLS_PER_EXT_BLOB {
        let requested = (mask >> i) & 1 == 1;
        assert_eq!(!blob_cells[i].is_null(), requested, "cell {i}");
        assert_eq!(!proofs[i].is_null(), requested, "proof {i}");
    }
}

#[tokio::test]
async fn blobs_v4_missing_stored_and_no_blob_returns_null_cell() {
    let context = context_with_chain_config(true).await;
    // Create a bundle with blobs elided (empty blobs vec) but valid commitments/proofs.
    let commitments: Vec<Commitment> = vec![[0u8; 48]];
    let proofs: Vec<Proof> = vec![[2u8; 48]; CELLS_PER_EXT_BLOB];
    let hashes: Vec<H256> = commitments
        .iter()
        .map(kzg_commitment_to_versioned_hash)
        .collect();
    let bundle = BlobsBundle {
        blobs: vec![], // elided
        commitments,
        proofs,
        version: 1,
    };
    let tx_hash = H256::from_low_u64_be(77);
    context
        .blockchain
        .mempool
        .add_blobs_bundle(tx_hash, bundle)
        .unwrap();

    // No stored cells, blob is elided — expect null cell.
    let request = blobs_v4_request(vec![hashes[0]], 1); // column 0 only
    let result = request.handle(context).await.unwrap();
    let arr = result.as_array().unwrap();
    // The hash resolved, so we get Some(BlobCellsAndProofsV1) with a null cell.
    let entry = &arr[0];
    if !entry.is_null() {
        let blob_cells = entry["blobCells"].as_array().unwrap();
        assert!(blob_cells[0].is_null());
    }
    // else: blob_idx=0 is out of bounds for empty blobs slice → None entry, also acceptable.
}

fn sample_bundle(count: usize) -> (BlobsBundle, Vec<H256>) {
    let blobs = vec![[1u8; BYTES_PER_BLOB]; count];
    let commitments: Vec<Commitment> = (0..count).map(|i| [i as u8; 48]).collect();
    let proofs: Vec<Proof> = vec![[2u8; 48]; count * CELLS_PER_EXT_BLOB];

    let hashes = commitments
        .iter()
        .map(kzg_commitment_to_versioned_hash)
        .collect();

    let bundle = BlobsBundle {
        blobs,
        commitments,
        proofs,
        version: 1,
    };
    (bundle, hashes)
}
