//! snap/2 capability-version gating tests (EIP-8189).
//!
//! Covers `SnapCapVersion::is_valid_code` and the version-aware
//! `Message::decode` dispatch that rejects cross-version codes at the
//! protocol boundary.

use ethrex_p2p::rlpx::message::{EthCapVersion, Message, SnapCapVersion};
use ethrex_rlp::error::RLPDecodeError;

#[test]
fn snap_v1_rejects_snap2_codes() {
    assert!(!SnapCapVersion::V1.is_valid_code(0x08));
    assert!(!SnapCapVersion::V1.is_valid_code(0x09));
    // snap/1 accepts 0x06 and 0x07
    assert!(SnapCapVersion::V1.is_valid_code(0x06));
    assert!(SnapCapVersion::V1.is_valid_code(0x07));
}

#[test]
fn snap_v2_rejects_trie_node_codes() {
    assert!(!SnapCapVersion::V2.is_valid_code(0x06));
    assert!(!SnapCapVersion::V2.is_valid_code(0x07));
    // snap/2 accepts 0x08, 0x09, and the shared codes 0x00-0x05
    assert!(SnapCapVersion::V2.is_valid_code(0x08));
    assert!(SnapCapVersion::V2.is_valid_code(0x09));
    assert!(SnapCapVersion::V2.is_valid_code(0x00));
}

#[test]
fn message_decode_rejects_snap1_code_on_v2_connection() {
    let eth_version = EthCapVersion::V68;
    // 0x06 is GetTrieNodes — valid in snap/1, rejected in snap/2
    let msg_id = eth_version.snap_capability_offset() + 0x06;
    let result = Message::decode(msg_id, &[], eth_version, Some(SnapCapVersion::V2));
    assert!(
        matches!(result, Err(RLPDecodeError::MalformedData)),
        "snap/2 connection must reject snap/1-only code 0x06"
    );
}

#[test]
fn message_decode_rejects_snap2_code_on_v1_connection() {
    let eth_version = EthCapVersion::V68;
    // 0x08 is Snap2GetBlockAccessLists — valid in snap/2, rejected in snap/1
    let msg_id = eth_version.snap_capability_offset() + 0x08;
    let result = Message::decode(msg_id, &[], eth_version, Some(SnapCapVersion::V1));
    assert!(
        matches!(result, Err(RLPDecodeError::MalformedData)),
        "snap/1 connection must reject snap/2-only code 0x08"
    );
}

#[test]
fn message_decode_rejects_snap_msg_with_no_snap_version() {
    // A snap-range message id with no negotiated snap version (None) must be rejected.
    let eth_version = EthCapVersion::V68;
    let msg_id = eth_version.snap_capability_offset();
    let result = Message::decode(msg_id, &[], eth_version, None);
    assert!(
        matches!(result, Err(RLPDecodeError::MalformedData)),
        "snap message with no negotiated snap version must return MalformedData"
    );
}
