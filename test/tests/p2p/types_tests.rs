use bytes::Bytes;
use ethrex_common::H512;
use ethrex_p2p::types::{Node, NodeRecord, NodeRecordPairs};
use ethrex_p2p::utils::public_key_from_signing_key;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;
use std::{
    net::{Ipv4Addr, SocketAddr},
    str::FromStr,
};

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

    assert!(!pairs.extra_fields.is_empty());
    assert!(
        pairs
            .extra_fields
            .iter()
            .any(|(key, _)| key == &Bytes::from_static(b"attnets"))
    );
    assert!(
        pairs
            .extra_fields
            .iter()
            .any(|(key, _)| key == &Bytes::from_static(b"eth2"))
    );
    assert_eq!(decoded.pairs().tcp_port, Some(13000));
    assert_eq!(decoded.encode_to_vec(), raw_record);
    assert!(decoded.verify_signature());

    // The accessors hand the payloads back without ethrex having to know what
    // shape they are meant to be in.
    assert_eq!(pairs.extra(b"attnets"), Some(Bytes::from_static(&[0; 8])));
    assert_eq!(
        pairs.extra(b"eth2"),
        Some(Bytes::from_static(&[
            0xfd, 0xca, 0x39, 0xb0, 0x00, 0x00, 0x01, 0x21, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff,
        ]))
    );
}

#[test]
fn unknown_pairs_survive_a_decode_encode_round_trip() {
    // Same intent as the assertions above, on a key nothing will ever start
    // recognising: an entry outside the dictionary has to come back verbatim
    // through decode and re-encode, or the signature over it stops verifying.
    let mut pairs = NodeRecordPairs {
        ip: Some(Ipv4Addr::LOCALHOST),
        udp_port: Some(30303),
        ..Default::default()
    };
    assert!(pairs.set_extra(b"zzz-not-a-real-entry", Bytes::from_static(b"abc")));
    let record = NodeRecord::from_pairs(1, &test_signer(), pairs).unwrap();

    let decoded = through_enr_url(&record);
    assert_eq!(decoded.pairs().extra_fields, record.pairs().extra_fields);
    assert_eq!(decoded.encode_to_vec(), record.encode_to_vec());
    assert!(decoded.verify_signature());
}

// --- extra ENR entries + `from_pairs` ---

fn test_signer() -> SecretKey {
    SecretKey::from_slice(&[
        16, 125, 177, 238, 167, 212, 168, 215, 239, 165, 77, 224, 199, 143, 55, 205, 9, 194, 87,
        139, 92, 46, 30, 191, 74, 37, 68, 242, 38, 225, 104, 246,
    ])
    .unwrap()
}

/// Round-trip a record through the `enr:` URL form, the way a remote peer
/// would receive it.
fn through_enr_url(record: &NodeRecord) -> NodeRecord {
    let url = record.enr_url().unwrap();
    let raw = ethrex_common::base64::decode(&url.as_bytes()[4..]);
    NodeRecord::decode(&raw).unwrap()
}

/// An entry set covering both shapes the helpers encode: an opaque payload
/// ethrex never interprets, and an integer.
fn with_extra_entries(pairs: &mut NodeRecordPairs) {
    pairs.set_extra(b"opaque", Bytes::from_static(&[0xfd, 0xca, 0x39, 0xb0]));
    pairs.set_extra_int(b"quic", 9001);
}

#[test]
fn set_extra_round_trips_every_extra_entry() {
    let mut pairs = NodeRecordPairs {
        ip: Some(Ipv4Addr::LOCALHOST),
        udp_port: Some(30303),
        ..Default::default()
    };
    with_extra_entries(&mut pairs);
    let record = NodeRecord::from_pairs(1, &test_signer(), pairs).unwrap();

    let decoded = through_enr_url(&record);
    assert!(decoded.verify_signature());

    let pairs = decoded.pairs();
    assert_eq!(
        pairs.extra(b"opaque"),
        Some(Bytes::from_static(&[0xfd, 0xca, 0x39, 0xb0]))
    );
    assert_eq!(pairs.extra_int::<u16>(b"quic"), Some(9001));
}

