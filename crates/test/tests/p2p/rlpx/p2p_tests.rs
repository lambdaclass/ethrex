use ethrex_p2p::rlpx::p2p::{Capability, DisconnectReason};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

#[test]
fn test_encode_capability() {
    let capability = Capability::eth(8);
    let encoded = capability.encode_to_vec();

    assert_eq!(&encoded, &[197_u8, 131, b'e', b't', b'h', 8]);
}

#[test]
fn test_decode_capability() {
    let encoded_bytes = &[197_u8, 131, b'e', b't', b'h', 8];
    let decoded = Capability::decode(encoded_bytes).unwrap();

    assert_eq!(decoded, Capability::eth(8));
}

#[test]
fn test_protocol() {
    let capability = Capability::eth(68);

    assert_eq!(capability.protocol(), "eth");
}

#[test]
fn test_disconnect_reason_all() {
    let all_reasons = DisconnectReason::all();

    assert_eq!(all_reasons.len(), 14);

    // This exhaustive match ensures we check all variants exist in all()
    // If a new variant is added to the enum, this match will fail to compile
    for reason in &all_reasons {
        match reason {
            DisconnectReason::DisconnectRequested
            | DisconnectReason::NetworkError
            | DisconnectReason::ProtocolError
            | DisconnectReason::UselessPeer
            | DisconnectReason::TooManyPeers
            | DisconnectReason::AlreadyConnected
            | DisconnectReason::IncompatibleVersion
            | DisconnectReason::InvalidIdentity
            | DisconnectReason::ClientQuitting
            | DisconnectReason::UnexpectedIdentity
            | DisconnectReason::SelfIdentity
            | DisconnectReason::PingTimeout
            | DisconnectReason::SubprotocolError
            | DisconnectReason::InvalidReason => {}
        }
    }
}
