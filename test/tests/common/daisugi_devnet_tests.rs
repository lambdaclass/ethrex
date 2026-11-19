//! daisugi-devnet-0's genesis and fork schedule.
//!
//! The devnet runs Prague with EIP-8141 frame transactions from genesis
//! (`eip8141PrototypeTime`) and EIP-8288, EIP-8250 and EIP-7906 from its Daisugi fork
//! (`eip8288PrototypeTime`). The expected values come from the network's published
//! configuration (ethpandaops/daisugi-devnets, `network-configs/devnet-0/metadata`).

use std::path::PathBuf;

use bytes::Bytes;
use ethrex_common::types::{
    ChainConfig, ChainFeatures, FRAME_SIG_SCHEME_P256, Fork, ForkId, Frame, FrameMode,
    FrameSignature, FrameTransaction, Genesis, Transaction,
};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use std::str::FromStr;

const GENESIS_TIMESTAMP: u64 = 1_795_013_940;
const DAISUGI_FORK_TIMESTAMP: u64 = 1_795_017_840;

fn genesis() -> Genesis {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures/genesis/daisugi-devnet-0.json");
    Genesis::try_from(path.as_path()).expect("the devnet genesis loads")
}

#[test]
fn the_genesis_block_hash_matches_the_network() {
    let expected =
        H256::from_str("0x3c3a46744395b7beae03e0f994ec80c46d1b8d7646aced7e7b0501dcded7765d")
            .unwrap();
    assert_eq!(genesis().get_block().hash(), expected);
}

#[test]
fn frame_transactions_run_from_genesis_and_the_rest_from_the_daisugi_fork() {
    let config = genesis().config;
    let frames_only = ChainFeatures {
        frame_transactions: true,
        ..Default::default()
    };
    let everything = ChainFeatures {
        frame_transactions: true,
        keyed_nonces: true,
        assert_frames: true,
        dependency_proofs: true,
    };
    assert_eq!(config.features(GENESIS_TIMESTAMP), frames_only);
    assert_eq!(config.features(DAISUGI_FORK_TIMESTAMP - 1), frames_only);
    assert_eq!(config.features(DAISUGI_FORK_TIMESTAMP), everything);
}

#[test]
fn the_fork_id_counts_the_daisugi_fork_but_not_genesis() {
    let genesis = genesis();
    let header = genesis.get_block().header;
    let before = ForkId::new(genesis.config, header.clone(), GENESIS_TIMESTAMP, 0);
    assert_eq!(before.fork_hash.as_bytes(), &[0x7a, 0x7a, 0xe4, 0x84]);
    assert_eq!(before.fork_next, DAISUGI_FORK_TIMESTAMP);

    let after = ForkId::new(genesis.config, header, DAISUGI_FORK_TIMESTAMP, 325);
    assert_eq!(after.fork_hash.as_bytes(), &[0x82, 0xaa, 0x1e, 0x1e]);
    assert_eq!(after.fork_next, 0);
}

#[test]
fn eip8288_alone_schedules_frame_transactions_at_the_same_time() {
    let mut config = ChainConfig {
        eip8288_prototype_time: Some(100),
        ..Default::default()
    };
    config.check_prototype_schedule().unwrap();
    assert_eq!(config.eip8141_prototype_time, Some(100));
}

#[test]
fn eip8288_may_not_activate_before_frame_transactions() {
    let mut config = ChainConfig {
        eip8141_prototype_time: Some(200),
        eip8288_prototype_time: Some(100),
        ..Default::default()
    };
    assert!(config.check_prototype_schedule().is_err());
}

#[test]
fn the_aggregated_key_override_needs_its_timestamp() {
    let mut key_only = ChainConfig {
        eip8288_aggregated_vk: Some(H256::repeat_byte(1)),
        ..Default::default()
    };
    assert!(key_only.check_prototype_schedule().is_err());

    let mut timestamp_only = ChainConfig {
        eip8288_aggregated_vk_timestamp: Some(100),
        ..Default::default()
    };
    assert!(timestamp_only.check_prototype_schedule().is_err());
}