#[test]
fn set_extra_encodes_a_byte_string_not_a_list() {
    // The footgun this helper exists to remove. A bare `Vec<u8>` hits the
    // blanket `Vec<T>` impl and emits an RLP *list* of per-byte scalars, which
    // round-trips fine locally and is unreadable to every other client. Pin the
    // encoded bytes so the distinction cannot silently regress.
    let mut pairs = NodeRecordPairs::default();
    pairs.set_extra(b"opaque", Bytes::from_static(&[0xaa, 0xbb]));

    let (_, value) = pairs
        .extra_fields
        .iter()
        .find(|(key, _)| key.as_ref() == b"opaque")
        .expect("opaque present");

    // 0x82 = byte string of length 2. A list would be 0xc2.
    assert_eq!(value.as_ref(), [0x82, 0xaa, 0xbb]);
}

#[test]
fn a_malformed_extra_entry_reads_as_missing_and_keeps_the_record_usable() {
    // Why the accessors answer `None` instead of propagating a decode error. A
    // peer emitting `quic` as a non-minimal integer, or an opaque payload as an
    // RLP list, must cost us that one entry and nothing more. Failing the whole
    // decode would cost the record, and because a discv5 NODES response decodes
    // `nodes` as a single field, every other peer in the batch with it.
    let mut pairs = NodeRecordPairs {
        ip: Some(Ipv4Addr::LOCALHOST),
        udp_port: Some(30303),
        ..Default::default()
    };
    // Pushed rather than set, because no setter will produce these: they are
    // what arrives from a peer, and this record stands in for that one.
    // 0x81 0x0a: non-minimal encoding of 10.
    pairs.extra_fields.push((
        Bytes::from_static(b"quic"),
        Bytes::from_static(&[0x81, 0x0a]),
    ));
    // 0xc2 ...: a list where a byte string belongs.
    pairs.extra_fields.push((
        Bytes::from_static(b"opaque"),
        Bytes::from_static(&[0xc2, 0x01, 0x02]),
    ));
    let record = NodeRecord::from_pairs(1, &test_signer(), pairs).unwrap();

    // The record survives, byte-exact, signature intact.
    let decoded = through_enr_url(&record);
    assert!(decoded.verify_signature());
    assert_eq!(decoded.encode_to_vec(), record.encode_to_vec());
    assert_eq!(decoded.pairs().udp_port, Some(30303));

    // Only the unreadable entries are unavailable.
    assert_eq!(decoded.pairs().extra_int::<u16>(b"quic"), None);
    assert_eq!(decoded.pairs().extra(b"opaque"), None);
}

#[test]
fn set_extra_replaces_rather_than_duplicating_a_key() {
    // EIP-778 requires unique keys and `encode_pairs` does not deduplicate, so
    // an appended second entry would verify locally and be rejected by every
    // remote, with no local symptom.
    let mut pairs = NodeRecordPairs::default();
    pairs.set_extra_int(b"quic", 9001);
    pairs.set_extra_int(b"quic", 9002);

    assert_eq!(pairs.extra_fields.len(), 1);
    assert_eq!(pairs.extra_int::<u16>(b"quic"), Some(9002));
}

