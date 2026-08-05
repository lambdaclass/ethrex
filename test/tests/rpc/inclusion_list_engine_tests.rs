//! EIP-7805 (FOCIL) engine API surface: `PayloadStatusV2`'s
//! `inclusionListSatisfied` field and `engine_getInclusionListV1`'s parameter
//! list, both per execution-apis.

use ethrex_common::H256;
use ethrex_rpc::engine::inclusion_list::GetInclusionListV1Request;
use ethrex_rpc::rpc::RpcHandler;
use ethrex_rpc::types::payload::{PayloadStatus, PayloadValidationStatus};
use ethrex_rpc::utils::RpcErr;
use serde_json::json;

// `PayloadStatusV2`: an unsatisfied inclusion list leaves the payload `VALID`
// and is reported only through `inclusionListSatisfied`, so the status enum
// stays `VALID | INVALID | SYNCING | ACCEPTED`. The consensus layer uses the
// flag to decide whether to attest, not whether to abandon the branch.

#[test]
fn unsatisfied_inclusion_list_keeps_payload_valid() {
    let status = PayloadStatus::valid_with_hash(H256::zero()).with_inclusion_list_satisfied(false);

    assert_eq!(status.status, PayloadValidationStatus::Valid);

    let json = serde_json::to_value(status).unwrap();
    assert_eq!(json["status"], "VALID");
    assert_eq!(json["inclusionListSatisfied"], false);
}

#[test]
fn satisfied_inclusion_list_serializes_true() {
    let status = PayloadStatus::valid().with_inclusion_list_satisfied(true);

    let json = serde_json::to_value(status).unwrap();
    assert_eq!(json["inclusionListSatisfied"], true);
}

/// Every pre-Hegotá method answers with `PayloadStatusV1`, which has no
/// `inclusionListSatisfied` field, so an unreported verdict must not appear.
#[test]
fn payload_status_omits_inclusion_list_satisfied_when_unreported() {
    let json = serde_json::to_value(PayloadStatus::syncing()).unwrap();

    assert!(json.get("inclusionListSatisfied").is_none());
}

// `engine_getInclusionListV1` takes no parameters: the list is built from the
// node's own view of the mempool against its canonical head.

#[test]
fn get_inclusion_list_accepts_empty_params() {
    assert!(GetInclusionListV1Request::parse(&Some(vec![])).is_ok());
    assert!(GetInclusionListV1Request::parse(&None).is_ok());
}

#[test]
fn get_inclusion_list_rejects_any_param() {
    let one_param =
        GetInclusionListV1Request::parse(&Some(vec![json!(format!("0x{:064x}", 0x42u64))]));
    assert!(matches!(one_param, Err(RpcErr::BadParams(_))));

    let two_params = GetInclusionListV1Request::parse(&Some(vec![json!("0x00"), json!("0x01")]));
    assert!(matches!(two_params, Err(RpcErr::BadParams(_))));
}
