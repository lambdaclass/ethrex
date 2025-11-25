//! Build script for the L2 SDK crate.
//! This script downloads dependencies and compiles contracts to be embedded as constants in the SDK.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use ethrex_sdk_contract_utils::git_clone;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    let out_dir = env::var_os("OUT_DIR").unwrap();
    let contracts_path = Path::new(&out_dir).join("contracts");
    fs::create_dir_all(contracts_path.join("lib")).expect("Failed to create contracts/lib");

    let oz_target = contracts_path.join("lib/openzeppelin-contracts-upgradeable");
    let oz_env_path = env::var("ETHREX_SDK_OPENZEPPELIN_DIR").ok().map(PathBuf::from);
    let env_path_exists = oz_env_path
        .as_ref()
        .map(|path| path.exists())
        .unwrap_or(false);
    let oz_source_root = if env_path_exists {
        oz_env_path.as_ref().unwrap().clone()
    } else {
        clone_openzeppelin(&oz_target);
        oz_target.clone()
    };

    // Compile the ERC1967Proxy contract
    let proxy_contract_path = oz_source_root.join("lib/openzeppelin-contracts/contracts/proxy/ERC1967/ERC1967Proxy.sol");
    let mut allow_paths: Vec<&Path> = vec![contracts_path.as_path(), oz_source_root.as_path()];
    if env_path_exists {
        if let Some(pre_fetched) = oz_env_path.as_ref() {
            allow_paths.push(pre_fetched.as_path());
        }
    }
    ethrex_sdk_contract_utils::compile_contract(
        &contracts_path,
        &proxy_contract_path,
        false,
        false,
        None,
        &allow_paths,
    )
    .expect("failed to compile ERC1967Proxy contract");

    let contract_bytecode_hex =
        fs::read_to_string(contracts_path.join("solc_out/ERC1967Proxy.bin"))
            .expect("failed to read ERC1967Proxy bytecode");
    let contract_bytecode = hex::decode(contract_bytecode_hex.trim())
        .expect("failed to hex-decode ERC1967Proxy bytecode");

    fs::write(
        contracts_path.join("solc_out/ERC1967Proxy.bytecode"),
        contract_bytecode,
    )
    .expect("failed to write ERC1967Proxy bytecode");
}

fn clone_openzeppelin(target: &Path) {
    git_clone(
        "https://github.com/OpenZeppelin/openzeppelin-contracts-upgradeable.git",
        target.to_str().expect("Failed to convert path to str"),
        Some("release-v5.4"),
        true,
    )
    .unwrap();
}
