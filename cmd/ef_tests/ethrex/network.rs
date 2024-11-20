use ethrex_core::types::ChainConfig;
use lazy_static::lazy_static;
use serde::Deserialize;

// Chain config for different forks as defined on https://ethereum.github.io/execution-spec-tests/v3.0.0/consuming_tests/common_types/#fork
lazy_static! {
    pub static ref MERGE_CONFIG: ChainConfig = ChainConfig {
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
        ..Default::default()
    };
    pub static ref MERGE_TO_SHANGHAI_AT_15K_CONFIG: ChainConfig = ChainConfig {
        shanghai_time: Some(0x3a98),
        ..*MERGE_CONFIG
    };
    pub static ref SHANGHAI_CONFIG: ChainConfig = ChainConfig {
        shanghai_time: Some(0),
        ..*MERGE_CONFIG
    };
    pub static ref SHANGHAI_TO_CANCUN_AT_15K_CONFIG: ChainConfig = ChainConfig {
        cancun_time: Some(0x3a98),
        ..*SHANGHAI_CONFIG
    };
    pub static ref CANCUN_CONFIG: ChainConfig = ChainConfig {
        cancun_time: Some(0),
        ..*SHANGHAI_CONFIG
    };
}

#[derive(Debug, Deserialize)]
pub enum Network {
    #[serde(alias = "Paris")]
    Merge,
    #[serde(alias = "ParisToShanghaiAtTime15k")]
    MergeToShanghaiAtTime15k,
    Shanghai,
    ShanghaiToCancunAtTime15k,
    Cancun,
}

impl Network {
    pub fn chain_config(&self) -> &ChainConfig {
        match self {
            Network::Merge => &MERGE_CONFIG,
            Network::MergeToShanghaiAtTime15k => &MERGE_TO_SHANGHAI_AT_15K_CONFIG,
            Network::Shanghai => &SHANGHAI_CONFIG,
            Network::ShanghaiToCancunAtTime15k => &SHANGHAI_TO_CANCUN_AT_15K_CONFIG,
            Network::Cancun => &CANCUN_CONFIG,
        }
    }
}
