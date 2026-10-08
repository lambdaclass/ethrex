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
/// base that is EIP-7623's weighted token count with no EIP-2780 value charge.
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
}