// ==================== Frame transaction envelopes ====================

/// The devnet's first three frame transactions (blocks 673, 675 and 676), rebuilt from
/// their JSON-RPC form and checked against their published hashes. 675 and 676 use the
/// keyed envelope with one key; 673 uses the empty key set.
const TX_673: &str = "0x06f901238429d103d8c0809405a20b6083bd71c58c10cbfa9db8036bf5a8e206f898cb010380c5830186a0808080f86b048080c48275308080b860000000000000000000000000000000000000000000000000000000000000001100000000000000000000000000000000000000000000000000000000000000004f0922a927365fec2cf13867145a3de76b3d26385681cd100c9b3d23c3c03d46de028094000000000000000000000000000000000000deadc482c350807b80f85cf85a019405a20b6083bd71c58c10cbfa9db8036bf5a8e20680b841016f7832e83fd7c388595d7b446ab9d1b05922bdd84cbc190e73a298f540e3d7a03470c708c423657fae82b574f91da2d15c0565c8aa564ea924e19b969145a544cd873ff2e795f500008477359400c0";
const TX_675: &str = "0x06f9012b8429d103d8c584ccbd53ec809405a20b6083bd71c58c10cbfa9db8036bf5a8e206f89bce010380c8830186a083017e808080f86b048080c48275308080b860000000000000000000000000000000000000000000000000000000000000001100000000000000000000000000000000000000000000000000000000000000004f0922a927365fec2cf13867145a3de76b3d26385681cd100c9b3d23c3c03d46de028094000000000000000000000000000000000000deadc482c350807b80f85cf85a019405a20b6083bd71c58c10cbfa9db8036bf5a8e20680b84101f9e4f70b43dcfddbd1ce7062d64373a45945d6772d68e28c35ca070c0fef77137420ebe89837279324ab8195934169a2b9f04542de1ec4eff441471a78c959c6cd8762bcad3f9900008477359400c0";
const TX_676: &str = "0x06f9012b8429d103d8c584ccbd53ec019405a20b6083bd71c58c10cbfa9db8036bf5a8e206f89bce010380c8830186a083017e808080f86b048080c48275308080b860000000000000000000000000000000000000000000000000000000000000001100000000000000000000000000000000000000000000000000000000000000004f0922a927365fec2cf13867145a3de76b3d26385681cd100c9b3d23c3c03d46de028094000000000000000000000000000000000000deadc482c350807b80f85cf85a019405a20b6083bd71c58c10cbfa9db8036bf5a8e20680b8410138250a893bc2bb6b6d27e7a1b49ca1fce367fa9646ff5fca69e294a83e345fca3c2d7af2357f06da24e1a6a9ae65c9728a2830460d142f674acb7dcd36960cb0cd8762bcad3f9900008477359400c0";

fn decode_frame_tx(raw: &str) -> (Vec<u8>, FrameTransaction) {
    let bytes = hex::decode(raw.trim_start_matches("0x")).unwrap();
    match Transaction::decode_canonical(&bytes).unwrap() {
        Transaction::FrameTransaction(tx) => (bytes, tx),
        other => panic!("expected a frame transaction, got {other:?}"),
    }
}

#[test]
fn the_devnet_frame_transactions_decode_and_hash_as_published() {
    for (raw, hash, keys, nonce_seq) in [
        (
            TX_673,
            "0x01fb5a14e6af4d73e8e7f62ef2fbbc4bb013ce5bc4e4400716d5c69a09434773",
            vec![],
            0,
        ),
        (
            TX_675,
            "0x576438a5eff9cac236779606800c9ed4b172faf3e760557eef527c3739477f38",
            vec![U256::from(0xccbd53ecu64)],
            0,
        ),
        (
            TX_676,
            "0x9c55285a3bdb6f3815ec45de4c158e8d0a8110b51201c23df5acebd0b485b2e3",
            vec![U256::from(0xccbd53ecu64)],
            1,
        ),
    ] {
        let (bytes, tx) = decode_frame_tx(raw);
        assert!(tx.has_nonce_keys);
        assert_eq!(tx.nonce_keys, keys);
        assert_eq!(tx.nonce_seq, nonce_seq);
        let tx = Transaction::FrameTransaction(tx);
        assert_eq!(tx.hash(&NativeCrypto), H256::from_str(hash).unwrap());
        assert_eq!(tx.encode_canonical_to_vec(), bytes);
    }
}

