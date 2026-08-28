//! Fork-name handling: map a t8n `--state.fork` name to an ethrex [`Fork`]
//! and synthesize a [`ChainConfig`] with every fork up to and including it
//! activated at genesis.

use ethrex_common::constants::MAINNET_DEPOSIT_CONTRACT_ADDRESS;
use ethrex_common::types::{BlobSchedule, ChainConfig, Fork};

use super::error::T8nError;

/// Fork names accepted by `--state.fork`, in activation order.
///
/// Only post-merge forks are supported: pre-merge transitions need ethash
/// difficulty and block rewards, which this tool does not implement.
pub const SUPPORTED_FORKS: &[(&str, Fork)] = &[
    // Transition tools conventionally call Paris `Merge`; `Paris` is
    // accepted as an alias in `parse_fork`.
    ("Merge", Fork::Paris),
    ("Shanghai", Fork::Shanghai),
    ("Cancun", Fork::Cancun),
    ("Prague", Fork::Prague),
    ("Osaka", Fork::Osaka),
    ("BPO1", Fork::BPO1),
    ("BPO2", Fork::BPO2),
    ("BPO3", Fork::BPO3),
    ("BPO4", Fork::BPO4),
    ("BPO5", Fork::BPO5),
    ("Amsterdam", Fork::Amsterdam),
];

/// Clap `after_help` text advertising the supported forks; test fillers
/// probe `t8n --help` for a fork name to decide whether it can be filled.
pub fn supported_forks_help() -> String {
    let names: Vec<&str> = SUPPORTED_FORKS.iter().map(|(name, _)| *name).collect();
    format!("Supported forks: {}", names.join(", "))
}

/// Resolve a fork name to an ethrex [`Fork`]. `Paris` is accepted as an
/// alias for `Merge`, its conventional transition-tool name.
pub fn parse_fork(name: &str) -> Result<Fork, T8nError> {
    if name == "Paris" {
        return Ok(Fork::Paris);
    }
    SUPPORTED_FORKS
        .iter()
        .find(|(fork_name, _)| *fork_name == name)
        .map(|(_, fork)| *fork)
        .ok_or_else(|| T8nError::UnsupportedFork(name.to_string()))
}

/// Build a [`ChainConfig`] in which every fork up to and including `fork`
/// activates at genesis, so any block timestamp executes under `fork`.
/// Blob parameters come from the canonical per-fork [`BlobSchedule`].
pub fn chain_config_for(fork: Fork, chain_id: u64) -> ChainConfig {
    let mut config = ChainConfig {
        chain_id,
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
        terminal_total_difficulty_passed: true,
        blob_schedule: BlobSchedule::default(),
        // EIP-6110 deposit request extraction filters receipt logs by this
        // address; tests deploy the deposit contract at its mainnet address.
        deposit_contract_address: MAINNET_DEPOSIT_CONTRACT_ADDRESS,
        ..Default::default()
    };
    if fork >= Fork::Shanghai {
        config.shanghai_time = Some(0);
    }
    if fork >= Fork::Cancun {
        config.cancun_time = Some(0);
    }
    if fork >= Fork::Prague {
        config.prague_time = Some(0);
    }
    if fork >= Fork::Osaka {
        config.osaka_time = Some(0);
    }
    if fork >= Fork::BPO1 {
        config.bpo1_time = Some(0);
    }
    if fork >= Fork::BPO2 {
        config.bpo2_time = Some(0);
    }
    if fork >= Fork::BPO3 {
        config.bpo3_time = Some(0);
    }
    if fork >= Fork::BPO4 {
        config.bpo4_time = Some(0);
    }
    if fork >= Fork::BPO5 {
        config.bpo5_time = Some(0);
    }
    if fork >= Fork::Amsterdam {
        config.amsterdam_time = Some(0);
    }
    config
}
