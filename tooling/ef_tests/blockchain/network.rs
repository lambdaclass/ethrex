use ethrex_common::{H160, types::ChainConfig};
use lazy_static::lazy_static;
use serde::Deserialize;
use std::str::FromStr;

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
    pub static ref CANCUN_TO_PRAGUE_AT_15K_CONFIG: ChainConfig = ChainConfig {
        prague_time: Some(0x3a98),
        // Mainnet address
        deposit_contract_address: H160::from_str("0x00000000219ab540356cbb839cbe05303d7705fa")
            .unwrap(),
        ..*CANCUN_CONFIG
    };
    pub static ref PRAGUE_CONFIG: ChainConfig = ChainConfig {
        prague_time: Some(0),
        ..*CANCUN_TO_PRAGUE_AT_15K_CONFIG
    };
}

#[derive(Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Network {
    Frontier = 0,          // For parsing tests
    Homestead = 1,         // For parsing tests
    ConstantinopleFix = 2, // For parsing tests
    Istanbul = 3,          // For parsing tests
    Byzantium = 4,         // For parsing tests
    London = 5,            // For parsing tests
    Berlin = 6,            // For parsing tests
    #[serde(alias = "Paris")]
    Merge = 7,
    #[serde(alias = "ParisToShanghaiAtTime15k")]
    MergeToShanghaiAtTime15k = 8,
    Shanghai = 9,
    ShanghaiToCancunAtTime15k = 10,
    Cancun = 11,
    CancunToPragueAtTime15k = 12,
    Prague = 13,
}

impl Network {
    pub fn chain_config(&self) -> &ChainConfig {
        match self {
            Network::Merge => &MERGE_CONFIG,
            Network::MergeToShanghaiAtTime15k => &MERGE_TO_SHANGHAI_AT_15K_CONFIG,
            Network::Shanghai => &SHANGHAI_CONFIG,
            Network::ShanghaiToCancunAtTime15k => &SHANGHAI_TO_CANCUN_AT_15K_CONFIG,
            Network::Cancun => &CANCUN_CONFIG,
            Network::CancunToPragueAtTime15k => &CANCUN_TO_PRAGUE_AT_15K_CONFIG,
            Network::Prague => &PRAGUE_CONFIG,
            Network::Frontier
            | Network::Homestead
            | Network::ConstantinopleFix
            | Network::Istanbul
            | Network::Byzantium
            | Network::London
            | Network::Berlin => {
                panic!("Ethrex doesn't support pre-Merge forks: {self:?}")
            }
        }
    }
}
