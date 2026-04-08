use ethrex_common::types::Genesis;

/// BSC mainnet chain ID.
pub const BSC_MAINNET_CHAIN_ID: u64 = 56;
/// BSC Chapel testnet chain ID.
pub const BSC_CHAPEL_CHAIN_ID: u64 = 97;

/// Embedded Chapel testnet genesis JSON.
const CHAPEL_GENESIS_CONTENTS: &str = include_str!("../allocs/chapel_genesis.json");

/// Returns the Genesis for BSC Chapel testnet (chain ID 97).
pub fn bsc_chapel_genesis() -> Genesis {
    serde_json::from_str(CHAPEL_GENESIS_CONTENTS).expect("chapel genesis should be valid JSON")
}

/// Returns a minimal Genesis for BSC mainnet (chain ID 56).
///
/// Stub: no allocs — mainnet genesis alloc is very large.
/// For mainnet, users should provide a genesis file or use snapshot sync.
pub fn bsc_mainnet_genesis() -> Genesis {
    Genesis {
        config: bsc_mainnet_chain_config(),
        ..Default::default()
    }
}

/// Returns true if the chain ID is a known BSC network.
pub fn is_bsc_chain(chain_id: u64) -> bool {
    chain_id == BSC_MAINNET_CHAIN_ID || chain_id == BSC_CHAPEL_CHAIN_ID
}

/// ChainConfig for BSC mainnet.
///
/// All standard EVM forks are active from genesis (block 0 / timestamp 0).
/// BSC-specific forks are tracked via ParliaConfig.
pub fn bsc_mainnet_chain_config() -> ethrex_common::types::ChainConfig {
    ethrex_common::types::ChainConfig {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chapel_genesis_parses() {
        let genesis = bsc_chapel_genesis();
        assert_eq!(genesis.config.chain_id, 97);
        assert_eq!(genesis.alloc.len(), 13);
    }

    #[test]
    fn test_chapel_genesis_has_system_contracts() {
        let genesis = bsc_chapel_genesis();
        // ValidatorContract at 0x1000 should have code
        let validator_addr = "0x0000000000000000000000000000000000001000"
            .parse()
            .unwrap();
        let account = genesis.alloc.get(&validator_addr);
        assert!(account.is_some(), "ValidatorContract should be in alloc");
    }
}
