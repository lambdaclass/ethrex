use ethrex_common::H32;
use ethrex_common::types::ForkId;
use ethrex_p2p::test_utils::local_node_record;
use ethrex_p2p::types::{Node, NodeRecord};
use ethrex_p2p::utils::public_key_from_signing_key;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use secp256k1::SecretKey;
use std::net::{IpAddr, Ipv4Addr};

fn signer() -> SecretKey {
    SecretKey::from_slice(&[7; 32]).unwrap()
}

fn local_node(signer: &SecretKey) -> Node {
    Node::new(
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
        30303,
        30303,
        public_key_from_signing_key(signer),
    )
}

#[test]
fn local_record_lists_served_snap_versions() {
    let signer = signer();
    let fork_id = ForkId {
        fork_hash: H32::from_low_u64_be(0xdead_beef),
        fork_next: 42,
    };

    let record = local_node_record(&local_node(&signer), Some(fork_id.clone()), &signer).unwrap();

    // The key followed by the list [1, 2].
    let snap_entry = [0x84, b's', b'n', b'a', b'p', 0xc2, 0x01, 0x02];
    let encoded = record.encode_to_vec();
    assert!(encoded.windows(snap_entry.len()).any(|w| w == snap_entry));

    let decoded = NodeRecord::decode(&encoded).unwrap();
    assert!(decoded.verify_signature());
    assert_eq!(decoded.pairs().snap, Some(vec![1, 2]));
    assert_eq!(decoded.pairs().eth, Some(fork_id));
}

#[test]
fn local_record_advertises_snap_without_a_fork_id() {
    let signer = signer();

    let record = local_node_record(&local_node(&signer), None, &signer).unwrap();

    assert_eq!(record.pairs().snap, Some(vec![1, 2]));
    assert_eq!(record.pairs().eth, None);
}
