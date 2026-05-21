//! Integration tests for the engine REST/SSZ surface.
//!
//! These exercise the router end-to-end via `tower::ServiceExt::oneshot`, with
//! a JWT-authenticated request body containing real SSZ-encoded payloads.

#![allow(clippy::unwrap_used)]

use axum::body::{Body, to_bytes};
use axum::http::{HeaderValue, Request, StatusCode, header};
use bytes::Bytes;
use ethrex_storage::{EngineType, Store};
use jsonwebtoken::{EncodingKey, Header, encode};
use libssz::{SszDecode, SszEncode};
use libssz_types::SszList;
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use tower::ServiceExt;

use ethrex_common::Address;
use ethrex_common::types::ChainConfig;
use ethrex_rpc::engine_rest::SSZ_REST_CAPABILITIES;
use ethrex_rpc::engine_rest::types::blobs::{GetBlobsV1Request, GetBlobsV1Response};
use ethrex_rpc::engine_rest::types::bodies::{
    GetPayloadBodiesByHashV1Request, PayloadBodiesV1Response,
};
use ethrex_rpc::engine_rest::types::capabilities::{
    ExchangeCapabilitiesRequest, ExchangeCapabilitiesResponse,
};
use ethrex_rpc::engine_rest::types::client_version::{
    ClientVersionV1, GetClientVersionV1Request, GetClientVersionV1Response,
};
use ethrex_rpc::engine_rest::types::common::{
    Bytes32, ForkchoiceStateV1, ForkchoiceUpdatedResponseV1, MAX_CAPABILITY_NAME_LENGTH,
    PayloadStatusV1,
};
use ethrex_rpc::engine_rest::types::execution_payload::{ExecutionPayloadV1, ExecutionPayloadV2};
use ethrex_rpc::engine_rest::types::forkchoice::ForkchoiceUpdatedV1Request;
use ethrex_rpc::engine_rest::types::new_payload::{NewPayloadV1Request, NewPayloadV2Request};
use ethrex_rpc::engine_rest::types::withdrawal::WithdrawalV1;
use ethrex_rpc::rpc::{ClientVersion, NodeData, RpcApiContext};
use ethrex_rpc::test_utils::{
    default_context_with_storage, example_local_node_record, example_p2p_node,
};

const TEST_SECRET: &[u8] = b"test-secret-keytest-secret-keyaa";

fn make_jwt() -> String {
    #[derive(Serialize)]
    struct Claims {
        iat: usize,
    }
    let iat = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize;
    encode(
        &Header::default(),
        &Claims { iat },
        &EncodingKey::from_secret(TEST_SECRET),
    )
    .unwrap()
}

async fn make_router() -> axum::Router {
    let storage = Store::new("", EngineType::InMemory).unwrap();
    make_router_with(storage).await
}

async fn make_router_with(storage: Store) -> axum::Router {
    let mut ctx: RpcApiContext = default_context_with_storage(storage).await;
    ctx.node_data = NodeData {
        jwt_secret: Bytes::copy_from_slice(TEST_SECRET),
        local_p2p_node: example_p2p_node(),
        local_node_record: example_local_node_record(),
        client_version: ClientVersion::new(
            "ethrex".to_string(),
            "0.1.0".to_string(),
            "test".to_string(),
            "abcd1234".to_string(),
            "x86_64-unknown-linux".to_string(),
            "1.70.0".to_string(),
        ),
        extra_data: Bytes::new(),
    };
    ethrex_rpc::engine_rest::router(ctx)
}

async fn make_router_with_chain_config(cc: ChainConfig) -> axum::Router {
    let mut storage = Store::new("", EngineType::InMemory).unwrap();
    storage.set_chain_config(&cc).await.unwrap();
    make_router_with(storage).await
}

fn auth_get(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .header(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", make_jwt())).unwrap(),
        )
        .body(Body::empty())
        .unwrap()
}

fn ssz_body<T: SszEncode>(v: &T) -> Vec<u8> {
    let mut buf = Vec::with_capacity(v.encoded_len());
    v.ssz_append(&mut buf);
    buf
}

