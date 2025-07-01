use std::{collections::HashMap, path::Path, path::PathBuf};

use bytes::Bytes;
use clap::Parser;
use cli::SystemContractsUpdaterOptions;
use error::SystemContractsUpdaterError;
use ethrex_common::types::{Genesis, GenesisAccount};
use ethrex_common::{Address, H160, U256};
use ethrex_l2::utils::test_data_io::read_genesis_file;
use ethrex_l2_sdk::{
    COMMON_BRIDGE_L2_ADDRESS, L2_TO_L1_MESSENGER_ADDRESS, address_to_word, compile_contract,
    download_contract_deps,
};
use genesis_tool::genesis::write_genesis_as_json;
use keccak_hash::keccak;
mod cli;
mod error;

pub const ADMIN_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xf0, 0x00,
]);

pub const IMPL_MASK: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x10, 0x00,
]);

fn main() -> Result<(), SystemContractsUpdaterError> {
    let opts = SystemContractsUpdaterOptions::parse();
    download_contract_deps(&opts.contracts_path)
        .map_err(|e| SystemContractsUpdaterError::FailedToDownloadDependencies(e.to_string()))?;
    compile_contract(&opts.contracts_path, "src/l2/L2Upgradeable.sol", true)?;
    compile_contract(&opts.contracts_path, "src/l2/CommonBridgeL2.sol", true)?;
    compile_contract(&opts.contracts_path, "src/l2/L2ToL1Messenger.sol", true)?;
    update_genesis_file(&opts.l2_genesis_path)?;
    Ok(())
}

fn get_erc1967_slot(name: &str) -> U256 {
    U256::from_big_endian(&keccak(name).0) - U256::one()
}

fn add_with_proxy(
    genesis: &mut Genesis,
    address: Address,
    code: Vec<u8>,
) -> Result<(), SystemContractsUpdaterError> {
    let impl_address = address ^ IMPL_MASK;
    genesis.alloc.insert(
        impl_address,
        GenesisAccount {
            code: Bytes::from(hex::decode(code)?),
            storage: HashMap::new(),
            balance: U256::zero(),
            nonce: 1,
        },
    );

    let proxy_code = std::fs::read("contracts/solc_out/UpgradeableSystemContract.bin-runtime")?;
    let mut storage = HashMap::new();
    storage.insert(
        get_erc1967_slot("eip1967.proxy.implementation"),
        address_to_word(impl_address),
    );
    storage.insert(
        get_erc1967_slot("eip1967.proxy.admin"),
        address_to_word(ADMIN_ADDRESS),
    );
    genesis.alloc.insert(
        address,
        GenesisAccount {
            code: Bytes::from(hex::decode(proxy_code)?),
            storage,
            balance: U256::zero(),
            nonce: 1,
        },
    );
    Ok(())
}

fn update_genesis_file(l2_genesis_path: &PathBuf) -> Result<(), SystemContractsUpdaterError> {
    let mut genesis = read_genesis_file(l2_genesis_path.to_str().ok_or(
        SystemContractsUpdaterError::InvalidPath(
            "Failed to convert l2 genesis path to string".to_string(),
        ),
    )?);

    let l2_bridge_runtime = std::fs::read("contracts/solc_out/CommonBridgeL2.bin-runtime")?;
    add_with_proxy(&mut genesis, COMMON_BRIDGE_L2_ADDRESS, l2_bridge_runtime)?;

    let l1_messenger_runtime = std::fs::read("contracts/solc_out/L2ToL1Messenger.bin-runtime")?;
    add_with_proxy(
        &mut genesis,
        L2_TO_L1_MESSENGER_ADDRESS,
        l1_messenger_runtime,
    )?;

    write_genesis_as_json(genesis, Path::new(l2_genesis_path)).map_err(std::io::Error::other)?;

    println!("Updated L2 genesis file.");

    Ok(())
}
