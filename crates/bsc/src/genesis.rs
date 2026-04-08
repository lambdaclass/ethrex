use ethrex_common::types::{ChainConfig, Genesis};

/// BSC mainnet chain ID.
pub const BSC_MAINNET_CHAIN_ID: u64 = 56;
/// BSC Chapel testnet chain ID.
pub const BSC_CHAPEL_CHAIN_ID: u64 = 97;

/// Returns a minimal Genesis for BSC mainnet (chain ID 56).
///
/// Stub: no allocs — just the chain config. Full genesis alloc will be
/// added when the embedded genesis JSON is available.
pub fn bsc_mainnet_genesis() -> Genesis {
    Genesis {
        config: bsc_mainnet_chain_config(),
        ..Default::default()
    }
}

/// Returns a minimal Genesis for BSC Chapel testnet (chain ID 97).
///
/// Stub: no allocs — just the chain config. Full genesis alloc will be
/// added when the embedded genesis JSON is available.
pub fn bsc_chapel_genesis() -> Genesis {
    Genesis {
        config: bsc_chapel_chain_config(),
        ..Default::default()
    }
}

/// Returns true if the chain ID is a known BSC network.
pub fn is_bsc_chain(chain_id: u64) -> bool {
    chain_id == BSC_MAINNET_CHAIN_ID || chain_id == BSC_CHAPEL_CHAIN_ID
}

/// Returns the Genesis for a given chain ID, if it's a known BSC network.
///
/// Stub: returns None for all chains until genesis data is fully populated.
pub fn genesis_for_chain(_chain_id: u64) -> Option<Genesis> {
    // TODO: add embedded genesis alloc JSON and chain configs for mainnet and Chapel.
    None
}

/// ChainConfig for BSC mainnet.
///
/// All EVM forks (up to Prague) are considered active.
/// BSC-specific fork schedule will be tracked via ParliaConfig.
pub fn bsc_mainnet_chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: BSC_MAINNET_CHAIN_ID,
        homestead_block: Some(0),
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
        shanghai_time: Some(0),
        cancun_time: Some(0),
        prague_time: Some(0),
        ..Default::default()
    }
}

/// ChainConfig for BSC Chapel testnet.
pub fn bsc_chapel_chain_config() -> ChainConfig {
    ChainConfig {
        chain_id: BSC_CHAPEL_CHAIN_ID,
        homestead_block: Some(0),
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
        shanghai_time: Some(0),
        cancun_time: Some(0),
        prague_time: Some(0),
        ..Default::default()
    }
}
