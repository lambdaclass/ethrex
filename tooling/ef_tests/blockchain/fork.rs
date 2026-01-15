use ethrex_common::{H160, types::ChainConfig};
use lazy_static::lazy_static;
use serde::Deserialize;
use std::str::FromStr;

// Chain config for different forks as defined on https://ethereum.github.io/execution-spec-tests/v3.0.0/consuming_tests/common_types/#fork
lazy_static! {
    // ============================================================
    // Pre-merge (PoW) chain configs
    // ============================================================

    /// Frontier - Genesis fork (no special features)
    pub static ref FRONTIER_CONFIG: ChainConfig = ChainConfig {
        chain_id: 1_u64,
        ..Default::default()
    };

    /// Frontier to Homestead transition at block 5
    pub static ref FRONTIER_TO_HOMESTEAD_AT_5_CONFIG: ChainConfig = ChainConfig {
        homestead_block: Some(5),
        ..*FRONTIER_CONFIG
    };

    /// Homestead - EIP-2 (DELEGATECALL), EIP-7, EIP-8
    pub static ref HOMESTEAD_CONFIG: ChainConfig = ChainConfig {
        homestead_block: Some(0),
        ..*FRONTIER_CONFIG
    };

    /// Homestead to DAO fork transition at block 5
    pub static ref HOMESTEAD_TO_DAO_AT_5_CONFIG: ChainConfig = ChainConfig {
        dao_fork_block: Some(5),
        dao_fork_support: true,
        ..*HOMESTEAD_CONFIG
    };

    /// Homestead to EIP150 transition at block 5
    pub static ref HOMESTEAD_TO_EIP150_AT_5_CONFIG: ChainConfig = ChainConfig {
        eip150_block: Some(5),
        ..*HOMESTEAD_CONFIG
    };

    /// EIP150 (Tangerine Whistle) - Gas cost changes for IO-heavy operations
    pub static ref EIP150_CONFIG: ChainConfig = ChainConfig {
        eip150_block: Some(0),
        ..*HOMESTEAD_CONFIG
    };

    /// EIP158 (Spurious Dragon) - State clearing, replay protection
    pub static ref EIP158_CONFIG: ChainConfig = ChainConfig {
        eip155_block: Some(0),
        eip158_block: Some(0),
        ..*EIP150_CONFIG
    };

    /// EIP158 to Byzantium transition at block 5
    pub static ref EIP158_TO_BYZANTIUM_AT_5_CONFIG: ChainConfig = ChainConfig {
        byzantium_block: Some(5),
        ..*EIP158_CONFIG
    };

    /// Byzantium - REVERT, RETURNDATASIZE/COPY, STATICCALL, precompiles
    pub static ref BYZANTIUM_CONFIG: ChainConfig = ChainConfig {
        byzantium_block: Some(0),
        ..*EIP158_CONFIG
    };

    /// Byzantium to ConstantinopleFix transition at block 5
    pub static ref BYZANTIUM_TO_CONSTANTINOPLE_FIX_AT_5_CONFIG: ChainConfig = ChainConfig {
        constantinople_block: Some(5),
        petersburg_block: Some(5),
        ..*BYZANTIUM_CONFIG
    };

    /// Constantinople - Bitwise shifts, CREATE2, EXTCODEHASH
    /// Note: Petersburg follows immediately (Constantinople had a bug)
    pub static ref CONSTANTINOPLE_CONFIG: ChainConfig = ChainConfig {
        constantinople_block: Some(0),
        ..*BYZANTIUM_CONFIG
    };

    /// ConstantinopleFix (Petersburg) - Constantinople with EIP-1283 removed
    pub static ref CONSTANTINOPLE_FIX_CONFIG: ChainConfig = ChainConfig {
        petersburg_block: Some(0),
        ..*CONSTANTINOPLE_CONFIG
    };

    /// Istanbul - CHAINID, Blake2, gas repricing
    pub static ref ISTANBUL_CONFIG: ChainConfig = ChainConfig {
        istanbul_block: Some(0),
        ..*CONSTANTINOPLE_FIX_CONFIG
    };

    /// Berlin - Access lists, typed transactions
    pub static ref BERLIN_CONFIG: ChainConfig = ChainConfig {
        muir_glacier_block: Some(0),
        berlin_block: Some(0),
        ..*ISTANBUL_CONFIG
    };

    /// Berlin to London transition at block 5
    pub static ref BERLIN_TO_LONDON_AT_5_CONFIG: ChainConfig = ChainConfig {
        london_block: Some(5),
        ..*BERLIN_CONFIG
    };

    /// London - EIP-1559 fee market, BASEFEE opcode
    pub static ref LONDON_CONFIG: ChainConfig = ChainConfig {
        london_block: Some(0),
        ..*BERLIN_CONFIG
    };

    /// Arrow Glacier to Paris (Merge) transition at difficulty 0xC0000
    pub static ref ARROW_GLACIER_TO_PARIS_AT_DIFF_C0000_CONFIG: ChainConfig = ChainConfig {
        arrow_glacier_block: Some(0),
        gray_glacier_block: Some(0),
        // TTD triggers the merge - set high enough that it doesn't trigger at block 0
        terminal_total_difficulty: Some(0xC0000),
        ..*LONDON_CONFIG
    };

    // ============================================================
    // Post-merge (PoS) chain configs
    // ============================================================

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

    pub static ref PRAGUE_TO_OSAKA_AT_15K_CONFIG: ChainConfig = ChainConfig {
        osaka_time: Some(0x3a98),
        ..*PRAGUE_CONFIG

    };

    pub static ref OSAKA_CONFIG: ChainConfig = ChainConfig {
        osaka_time: Some(0),
        ..*PRAGUE_CONFIG
    };

    pub static ref OSAKA_TO_BPO1_AT_15K_CONFIG: ChainConfig = ChainConfig {
        bpo1_time: Some(0x3a98),
        ..*OSAKA_CONFIG
    };

    pub static ref BPO1_TO_BPO2_AT_15K_CONFIG: ChainConfig = ChainConfig {
        bpo1_time: Some(0),
        bpo2_time: Some(0x3a98),
        ..*OSAKA_CONFIG
    };

    pub static ref BPO2_TO_BPO3_AT_15K_CONFIG: ChainConfig = ChainConfig {
        bpo2_time: Some(0),
        bpo3_time: Some(0x3a98),
        ..*OSAKA_CONFIG
    };
    pub static ref BPO3_TO_BPO4_AT_15K_CONFIG: ChainConfig = ChainConfig {
        bpo3_time: Some(0),
        bpo4_time: Some(0x3a98),
        ..*OSAKA_CONFIG
    };
    pub static ref BPO4_TO_BPO5_AT_15K_CONFIG: ChainConfig = ChainConfig {
        bpo4_time: Some(0),
        bpo5_time: Some(0x3a98),
        ..*OSAKA_CONFIG
    };

}

