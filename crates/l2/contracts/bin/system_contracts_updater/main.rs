use std::{collections::HashMap, path::Path, path::PathBuf};

use bytes::Bytes;
use clap::Parser;
use cli::SystemContractsUpdaterOptions;
use error::SystemContractsUpdaterError;
use ethrex_common::U256;
use ethrex_common::types::GenesisAccount;
use ethrex_l2::utils::test_data_io::read_genesis_file;
use ethrex_l2_sdk::{COMMON_BRIDGE_L2_ADDRESS, compile_contract};
use genesis_tool::genesis::write_genesis_as_json;
mod cli;
mod error;

fn main() -> Result<(), SystemContractsUpdaterError> {
    let opts = SystemContractsUpdaterOptions::parse();
    compile_contract(&opts.contracts_path, "src/l2/CommonBridgeL2.sol", true)?;
    update_genesis_file(&opts.l2_genesis_path)?;
    Ok(())
}

fn update_genesis_file(l2_genesis_path: &PathBuf) -> Result<(), SystemContractsUpdaterError> {
    let mut genesis = read_genesis_file(l2_genesis_path.to_str().ok_or(
        SystemContractsUpdaterError::InvalidPath(
            "Failed to convert l2 genesis path to string".to_string(),
        ),
    )?);

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

    write_genesis_as_json(genesis, Path::new(l2_genesis_path)).map_err(std::io::Error::other)?;

    println!("Updated L2 genesis file.");

    Ok(())
}
