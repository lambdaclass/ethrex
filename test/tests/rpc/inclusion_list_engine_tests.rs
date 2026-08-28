//! EIP-7805 (FOCIL) engine API surface: `PayloadStatusV2`'s
//! `inclusionListSatisfied` field, per execution-apis `bogota.md`.

use ethrex_common::H256;
use ethrex_rpc::types::payload::{PayloadStatus, PayloadValidationStatus};

/// An unsatisfied inclusion list leaves the payload `VALID` and is reported only
/// through `inclusionListSatisfied`, so the status set stays
/// `VALID | INVALID | SYNCING | ACCEPTED`. The consensus layer uses the flag to
/// decide whether to attest, not whether to abandon the branch.
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

/// `bogota.md` requires `inclusionListSatisfied` to be `null` unless the payload
/// is `VALID`, and every pre-Bogotá method answers with `PayloadStatusV1`, which
/// has no such field at all. Both cases must serialize without it.
#[test]
fn payload_status_omits_inclusion_list_satisfied_when_unreported() {
    for status in [
        PayloadStatus::syncing(),
        PayloadStatus::accepted(),
        PayloadStatus::invalid_with_err("boom"),
        PayloadStatus::valid_with_hash(H256::zero()),
    ] {
        let json = serde_json::to_value(status).unwrap();
        assert!(json.get("inclusionListSatisfied").is_none());
    }
}
