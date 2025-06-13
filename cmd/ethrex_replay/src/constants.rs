// Blockchain related constants

use std::str::FromStr;

use ethrex_common::{
    H160,
    types::{BlobSchedule, ChainConfig},
};

// Chain config for different forks as defined on https://ethereum.github.io/execution-spec-tests/v3.0.0/consuming_tests/common_types/#fork
pub fn make_chainconfig(chain_id: u64) -> ChainConfig {
    ChainConfig {
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
        shanghai_time: Some(0),
        cancun_time: Some(0),
        prague_time: Some(0),
        terminal_total_difficulty_passed: false,
        verkle_time: None,
        blob_schedule: BlobSchedule::default(),
        // Mainnet address
        deposit_contract_address: H160::from_str("0x00000000219ab540356cbb839cbe05303d7705fa")
            .expect("Invalid deposit contract address"),
    }
}

pub fn get_chain_config(name: &str) -> eyre::Result<ChainConfig> {
    Ok(match name {
        "mainnet" => make_chainconfig(1),
        "cancun" => ChainConfig {
            prague_time: None,
            ..make_chainconfig(1)
        },
        "holesky" => make_chainconfig(17000),
        "hoodi" => make_chainconfig(560048),
        "sepolia" => make_chainconfig(11155111),
        _ => {
            if let Ok(num) = u64::from_str(name) {
                make_chainconfig(num)
            } else {
                return Err(eyre::Error::msg("Network name not known."));
            }
        }
    })
}
