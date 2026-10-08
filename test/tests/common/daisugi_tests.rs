//! The Daisugi testnet: a chain-1337 network on a Prague base that activates
//! EIP-8141 frame transactions and EIP-8288 dependency aggregation by timestamp.
//!
//! These pin facts about the live network that ethrex must reproduce to follow it.
//! The transactions below are rebuilt field by field from the chain's JSON-RPC
//! output; a hash that matches the chain's proves the encoding, and the receipts
//! fix the gas each one was charged.

use bytes::Bytes;
use ethrex_common::types::{
    ChainConfig, ChainFeatures, Fork, Frame, FrameSignature, FrameTransaction, Genesis, Transaction,
};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;

const DAISUGI_GENESIS: &str = include_str!("../../../fixtures/genesis/daisugi.json");

/// The genesis hash every Daisugi node agrees on.
const DAISUGI_GENESIS_HASH: &str =
    "3f08ccf3cbc9a60e7a328a3260f2fccf1ee2e8e36e647f030f77b8122cd83e55";

/// `eip8141PrototypeTime`: frame transactions activate (block 519,324).
const FRAMES_TIME: u64 = 1_790_274_781;
/// `eip8288PrototypeTime`: EIP-8288 and keyed nonces activate (block 1,000,889).
const DEPENDENCIES_TIME: u64 = 1_791_238_303;

fn h256(hex_str: &str) -> H256 {
    H256::from_slice(&hex::decode(hex_str.trim_start_matches("0x")).unwrap())
}

fn address(hex_str: &str) -> Address {
    Address::from_slice(&hex::decode(hex_str.trim_start_matches("0x")).unwrap())
}

fn bytes(hex_str: &str) -> Bytes {
    Bytes::from(hex::decode(hex_str.trim_start_matches("0x")).unwrap())
}

fn frame(
    mode: u8,
    flags: u8,
    target: Option<&str>,
    gas_limit: u64,
    state_gas_limit: u64,
    value: u64,
    data: &str,
) -> Frame {
    Frame {
        mode,
        flags,
        target: target.map(address),
        gas_limit,
        state_gas_limit,
        value: U256::from(value),
        data: bytes(data),
    }
}

fn daisugi_config() -> ChainConfig {
    let genesis: Genesis = serde_json::from_str(DAISUGI_GENESIS).expect("genesis parses");
    genesis.config
}

/// Block 519,326, the first frame transaction: scalar nonce, one SECP256K1
/// signature over the signature hash, a VERIFY frame running the default code and
/// a SENDER frame moving 123 wei.
fn scalar_secp256k1_tx() -> FrameTransaction {
    let sender = "0xb8e29b2ff15044db5c2bc61728bc9b6a58b1d999";
    FrameTransaction {
        chain_id: 1337,
        nonce: 0,
        nonce_keys: None,
        sender: address(sender),
        frames: vec![
            frame(1, 3, Some(sender), 150_000, 0, 0, "0x"),
            frame(
                2,
                0,
                Some("0xbbabf5a253e85d3577d00fced5b8835e328206da"),
                100_000,
                100_000,
                123,
                "0x",
            ),
        ],
        signatures: vec![FrameSignature {
            scheme: 1,
            signer: Some(address(sender)),
            msg: Bytes::new(),
            signature: bytes(
                "0x008f4f48e2c656156a362c7c3c30f5f6c2c3919749330f1a088f9df2ee357d970f0f0f9c4b5265c8de5319e8c61261d0feed18fbf323b12bccf9a6e5bbb034d2ad",
            ),
        }],
        max_priority_fee_per_gas: U256::from(0x3b9aca00u64),
        max_fee_per_gas: U256::from(0x3b9aca0eu64),
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: vec![],
        recent_root_refs: None,
        ..Default::default()
    }
}