#[test]
fn a_dictionary_key_is_refused_as_an_extra_entry() {
    // The same unique-key rule, across the split rather than within the bag.
    // `tcp` already comes from `tcp_port`, so a second one from here would be
    // emitted twice: valid-looking locally, rejected by every remote.
    let mut pairs = NodeRecordPairs {
        ip: Some(Ipv4Addr::LOCALHOST),
        tcp_port: Some(30303),
        udp_port: Some(30303),
        ..Default::default()
    };

    assert!(!pairs.set_extra_int(b"tcp", 9999));
    assert!(!pairs.set_extra(b"id", Bytes::from_static(b"v5")));
    assert!(!pairs.set_extra(b"secp256k1", Bytes::from_static(b"\x01")));
    assert!(pairs.extra_fields.is_empty());

    // ...and the one key that is genuinely ours still goes in.
    assert!(pairs.set_extra_int(b"quic", 9001));

    let record = NodeRecord::from_pairs(1, &test_signer(), pairs).unwrap();
    let keys: Vec<_> = record
        .pairs()
        .encode_pairs()
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    let mut unique = keys.clone();
    unique.dedup();
    assert_eq!(keys, unique, "EIP-778 requires keys to be unique");
    assert_eq!(record.pairs().tcp_port, Some(30303));
    assert!(through_enr_url(&record).verify_signature());
}

#[test]
fn from_pairs_overwrites_the_identity_entries_it_owns() {
    // `id` and `secp256k1` are fixed by the v4 scheme and the signer, so a
    // caller stating them differently must not be able to sign a record that
    // claims another identity.
    let signer = test_signer();
    let record = NodeRecord::from_pairs(
        1,
        &signer,
        NodeRecordPairs {
            id: Some("v5".to_string()),
            secp256k1: Some(ethrex_common::H264::zero()),
            ip: Some(Ipv4Addr::LOCALHOST),
            udp_port: Some(30303),
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(record.pairs().id.as_deref(), Some("v4"));
    assert_ne!(record.pairs().secp256k1, Some(ethrex_common::H264::zero()));
    assert_eq!(
        Node::from_enr(&record).unwrap().public_key,
        public_key_from_signing_key(&signer)
    );
    assert!(through_enr_url(&record).verify_signature());
}

#[test]
fn edit_bumps_seq_once_and_preserves_untouched_entries() {
    // The whole point of `edit` over rebuilding from a `Node`: entries the
    // closure never mentions have to survive, extra ones included, and the
    // record must be re-signed exactly once.
    let signer = test_signer();
    let mut pairs = NodeRecordPairs {
        ip: Some(Ipv4Addr::LOCALHOST),
        udp_port: Some(30303),
        ..Default::default()
    };
    with_extra_entries(&mut pairs);
    let record = NodeRecord::from_pairs(1, &signer, pairs).unwrap();

    let mut edited = record.clone();
    let new_ip = Ipv4Addr::new(203, 0, 113, 7);
    edited
        .edit(&signer, |pairs| pairs.ip = Some(new_ip))
        .unwrap();

    assert_eq!(edited.seq, record.seq + 1, "exactly one seq bump");
    assert!(
        edited.verify_signature(),
        "must be re-signed after the edit"
    );

    let pairs = through_enr_url(&edited).pairs().clone();
    assert_eq!(pairs.ip, Some(new_ip));
    assert_eq!(pairs.udp_port, Some(30303));
    assert_eq!(pairs.extra_int::<u16>(b"quic"), Some(9001));
    assert_eq!(
        pairs.extra(b"opaque"),
        Some(Bytes::from_static(&[0xfd, 0xca, 0x39, 0xb0]))
    );
}

#[test]
fn an_extra_int_of_zero_encodes_as_the_empty_byte_string() {
    // ENR integers are big-endian with no leading zero bytes, so zero is the
    // empty byte string. RLP's integer codec already does exactly that, so what
    // this pins is that `set_extra_int` routes through it rather than padding
    // the value to a fixed width.
    let mut pairs = NodeRecordPairs::default();
    pairs.set_extra_int(b"counter", 0);
    let record = NodeRecord::from_pairs(1, &test_signer(), pairs).unwrap();

    let (_, value) = record
        .pairs()
        .encode_pairs()
        .into_iter()
        .find(|(key, _)| key.as_ref() == b"counter")
        .expect("counter entry present");
    assert_eq!(value.as_ref(), [0x80], "RLP empty byte string");

    // Zero must also stay distinguishable from an absent entry.
    assert_eq!(
        through_enr_url(&record)
            .pairs()
            .extra_int::<u64>(b"counter"),
        Some(0)
    );
}
