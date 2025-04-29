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
    update_genesis_file(&opts.l2_genesis_path)?;
    Ok(())
}

fn update_genesis_file(l2_genesis_path: &str) -> Result<(), SystemContractsUpdaterError> {
    let mut genesis = read_genesis_file(l2_genesis_path);

    let runtime_code = std::fs::read("contracts/solc_out/CommonBridgeL2.bin-runtime")?;

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

    std::fs::write(l2_genesis_path, modified_genesis)?;

    println!("Updated L2 genesis file.");

    Ok(())
}