/// Block 1,022,048: keyed nonce set `[0]`, a DEFAULT frame deploying the sender's
/// account, a DEP_VERIFY frame with one LEANSPHINCS triple, then VERIFY and SENDER.
fn keyed_dependency_tx() -> FrameTransaction {
    let sender = "0x061970b82d71ed71c4a0954d528685bc79fee231";
    FrameTransaction {
        chain_id: 1337,
        nonce: 0,
        nonce_keys: Some(vec![U256::zero()]),
        sender: address(sender),
        frames: vec![
            frame(
                0,
                0,
                Some("0x4940e0d0883fa40985fec192fdd9e11c7a34d9d0"),
                100_000,
                450_000,
                0,
                "0x8ca6da3c0366f919ce66b78acb23f67d5e0f6c64000000000000000000000000000000002de6160d80933c1951574f96915fb2b9000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            ),
            frame(
                4,
                0,
                None,
                3_000,
                0,
                0,
                "0x0000000000000000000000000000000000000000000000000000000000000010ded6bbc9cddb29414b82b91a248d43c20c2efa231c86601a4ec4060c51d4a0d25e65faeafe52ac56b609bb52e6d9b502d8bfae068668135635b70229ba44e15c",
            ),
            frame(1, 3, None, 100_000, 0, 0, "0x"),
            frame(2, 0, Some(sender), 250_000, 250_000, 0, "0x"),
        ],
        signatures: vec![],
        max_priority_fee_per_gas: U256::from(0x3b9aca00u64),
        max_fee_per_gas: U256::from(0x3b9aca0eu64),
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: vec![],
        recent_root_refs: None,
        ..Default::default()
    }
}

#[test]
fn daisugi_genesis_hash_matches_the_network() {
    let genesis: Genesis = serde_json::from_str(DAISUGI_GENESIS).expect("genesis parses");
    let hash = genesis.get_block().hash();
    assert_eq!(hash, h256(DAISUGI_GENESIS_HASH), "genesis block hash");
}

#[test]
fn feature_schedule_follows_the_prototype_timestamps() {
    let config = daisugi_config();
    // The base fork never leaves Prague: no Osaka or Amsterdam rules apply.
    assert_eq!(config.fork(DEPENDENCIES_TIME + 1_000_000), Fork::Prague);

    assert_eq!(config.features(FRAMES_TIME - 1), ChainFeatures::default());
    assert_eq!(
        config.features(FRAMES_TIME),
        ChainFeatures {
            frame_transactions: true,
            legacy_frames: true,
            ..Default::default()
        }
    );
    assert_eq!(
        config.features(DEPENDENCIES_TIME),
        ChainFeatures {
            frame_transactions: true,
            dependency_frames: true,
            keyed_nonces: true,
            recent_roots: true,
            post_tx_frames: true,
            legacy_frames: true,
        }
    );
}

#[test]
fn scalar_frame_tx_hash_matches_the_chain() {
    let tx = Transaction::FrameTransaction(scalar_secp256k1_tx());
    assert_eq!(
        tx.hash(&NativeCrypto),
        h256("0x48e7f41c88c49370268cb2708fd4fcdc8e7ac4e0bca4c70fde6c1fd6df61ad2c")
    );
}

#[test]
fn keyed_frame_tx_hash_matches_the_chain() {
    let tx = Transaction::FrameTransaction(keyed_dependency_tx());
    assert_eq!(
        tx.hash(&NativeCrypto),
        h256("0xf6babf5118b7c5b4ba6befa0f8c0ce6eb725d42b52b7a4dccb19fb6f8e239b67")
    );
}

#[test]
fn keyed_frame_tx_round_trips_through_rlp() {
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
    let tx = keyed_dependency_tx();
    let decoded = FrameTransaction::decode(&tx.encode_to_vec()).expect("decodes");
    assert_eq!(decoded.nonce_keys, Some(vec![U256::zero()]));
    assert_eq!(decoded, tx);
    let scalar = scalar_secp256k1_tx();
    let decoded = FrameTransaction::decode(&scalar.encode_to_vec()).expect("decodes");
    assert_eq!(decoded.nonce_keys, None);
    assert_eq!(decoded, scalar);
}

#[test]
fn signature_hash_matches_what_the_live_signer_signed() {
    let tx = scalar_secp256k1_tx();
    assert!(ethrex_levm::vm::validate_frame_signatures(
        &tx.signatures,
        tx.compute_sig_hash(),
        tx.sender,
        Fork::Prague,
        &NativeCrypto,
    ));
}

/// Intrinsic gas is the receipt's total minus what the frames used. On a Prague
/// base that is EIP-7623's weighted token count with no EIP-2780 value charge,
/// and in the keyed form the nonce key set and sequence are priced too.
#[test]
fn intrinsic_gas_matches_the_receipts() {
    // 19,798 total; frames used 100 + 2,600.
    let scalar = scalar_secp256k1_tx();
    assert_eq!(
        scalar
            .mandatory_gas(Fork::Prague)
            .saturating_add(scalar.data_cost()),
        19_798 - 100 - 2_600
    );
    // 81,791 total; frames used 54,808 + 3,000 + 7,730 + 309.
    let keyed = keyed_dependency_tx();
    assert_eq!(
        keyed
            .mandatory_gas(Fork::Prague)
            .saturating_add(keyed.data_cost()),
        81_791 - 54_808 - 3_000 - 7_730 - 309
    );
}

/// Block 1,000,889, the first block after `eip8288PrototypeTime`. It declares no
/// dependencies, so its `recursive_stark` is the 12-byte empty envelope and the
/// digest of the empty set, and the header is otherwise fully known from JSON-RPC.
/// Matching the chain's hash pins the layout: the Prague fields, then the empty
/// block-access-list placeholder and slot number zero, then the proof entry.
fn first_dependency_era_header() -> ethrex_common::types::BlockHeader {
    ethrex_common::types::BlockHeader {
        parent_hash: h256("0xf42f2896f95a25a6fa7acc33566e1e037d2d9f220cf4d9f821c2c85e7e33a0d6"),
        ommers_hash: h256("0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347"),
        coinbase: address("0x8943545177806ed17b9f23f0a21ee5948ecaa776"),
        state_root: h256("0x30b2863b5be476ae933c108697f209458a0566020d041df02597b09d3333baea"),
        transactions_root: h256(
            "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
        ),
        receipts_root: h256("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"),
        number: 1_000_889,
        gas_limit: 60_000_000,
        gas_used: 0,
        timestamp: DEPENDENCIES_TIME,
        extra_data: bytes("0x4e65746865726d696e642076322e322e3061"),
        prev_randao: h256("0x50de00f6fd3ffdadd946d5ed23a623447ae26c8d83d724b101a6f0fca6cccb26"),
        nonce: 0,
        base_fee_per_gas: Some(7),
        withdrawals_root: Some(h256(
            "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
        )),
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        parent_beacon_block_root: Some(h256(
            "0xcfddeb071b139b68c8de3f05c125368cf04b05052c42f806d824f3960645881f",
        )),
        requests_hash: Some(h256(
            "0xe3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        )),
        recursive_stark: Some(ethrex_common::types::RecursiveStark {
            proof: Bytes::copy_from_slice(&ethrex_dep_aggregation::envelope::EMPTY_ENVELOPE),
            block_deps_hash: ethrex_common::types::prototype_dependencies_hash(&[]),
        }),
        ..Default::default()
    }
}

#[test]
fn dependency_era_header_hash_matches_the_chain() {
    let header = first_dependency_era_header();
    assert_eq!(
        header.compute_block_hash(&NativeCrypto),
        h256("0x74ab09512f98aae72c1bfc5dedbca910f9759751a9f07ed97362835c821a1180")
    );
}

#[test]
fn dependency_era_header_round_trips_with_its_placeholders() {
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
    let header = first_dependency_era_header();
    let encoded = header.encode_to_vec();
    let decoded = ethrex_common::types::BlockHeader::decode(&encoded).expect("decodes");
    assert_eq!(decoded.block_access_list_hash, None);
    assert_eq!(decoded.slot_number, Some(0));
    assert_eq!(decoded.recursive_stark, header.recursive_stark);
    assert_eq!(decoded.encode_to_vec(), encoded);
}

/// The keyed form is valid only once keyed nonces are active, and the scalar
/// form survives them only through the network's compatibility rule.
#[test]
fn nonce_forms_follow_the_feature_schedule() {
    let config = daisugi_config();
    let era_a = config.features(FRAMES_TIME);
    let era_b = config.features(DEPENDENCIES_TIME);
    assert!(
        scalar_secp256k1_tx()
            .validate_fork_constraints(era_a)
            .is_ok()
    );
    assert!(
        scalar_secp256k1_tx()
            .validate_fork_constraints(era_b)
            .is_ok()
    );
    assert!(
        keyed_dependency_tx()
            .validate_fork_constraints(era_a)
            .is_err()
    );
    assert!(
        keyed_dependency_tx()
            .validate_fork_constraints(era_b)
            .is_ok()
    );
    let without_legacy = ChainFeatures {
        legacy_frames: false,
        ..era_b
    };
    assert!(
        scalar_secp256k1_tx()
            .validate_fork_constraints(without_legacy)
            .is_err()
    );
}

/// The JSON-RPC shape of a frame transaction, as the network's node serves it for
/// the same transaction: the key set, `executionGas`/`stateGas`/`target` per frame
/// with no `target` for a frame that has none, and `gas` as the frames' total
/// budget. Wallets and explorers built against the network read these names.
#[test]
fn frame_transaction_json_matches_the_network_shape() {
    let json = serde_json::to_value(keyed_dependency_tx()).expect("serializes");
    assert_eq!(json["nonceKeys"], serde_json::json!(["0x0"]));
    assert_eq!(json["gas"], "0x1197e8");
    assert!(
        json.get("sender").is_none(),
        "the sender is reported as `from`"
    );
    assert_eq!(
        json["frames"][1],
        serde_json::json!({
            "mode": "0x4",
            "flags": "0x0",
            "executionGas": "0xbb8",
            "stateGas": "0x0",
            "value": "0x0",
            "data": "0x0000000000000000000000000000000000000000000000000000000000000010ded6bbc9cddb29414b82b91a248d43c20c2efa231c86601a4ec4060c51d4a0d25e65faeafe52ac56b609bb52e6d9b502d8bfae068668135635b70229ba44e15c",
        })
    );
    assert_eq!(
        json["frames"][3]["target"],
        "0x061970b82d71ed71c4a0954d528685bc79fee231"
    );

    // An ARBITRARY signature names no signer, and the key is left out.
    let mut tx = scalar_secp256k1_tx();
    tx.signatures[0].scheme = 0;
    tx.signatures[0].signer = None;
    let json = serde_json::to_value(tx).expect("serializes");
    assert!(json["signatures"][0].get("signer").is_none());
    assert!(
        json.get("nonceKeys").is_none(),
        "the scalar form has no key set"
    );
}
