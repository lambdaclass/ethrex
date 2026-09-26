use ethrex_common::{
    H256,
    types::{Block, BlockBody, BlockHeader, block_access_list::BlockAccessList},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::{
    engine::payload::NewPayloadV5Request,
    rpc::{RpcApiContext, RpcHandler},
    test_utils::default_context_with_storage,
    utils::RpcErrorMetadata,
};
use ethrex_storage::{EngineType, Store};
use serde_json::{Value, json};

async fn fresh_context() -> RpcApiContext {
    let store = Store::new("test", EngineType::InMemory).expect("store");
    default_context_with_storage(store).await
}

/// Build the 4-element engine_newPayloadV5 params with the given `blockAccessList`
/// JSON value spliced into an otherwise well-formed payload object.
fn v5_params(bal: Option<Value>) -> Option<Vec<Value>> {
    let payload = ethrex_rpc::types::payload::ExecutionPayload::from_block(
        Block::new(BlockHeader::default(), BlockBody::default()),
        None,
    );
    let mut payload_json = serde_json::to_value(payload).expect("payload to json");
    match bal {
        Some(v) => {
            payload_json["blockAccessList"] = v;
        }
        None => {
            payload_json
                .as_object_mut()
                .expect("payload object")
                .remove("blockAccessList");
        }
    }
    Some(vec![
        payload_json,
        json!([]),
        json!(H256::zero()),
        json!([]),
    ])
}

/// amsterdam.md newPayloadV5 spec 3: a `blockAccessList` that is well-formed DATA but
/// not a valid RLP encoding of the BAL MUST produce
/// `{status: INVALID, latestValidHash: null}` — a payload status, not -32602.
#[tokio::test]
async fn undecodable_bal_returns_invalid_status_not_an_error() {
    // 0xde opens a 30-byte RLP list but only 3 bytes follow: valid DATA, invalid RLP.
    let params = v5_params(Some(json!("0xdeadbeef")));

    let request = NewPayloadV5Request::parse(&params).expect("parse must not fail");
    assert!(
        request.undecodable_bal,
        "the BAL must be flagged undecodable"
    );

    let ctx = fresh_context().await;
    let response = request
        .handle(ctx)
        .await
        .expect("must be a result, not an error");
    assert_eq!(response["status"], "INVALID");
    assert_eq!(response["latestValidHash"], Value::Null);
    assert!(
        response["validationError"].is_string(),
        "validationError should carry the reason, got {response:?}"
    );
}

/// A valid RLP BAL (here the empty list) must not trip the undecodable flag, so the
/// request proceeds into regular validation.
#[test]
fn well_formed_bal_is_not_flagged() {
    let bal_hex = format!(
        "0x{}",
        hex::encode(BlockAccessList::default().encode_to_vec())
    );
    let params = v5_params(Some(json!(bal_hex)));

    let request = NewPayloadV5Request::parse(&params).expect("parse");
    assert!(!request.undecodable_bal);
    assert!(request.raw_bal_hash.is_some());
    assert!(
        request.payload.block_access_list.is_some(),
        "a decodable BAL must survive into the payload"
    );
}

/// amsterdam.md newPayloadV5 spec 2 (unchanged): a MISSING blockAccessList stays a
/// -32602 error — the INVALID-status rule is only for present-but-undecodable values.
#[tokio::test]
async fn missing_bal_still_returns_invalid_params() {
    let params = v5_params(None);
    let request = NewPayloadV5Request::parse(&params).expect("parse");
    assert!(!request.undecodable_bal);

    let ctx = fresh_context().await;
    let err = request.handle(ctx).await.expect_err("must be an error");
    let metadata = RpcErrorMetadata::from(err);
    assert_eq!(metadata.code, -32602);
}

/// A schema-invalid value (missing the mandatory 0x prefix) is not "undecodable RLP";
/// it fails DATA validation and stays -32602 at parse time.
#[test]
fn schema_invalid_bal_still_fails_parse_with_invalid_params() {
    let params = v5_params(Some(json!("deadbeef")));
    let Err(err) = NewPayloadV5Request::parse(&params) else {
        panic!("parse must fail for an unprefixed blockAccessList");
    };
    let metadata = RpcErrorMetadata::from(err);
    assert_eq!(metadata.code, -32602);
}
