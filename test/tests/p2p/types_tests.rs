use bytes::Bytes;
use ethrex_common::H512;
use ethrex_p2p::types::{Node, NodeRecord};
use ethrex_p2p::utils::public_key_from_signing_key;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;
use std::{net::SocketAddr, str::FromStr};

const TEST_GENESIS: &str = include_str!("../../../fixtures/genesis/l1.json");

#[test]
fn parse_node_from_enode_string() {
    let input = "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303";
    let bootnode = Node::from_enode_url(input).unwrap();
    let public_key = H512::from_str(
        "d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666")
        .unwrap();
    let socket_address = SocketAddr::from_str("18.138.108.67:30303").unwrap();
    let expected_bootnode = Node::new(
        socket_address.ip(),
        socket_address.port(),
        socket_address.port(),
        public_key,
    );
    assert_eq!(bootnode, expected_bootnode);
}

#[test]
fn parse_node_with_discport_from_enode_string() {
    let input = "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303?discport=30305";
    let node = Node::from_enode_url(input).unwrap();
    let public_key = H512::from_str(
        "d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666")
        .unwrap();
    let socket_address = SocketAddr::from_str("18.138.108.67:30303").unwrap();
    let expected_bootnode = Node::new(
        socket_address.ip(),
        30305,
        socket_address.port(),
        public_key,
    );
    assert_eq!(node, expected_bootnode);
}

#[test]
fn parse_node_from_enr_string() {
    // https://github.com/ethereum/devp2p/blob/master/enr.md#test-vectors
    let enr_string = "enr:-IS4QHCYrYZbAKWCBRlAy5zzaDZXJBGkcnh4MHcBFZntXNFrdvJjX04jRzjzCBOonrkTfj499SZuOh8R33Ls8RRcy5wBgmlkgnY0gmlwhH8AAAGJc2VjcDI1NmsxoQPKY0yuDUmstAHYpMa2_oxVtw0RW_QAdpzBQA8yWM0xOIN1ZHCCdl8";
    let node = Node::from_enr_url(enr_string).unwrap();
    let public_key =
        H512::from_str("0xca634cae0d49acb401d8a4c6b6fe8c55b70d115bf400769cc1400f3258cd31387574077f301b421bc84df7266c44e9e6d569fc56be00812904767bf5ccd1fc7f")
            .unwrap();
    let socket_address = SocketAddr::from_str("127.0.0.1:30303").unwrap();
    let expected_node = Node::new(
        socket_address.ip(),
        socket_address.port(),
        socket_address.port(),
        public_key,
    );
    assert_eq!(node, expected_node);
}

#[tokio::test]
async fn encode_node_record_to_enr_url() {
    // https://github.com/ethereum/devp2p/blob/master/enr.md#test-vectors
    let signer = SecretKey::from_slice(&[
        16, 125, 177, 238, 167, 212, 168, 215, 239, 165, 77, 224, 199, 143, 55, 205, 9, 194, 87,
        139, 92, 46, 30, 191, 74, 37, 68, 242, 38, 225, 104, 246,
    ])
    .unwrap();
    let addr = std::net::SocketAddr::from_str("127.0.0.1:30303").unwrap();

    let mut storage =
        Store::new("", EngineType::InMemory).expect("Failed to create in-memory storage");
    storage
        .add_initial_state(serde_json::from_str(TEST_GENESIS).unwrap())
        .await
        .expect("Failed to build test genesis");

    let node = Node::new(
        addr.ip(),
        addr.port(),
        addr.port(),
        public_key_from_signing_key(&signer),
    );
    let record = NodeRecord::from_node(&node, 1, &signer).unwrap();

    let expected_enr_string = "enr:-Iu4QIQVZPoFHwH3TCVkFKpW3hm28yj5HteKEO0QTVsavAGgD9ISdBmAgsIyUzdD9Yrqc84EhT067h1VA1E1HSLKcMgBgmlkgnY0gmlwhH8AAAGJc2VjcDI1NmsxoQJtSDUljLLg3EYuRCp8QJvH8G2F9rmUAQtPKlZjq_O7loN0Y3CCdl-DdWRwgnZf";

    assert_eq!(record.enr_url().unwrap(), expected_enr_string);
}

