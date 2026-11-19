//! daisugi-devnet-0's genesis and fork schedule.
//!
//! The devnet runs Prague with EIP-8141 frame transactions from genesis
//! (`eip8141PrototypeTime`) and EIP-8288, EIP-8250 and EIP-7906 from its Daisugi fork
//! (`eip8288PrototypeTime`). The expected values come from the network's published
//! configuration (ethpandaops/daisugi-devnets, `network-configs/devnet-0/metadata`).

use std::path::PathBuf;

use ethrex_common::H256;
use ethrex_common::types::{ChainConfig, ChainFeatures, ForkId, Genesis};
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
