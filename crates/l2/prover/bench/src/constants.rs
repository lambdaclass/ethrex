// Blockchain related constants

use std::time::Duration;

use ethrex_core::types::{ChainConfig, BlobSchedule};
use revm_primitives::SpecId;

use lazy_static::lazy_static;

lazy_static! {

// Chain config for different forks as defined on https://ethereum.github.io/execution-spec-tests/v3.0.0/consuming_tests/common_types/#fork
pub static ref CANCUN_CONFIG: ChainConfig = ChainConfig {
    chain_id: 1_u64,
    homestead_block: Some(0),
    dao_fork_block: Some(0),
    dao_fork_support: true,
    eip150_block: Some(0),
    eip155_block: Some(0),
    eip158_block: Some(0),
    byzantium_block: Some(0),
    constantinople_block: Some(0),
    petersburg_block: Some(0),
    istanbul_block: Some(0),
    muir_glacier_block: Some(0),
    berlin_block: Some(0),
    london_block: Some(0),
    arrow_glacier_block: Some(0),
    gray_glacier_block: Some(0),
    merge_netsplit_block: Some(0),
    terminal_total_difficulty: Some(0),
    shanghai_time: Some(0),
    cancun_time: Some(0),
    prague_time: None,
    terminal_total_difficulty_passed: false,
    verkle_time: None,
    blob_schedule: BlobSchedule::default()
};
}
pub const MAINNET_CHAIN_ID: u64 = 0x1;
pub const MAINNET_SPEC_ID: SpecId = SpecId::CANCUN;

// RPC related constants

pub const RPC_RATE_LIMIT: usize = 100; // requests per second