fn auth_post(path: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", make_jwt())).unwrap(),
        )
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn rest_endpoint_requires_jwt() {
    let app = make_router().await;
    let req = Request::builder()
        .method("POST")
        .uri("/engine/v1/capabilities")
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn capabilities_round_trip() {
    let app = make_router().await;
    // Build an empty CL capabilities request (we don't filter by it).
    let req_body = ExchangeCapabilitiesRequest {
        capabilities: Vec::<SszList<u8, MAX_CAPABILITY_NAME_LENGTH>>::new()
            .try_into()
            .unwrap(),
    };
    let bytes = ssz_body(&req_body);
    let resp = app
        .oneshot(auth_post("/engine/v1/capabilities", bytes))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let decoded = ExchangeCapabilitiesResponse::from_ssz_bytes(&body).unwrap();
    let strings: Vec<String> = decoded
        .capabilities
        .iter()
        .map(|c| String::from_utf8_lossy(c).to_string())
        .collect();
    // Should include both JSON-RPC method names and SSZ REST endpoints.
    assert!(strings.iter().any(|s| s == "engine_newPayloadV1"));
    for cap in SSZ_REST_CAPABILITIES {
        assert!(strings.iter().any(|s| s == cap), "missing capability {cap}");
    }
}

#[tokio::test]
async fn client_version_round_trip() {
    let app = make_router().await;
    let cl_version = ClientVersionV1 {
        code: b"CL".to_vec().try_into().unwrap(),
        name: b"lighthouse".to_vec().try_into().unwrap(),
        version: b"v5.0.0".to_vec().try_into().unwrap(),
        commit: [1, 2, 3, 4],
    };
    let req = GetClientVersionV1Request {
        client_version: cl_version,
    };
    let bytes = ssz_body(&req);
    let resp = app
        .oneshot(auth_post("/engine/v1/client/version", bytes))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let decoded = GetClientVersionV1Response::from_ssz_bytes(&body).unwrap();
    assert_eq!(decoded.versions.len(), 1);
    let v0 = &decoded.versions[0];
    assert_eq!(&v0.code[..], b"EX");
    assert_eq!(&v0.name[..], b"ethrex");
    assert_eq!(&v0.version[..], b"v0.1.0");
    assert_eq!(v0.commit, [0xab, 0xcd, 0x12, 0x34]);
}

#[tokio::test]
async fn bodies_by_hash_unknown_returns_empty_slots() {
    let app = make_router().await;
    let hashes: Vec<Bytes32> = vec![[0xaa; 32], [0xbb; 32]];
    let req = GetPayloadBodiesByHashV1Request {
        block_hashes: hashes.try_into().unwrap(),
    };
    let resp = app
        .oneshot(auth_post(
            "/engine/v1/payloads/bodies/by-hash",
            ssz_body(&req),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let decoded = PayloadBodiesV1Response::from_ssz_bytes(&body).unwrap();
    assert_eq!(decoded.payload_bodies.len(), 2);
    for slot in decoded.payload_bodies.iter() {
        assert_eq!(slot.len(), 0, "unknown blocks must produce empty slot");
    }
}

#[tokio::test]
async fn blobs_v1_empty_mempool_returns_empty_list() {
    let app = make_router().await;
    let hashes: Vec<Bytes32> = vec![[0xcc; 32]];
    let req = GetBlobsV1Request {
        blob_versioned_hashes: hashes.try_into().unwrap(),
    };
    let resp = app
        .oneshot(auth_post("/engine/v1/blobs", ssz_body(&req)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let decoded = GetBlobsV1Response::from_ssz_bytes(&body).unwrap();
    assert!(decoded.blobs_and_proofs.is_empty());
}

#[tokio::test]
async fn missing_content_type_rejects() {
    let app = make_router().await;
    let req = Request::builder()
        .method("POST")
        .uri("/engine/v1/blobs")
        .header(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", make_jwt())).unwrap(),
        )
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn payload_status_v1_roundtrip() {
    let s = PayloadStatusV1 {
        status: 0,
        latest_valid_hash: vec![[7u8; 32]].try_into().unwrap(),
        validation_error: Vec::new().try_into().unwrap(),
    };
    let bytes = ssz_body(&s);
    let decoded = PayloadStatusV1::from_ssz_bytes(&bytes).unwrap();
    assert_eq!(decoded, s);
}

#[test]
fn execution_payload_v1_roundtrip() {
    let p = ExecutionPayloadV1 {
        parent_hash: [1u8; 32],
        fee_recipient: [2u8; 20],
        state_root: [3u8; 32],
        receipts_root: [4u8; 32],
        logs_bloom: [5u8; 256],
        prev_randao: [6u8; 32],
        block_number: 42,
        gas_limit: 30_000_000,
        gas_used: 21_000,
        timestamp: 1_700_000_000,
        extra_data: vec![0xde, 0xad].try_into().unwrap(),
        base_fee_per_gas: {
            let mut a = [0u8; 32];
            a[0] = 0xff;
            a
        },
        block_hash: [7u8; 32],
        transactions: Vec::<
            SszList<u8, { ethrex_rpc::engine_rest::types::common::MAX_BYTES_PER_TRANSACTION }>,
        >::new()
        .try_into()
        .unwrap(),
    };
    let bytes = ssz_body(&p);
    let decoded = ExecutionPayloadV1::from_ssz_bytes(&bytes).unwrap();
    assert_eq!(decoded, p);
}

#[test]
fn forkchoice_response_nullable_id_none() {
    let r = ForkchoiceUpdatedResponseV1 {
        payload_status: PayloadStatusV1 {
            status: 2, // SYNCING
            latest_valid_hash: Vec::new().try_into().unwrap(),
            validation_error: Vec::new().try_into().unwrap(),
        },
        payload_id: Vec::new().try_into().unwrap(),
    };
    let bytes = ssz_body(&r);
    let decoded = ForkchoiceUpdatedResponseV1::from_ssz_bytes(&bytes).unwrap();
    assert_eq!(decoded, r);
    assert!(decoded.payload_id.is_empty());
}

#[test]
fn forkchoice_response_nullable_id_some() {
    let r = ForkchoiceUpdatedResponseV1 {
        payload_status: PayloadStatusV1 {
            status: 0,
            latest_valid_hash: vec![[9u8; 32]].try_into().unwrap(),
            validation_error: Vec::new().try_into().unwrap(),
        },
        payload_id: vec![[0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0]]
            .try_into()
            .unwrap(),
    };
    let bytes = ssz_body(&r);
    let decoded = ForkchoiceUpdatedResponseV1::from_ssz_bytes(&bytes).unwrap();
    assert_eq!(decoded, r);
    assert_eq!(decoded.payload_id.len(), 1);
}

#[tokio::test]
async fn forkchoice_v1_no_attrs_returns_status() {
    // All-zero forkchoice state against the default-genesis store: the head
    // hash doesn't resolve to a known block, apply_fork_choice falls through
    // to the catch-all branch which reports INVALID against the canonical tip.
    // Result: 200 OK with PayloadStatus=INVALID (1) and no payload_id.
    let app = make_router().await;
    let req = ForkchoiceUpdatedV1Request {
        forkchoice_state: ForkchoiceStateV1::default(),
        payload_attributes: Vec::new().try_into().unwrap(),
    };
    let resp = app
        .oneshot(auth_post("/engine/v1/forkchoice", ssz_body(&req)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let decoded = ForkchoiceUpdatedResponseV1::from_ssz_bytes(&body).unwrap();
    assert_eq!(decoded.payload_status.status, 1, "expected INVALID");
    assert!(decoded.payload_id.is_empty());
}

#[tokio::test]
async fn new_payload_v1_invalid_returns_status() {
    let app = make_router().await;
    let p = ExecutionPayloadV1 {
        parent_hash: [0u8; 32],
        fee_recipient: [0u8; 20],
        state_root: [0u8; 32],
        receipts_root: [0u8; 32],
        logs_bloom: [0u8; 256],
        prev_randao: [0u8; 32],
        block_number: 1,
        gas_limit: 30_000_000,
        gas_used: 0,
        timestamp: 1,
        extra_data: Vec::new().try_into().unwrap(),
        base_fee_per_gas: [0u8; 32],
        block_hash: [0u8; 32],
        transactions: Vec::<
            SszList<u8, { ethrex_rpc::engine_rest::types::common::MAX_BYTES_PER_TRANSACTION }>,
        >::new()
        .try_into()
        .unwrap(),
    };
    let req = NewPayloadV1Request {
        execution_payload: p,
    };
    let resp = app
        .oneshot(auth_post("/engine/v1/payloads", ssz_body(&req)))
        .await
        .unwrap();
    // All-zero payload at timestamp=1: validators pass, then validate_block_hash
    // rejects (computed hash != payload.block_hash=0), which is reported as a
    // 200 OK with status=INVALID per the engine spec.
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let s = PayloadStatusV1::from_ssz_bytes(&body).unwrap();
    assert_eq!(s.status, 1, "expected INVALID, got {}", s.status);
}

#[tokio::test]
async fn malformed_ssz_body_returns_bad_request() {
    let app = make_router().await;
    let resp = app
        .oneshot(auth_post("/engine/v1/payloads", vec![0u8; 4]))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn get_payload_v1_unknown_returns_404() {
    // Empty store has no built payload — classify_rpc_err must map
    // UnknownPayload to 404, not 500.
    let app = make_router().await;
    let resp = app
        .oneshot(auth_get("/engine/v1/payloads/0x0000000000000000"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_payload_v1_invalid_hex_returns_400() {
    let app = make_router().await;
    let resp = app
        .oneshot(auth_get("/engine/v1/payloads/notHex"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn get_payload_v1_wrong_length_returns_400() {
    let app = make_router().await;
    let resp = app
        .oneshot(auth_get("/engine/v1/payloads/0xdeadbeef"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

fn empty_payload_v2(timestamp: u64) -> ExecutionPayloadV2 {
    ExecutionPayloadV2 {
        parent_hash: [0u8; 32],
        fee_recipient: [0u8; 20],
        state_root: [0u8; 32],
        receipts_root: [0u8; 32],
        logs_bloom: [0u8; 256],
        prev_randao: [0u8; 32],
        block_number: 1,
        gas_limit: 30_000_000,
        gas_used: 0,
        timestamp,
        extra_data: Vec::new().try_into().unwrap(),
        base_fee_per_gas: [0u8; 32],
        block_hash: [0u8; 32],
        transactions: Vec::<
            SszList<u8, { ethrex_rpc::engine_rest::types::common::MAX_BYTES_PER_TRANSACTION }>,
        >::new()
        .try_into()
        .unwrap(),
        withdrawals: Vec::<WithdrawalV1>::new().try_into().unwrap(),
    }
}

#[tokio::test]
async fn new_payload_v2_pre_shanghai_with_withdrawals_returns_422() {
    // Shanghai not activated; sending any withdrawals must be rejected with 422.
    let cc = ChainConfig {
        chain_id: 1,
        shanghai_time: None,
        deposit_contract_address: Address::zero(),
        ..Default::default()
    };
    let app = make_router_with_chain_config(cc).await;
    let mut payload = empty_payload_v2(1);
    payload.withdrawals = vec![WithdrawalV1 {
        index: 0,
        validator_index: 0,
        address: [0u8; 20],
        amount: 0,
    }]
    .try_into()
    .unwrap();
    let req = NewPayloadV2Request {
        execution_payload: payload,
    };
    let resp = app
        .oneshot(auth_post("/engine/v2/payloads", ssz_body(&req)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn new_payload_v2_pre_shanghai_empty_withdrawals_accepted() {
    // Empty withdrawals pre-Shanghai must be silently stripped, not rejected.
    let cc = ChainConfig {
        chain_id: 1,
        shanghai_time: None,
        deposit_contract_address: Address::zero(),
        ..Default::default()
    };
    let app = make_router_with_chain_config(cc).await;
    let req = NewPayloadV2Request {
        execution_payload: empty_payload_v2(1),
    };
    let resp = app
        .oneshot(auth_post("/engine/v2/payloads", ssz_body(&req)))
        .await
        .unwrap();
    // We accept any status except 422 — the pre-Shanghai-with-withdrawals
    // rejection should NOT fire when the withdrawals list is empty.
    assert_ne!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
