use std::collections::HashMap;

use bytes::Bytes;
use clap::Parser;
use cli::SystemContractsUpdaterOptions;
use error::SystemContractsUpdaterError;
use ethrex_common::types::GenesisAccount;
use ethrex_common::U256;
use ethrex_l2::utils::test_data_io::read_genesis_file;
use ethrex_l2_sdk::{compile_contract, COMMON_BRIDGE_L2_ADDRESS};

mod cli;
mod error;

fn main() -> Result<(), SystemContractsUpdaterError> {
    let opts = SystemContractsUpdaterOptions::parse();
    compile_contract(&opts.contracts_path, "src/l2/CommonBridgeL2.sol", true)?;
    update_genesis_file(&opts)?;
    Ok(())
}

fn update_genesis_file(
    opts: &SystemContractsUpdaterOptions,
) -> Result<(), SystemContractsUpdaterError> {
    let mut genesis = read_genesis_file(&opts.genesis_l1_path);

    let runtime_code = std::fs::read(&opts.contracts_path)?;

    genesis.alloc.insert(
        COMMON_BRIDGE_L2_ADDRESS,
        GenesisAccount {
            code: Bytes::from(hex::decode(runtime_code)?),
            storage: HashMap::new(),
            balance: U256::zero(),
            nonce: 1,
        },
    );

    let modified_genesis = serde_json::to_string(&genesis)?;

    std::fs::write(&opts.genesis_l1_path, modified_genesis)?;

    Ok(())
}