#[tokio::test]
async fn encode_decode_node_record_with_forkid() {
    let signer = SecretKey::from_slice(&[
        16, 125, 177, 238, 167, 212, 168, 215, 239, 165, 77, 224, 199, 143, 55, 205, 9, 194, 87,
        139, 92, 46, 30, 191, 74, 37, 68, 242, 38, 225, 104, 246,
    ])
    .unwrap();
    let addr = std::net::SocketAddr::from_str("127.0.0.1:30303").unwrap();

    let mut storage =
        Store::new("", EngineType::InMemory).expect("Failed to create in-memory storage");
    storage
        .add_initial_state(serde_json::from_str(TEST_GENESIS).unwrap())
        .await
        .expect("Failed to build test genesis");

    let node = Node::new(
        addr.ip(),
        addr.port(),
        addr.port(),
        public_key_from_signing_key(&signer),
    );
    let fork_id = storage.get_fork_id().await.unwrap();

    let mut record = NodeRecord::from_node(&node, 1, &signer).unwrap();
    record.set_fork_id(fork_id.clone(), &signer).unwrap();

    record.sign_record(&signer).unwrap();

    let enr_url = record.enr_url().unwrap();
    let base64_decoded = ethrex_common::base64::decode(&enr_url.as_bytes()[4..]);
    let parsed_record = NodeRecord::decode(&base64_decoded).unwrap();
    let pairs = parsed_record.pairs();

    assert_eq!(pairs.eth, Some(fork_id));
}

#[test]
fn verify_enr_signature_valid() {
    // https://github.com/ethereum/devp2p/blob/master/enr.md#test-vectors
    let enr_string = "enr:-IS4QHCYrYZbAKWCBRlAy5zzaDZXJBGkcnh4MHcBFZntXNFrdvJjX04jRzjzCBOonrkTfj499SZuOh8R33Ls8RRcy5wBgmlkgnY0gmlwhH8AAAGJc2VjcDI1NmsxoQPKY0yuDUmstAHYpMa2_oxVtw0RW_QAdpzBQA8yWM0xOIN1ZHCCdl8";
    let base64_decoded = ethrex_common::base64::decode(&enr_string.as_bytes()[4..]);
    let record = NodeRecord::decode(&base64_decoded).unwrap();
    assert!(record.verify_signature());
}

#[test]
fn verify_enr_signature_invalid() {
    // Use a valid ENR and tamper with the signature
    let enr_string = "enr:-IS4QHCYrYZbAKWCBRlAy5zzaDZXJBGkcnh4MHcBFZntXNFrdvJjX04jRzjzCBOonrkTfj499SZuOh8R33Ls8RRcy5wBgmlkgnY0gmlwhH8AAAGJc2VjcDI1NmsxoQPKY0yuDUmstAHYpMa2_oxVtw0RW_QAdpzBQA8yWM0xOIN1ZHCCdl8";
    let base64_decoded = ethrex_common::base64::decode(&enr_string.as_bytes()[4..]);
    let mut record = NodeRecord::decode(&base64_decoded).unwrap();
    // Tamper with the signature
    record.signature = ethrex_common::H512::zero();
    assert!(!record.verify_signature());
}

#[test]
fn verify_enr_signature_fails_when_decode_drops_unknown_pairs() {
    /*
    Record has sequence number 1 and 7 key/value pairs.
        "attnets"   0000000000000000
        "eth2"      fdca39b000000121ffffffffffffffff
        "id"        "v4"
        "ip"        192.168.86.67
        "secp256k1" 0311501bf6f21a04763aedb7b408c14b514de61c29eb9bd902a0884b2f9a2653d5
        "tcp"       13000
        "udp"       12000
    */
    let enr_string = "enr:-LK4QMer7ejH4SWXlSIdM6gOBUD6WH86M95-6ZQ04KOrsAWaDaswyYp9hFmzRpnGVypSlHL_QB2VzNT8ATRckIfnmosBh2F0dG5ldHOIAAAAAAAAAACEZXRoMpD9yjmwAAABIf__________gmlkgnY0gmlwhMCoVkOJc2VjcDI1NmsxoQMRUBv28hoEdjrtt7QIwUtRTeYcKeub2QKgiEsvmiZT1YN0Y3CCMsiDdWRwgi7g";
    let raw_record = ethrex_common::base64::decode(&enr_string.as_bytes()[4..]);
    let decoded = NodeRecord::decode(&raw_record).unwrap();
    let pairs = decoded.pairs();

    assert!(!pairs.other.is_empty());
    assert!(
        pairs
            .other
            .iter()
            .any(|(key, _)| key == &Bytes::from_static(b"attnets"))
    );
    assert!(
        pairs
            .other
            .iter()
            .any(|(key, _)| key == &Bytes::from_static(b"eth2"))
    );
    assert_eq!(decoded.pairs().tcp_port, Some(13000));
    assert_eq!(decoded.encode_to_vec(), raw_record);
    assert!(decoded.verify_signature());
}
