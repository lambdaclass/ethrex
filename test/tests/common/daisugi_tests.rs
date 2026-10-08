//! The Daisugi testnet: a chain-1337 network on a Prague base that activates
//! EIP-8141 frame transactions and EIP-8288 dependency aggregation by timestamp.
//!
//! These pin facts about the live network that ethrex must reproduce to follow it.

use ethrex_common::H256;
use ethrex_common::types::{ChainConfig, ChainFeatures, Fork, Genesis};

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

fn daisugi_config() -> ChainConfig {
    let genesis: Genesis = serde_json::from_str(DAISUGI_GENESIS).expect("genesis parses");
    genesis.config
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