/// A frame transaction in EIP-8141's own envelope, before EIP-8250: a scalar
/// `nonce` right after `chain_id`.
fn scalar_nonce_tx() -> FrameTransaction {
    FrameTransaction {
        chain_id: 701_563_864,
        has_nonce_keys: false,
        nonce_seq: 5,
        sender: Address::from_low_u64_be(0xABCD),
        frames: vec![Frame {
            mode: FrameMode::Verify as u8,
            flags: 3,
            gas_limit: 50_000,
            ..Default::default()
        }],
        max_fee: U256::from(1_000_000_000_000_000u64),
        ..Default::default()
    }
}

#[test]
fn the_scalar_nonce_envelope_round_trips_without_nonce_calldata() {
    let tx = scalar_nonce_tx();
    let encoded = Transaction::FrameTransaction(tx.clone()).encode_canonical_to_vec();
    // chain_id (a 4-byte string) is followed directly by the scalar nonce 5, not by a list.
    assert!(
        encoded
            .windows(6)
            .any(|w| w == [0x84, 0x29, 0xd1, 0x03, 0xd8, 0x05])
    );

    let (_, decoded) = decode_frame_tx(&format!("0x{}", hex::encode(&encoded)));
    assert!(!decoded.has_nonce_keys);
    assert!(decoded.nonce_keys.is_empty());
    assert_eq!(decoded.nonce_seq, 5);
    assert!(decoded.nonce_calldata().is_empty());

    // The keyed envelope with the empty key set is a different transaction.
    let keyed = FrameTransaction {
        has_nonce_keys: true,
        ..tx.clone()
    };
    assert_ne!(keyed.compute_sig_hash(), tx.compute_sig_hash());
    assert!(!keyed.nonce_calldata().is_empty());
}

#[test]
fn each_envelope_is_valid_only_on_its_side_of_the_daisugi_fork() {
    let config = genesis().config;
    let before = config.features(DAISUGI_FORK_TIMESTAMP - 1);
    let after = config.features(DAISUGI_FORK_TIMESTAMP);
    let scalar = scalar_nonce_tx();
    let keyed = FrameTransaction {
        has_nonce_keys: true,
        ..scalar.clone()
    };
    assert!(scalar.validate_fork_rules(before, Fork::Prague).is_ok());
    assert!(scalar.validate_fork_rules(after, Fork::Prague).is_err());
    assert!(keyed.validate_fork_rules(before, Fork::Prague).is_err());
    assert!(keyed.validate_fork_rules(after, Fork::Prague).is_ok());
}

#[test]
fn assert_frames_need_the_daisugi_fork() {
    let config = genesis().config;
    let mut tx = scalar_nonce_tx();
    tx.frames.push(Frame {
        mode: FrameMode::Assert as u8,
        ..Default::default()
    });
    assert!(
        tx.validate_fork_rules(config.features(DAISUGI_FORK_TIMESTAMP - 1), Fork::Prague)
            .is_err()
    );
    tx.has_nonce_keys = true;
    assert!(
        tx.validate_fork_rules(config.features(DAISUGI_FORK_TIMESTAMP), Fork::Prague)
            .is_ok()
    );
}

#[test]
fn p256_signatures_need_the_osaka_precompile() {
    let features = genesis().config.features(GENESIS_TIMESTAMP);
    let mut tx = scalar_nonce_tx();
    tx.signatures.push(FrameSignature {
        scheme: FRAME_SIG_SCHEME_P256,
        signer: None,
        msg: Bytes::new(),
        signature: Bytes::from(vec![1u8; 128]),
    });
    assert!(tx.validate_fork_rules(features, Fork::Prague).is_err());
    assert!(tx.validate_fork_rules(features, Fork::Osaka).is_ok());
}
