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
    fn test_chapel_genesis_bsc_specific_fields() {
        // Verify that BSC-specific fork fields are correctly deserialized from
        // the Chapel genesis JSON. These fields are required for correct fork ID
        // computation (EIP-2124).
        let genesis = bsc_chapel_genesis();
        let cfg = &genesis.config;

        // Block-number forks
        assert_eq!(cfg.ramanujan_block, Some(1_010_000), "ramanujanBlock");
        assert_eq!(cfg.niels_block, Some(1_014_369), "nielsBlock");
        assert_eq!(cfg.mirror_sync_block, Some(5_582_500), "mirrorSyncBlock");
        assert_eq!(cfg.bruno_block, Some(13_837_000), "brunoBlock");
        assert_eq!(cfg.euler_block, Some(19_203_503), "eulerBlock");
        assert_eq!(cfg.gibbs_block, Some(22_800_220), "gibbsBlock");
        assert_eq!(cfg.nano_block, Some(23_482_428), "nanoBlock");
        assert_eq!(cfg.moran_block, Some(23_603_940), "moranBlock");
        assert_eq!(cfg.planck_block, Some(28_196_022), "planckBlock");
        assert_eq!(cfg.luban_block, Some(29_295_050), "lubanBlock");
        assert_eq!(cfg.plato_block, Some(29_861_024), "platoBlock");
        assert_eq!(cfg.hertz_block, Some(31_103_030), "hertzBlock");
        assert_eq!(cfg.hertzfix_block, Some(35_682_300), "hertzfixBlock");

        // Timestamp forks
        assert_eq!(cfg.kepler_time, Some(1_702_972_800), "keplerTime");
        assert_eq!(cfg.feynman_time, Some(1_710_136_800), "feynmanTime");
        assert_eq!(cfg.feynman_fix_time, Some(1_711_342_800), "feynmanFixTime");
        assert_eq!(cfg.haber_time, Some(1_716_962_820), "haberTime");
        assert_eq!(cfg.haber_fix_time, Some(1_719_986_788), "haberFixTime");
        assert_eq!(cfg.bohr_time, Some(1_724_116_996), "bohrTime");
        assert_eq!(cfg.pascal_time, Some(1_740_452_880), "pascalTime");
        assert_eq!(cfg.lorentz_time, Some(1_744_097_580), "lorentzTime");
        assert_eq!(cfg.maxwell_time, Some(1_748_243_100), "maxwellTime");
        assert_eq!(cfg.fermi_time, Some(1_762_741_500), "fermiTime");
        assert_eq!(cfg.mendel_time, Some(1_774_319_400), "mendelTime");
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