/// Most of the fork variants are just for parsing the tests
/// It's important for the pre-merge forks to be before Paris because we make a comparison for executing post-merge forks only.
#[derive(Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fork {
    Constantinople,
    EIP150,
    EIP158,
    EIP158ToByzantiumAt5,
    ArrowGlacierToParisAtDiffC0000,
    BerlinToLondonAt5,
    ByzantiumToConstantinopleFixAt5,
    FrontierToHomesteadAt5,
    HomesteadToDaoAt5,
    HomesteadToEIP150At5,
    Frontier,
    Homestead,
    ConstantinopleFix,
    Istanbul,
    Byzantium,
    London,
    Berlin,
    #[serde(alias = "Paris")]
    Merge,
    #[serde(alias = "ParisToShanghaiAtTime15k")]
    MergeToShanghaiAtTime15k,
    Shanghai,
    ShanghaiToCancunAtTime15k,
    Cancun,
    CancunToPragueAtTime15k,
    Prague,
    PragueToOsakaAtTime15k,
    Osaka,
    OsakaToBPO1AtTime15k,
    BPO1ToBPO2AtTime15k,
    BPO2ToBPO3AtTime15k,
    BPO3ToBPO4AtTime15k,
    BPO4ToBPO5AtTime15k,
}

impl Fork {
    pub fn chain_config(&self) -> &ChainConfig {
        match self {
            // Pre-merge (PoW) forks
            Fork::Frontier => &FRONTIER_CONFIG,
            Fork::FrontierToHomesteadAt5 => &FRONTIER_TO_HOMESTEAD_AT_5_CONFIG,
            Fork::Homestead => &HOMESTEAD_CONFIG,
            Fork::HomesteadToDaoAt5 => &HOMESTEAD_TO_DAO_AT_5_CONFIG,
            Fork::HomesteadToEIP150At5 => &HOMESTEAD_TO_EIP150_AT_5_CONFIG,
            Fork::EIP150 => &EIP150_CONFIG,
            Fork::EIP158 => &EIP158_CONFIG,
            Fork::EIP158ToByzantiumAt5 => &EIP158_TO_BYZANTIUM_AT_5_CONFIG,
            Fork::Byzantium => &BYZANTIUM_CONFIG,
            Fork::ByzantiumToConstantinopleFixAt5 => &BYZANTIUM_TO_CONSTANTINOPLE_FIX_AT_5_CONFIG,
            Fork::Constantinople => &CONSTANTINOPLE_CONFIG,
            Fork::ConstantinopleFix => &CONSTANTINOPLE_FIX_CONFIG,
            Fork::Istanbul => &ISTANBUL_CONFIG,
            Fork::Berlin => &BERLIN_CONFIG,
            Fork::BerlinToLondonAt5 => &BERLIN_TO_LONDON_AT_5_CONFIG,
            Fork::London => &LONDON_CONFIG,
            Fork::ArrowGlacierToParisAtDiffC0000 => &ARROW_GLACIER_TO_PARIS_AT_DIFF_C0000_CONFIG,
            // Post-merge (PoS) forks
            Fork::Merge => &MERGE_CONFIG,
            Fork::MergeToShanghaiAtTime15k => &MERGE_TO_SHANGHAI_AT_15K_CONFIG,
            Fork::Shanghai => &SHANGHAI_CONFIG,
            Fork::ShanghaiToCancunAtTime15k => &SHANGHAI_TO_CANCUN_AT_15K_CONFIG,
            Fork::Cancun => &CANCUN_CONFIG,
            Fork::CancunToPragueAtTime15k => &CANCUN_TO_PRAGUE_AT_15K_CONFIG,
            Fork::Prague => &PRAGUE_CONFIG,
            Fork::PragueToOsakaAtTime15k => &PRAGUE_TO_OSAKA_AT_15K_CONFIG,
            Fork::Osaka => &OSAKA_CONFIG,
            Fork::OsakaToBPO1AtTime15k => &OSAKA_TO_BPO1_AT_15K_CONFIG,
            Fork::BPO1ToBPO2AtTime15k => &BPO1_TO_BPO2_AT_15K_CONFIG,
            Fork::BPO2ToBPO3AtTime15k => &BPO2_TO_BPO3_AT_15K_CONFIG,
            Fork::BPO3ToBPO4AtTime15k => &BPO3_TO_BPO4_AT_15K_CONFIG,
            Fork::BPO4ToBPO5AtTime15k => &BPO4_TO_BPO5_AT_15K_CONFIG,
        }
    }

    /// Returns true if this fork is pre-merge (PoW)
    pub fn is_pre_merge(&self) -> bool {
        matches!(
            self,
            Fork::Frontier
                | Fork::FrontierToHomesteadAt5
                | Fork::Homestead
                | Fork::HomesteadToDaoAt5
                | Fork::HomesteadToEIP150At5
                | Fork::EIP150
                | Fork::EIP158
                | Fork::EIP158ToByzantiumAt5
                | Fork::Byzantium
                | Fork::ByzantiumToConstantinopleFixAt5
                | Fork::Constantinople
                | Fork::ConstantinopleFix
                | Fork::Istanbul
                | Fork::Berlin
                | Fork::BerlinToLondonAt5
                | Fork::London
                | Fork::ArrowGlacierToParisAtDiffC0000
        )
    }
}
