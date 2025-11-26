//! Build script for the L2 SDK crate.
//! This script downloads dependencies and compiles contracts to be embedded as constants in the SDK.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR not set"));
    let contracts_path = out_dir.join("contracts");
    fs::create_dir_all(contracts_path.join("lib")).expect("failed to create contracts/lib");

    let openzeppelin_contracts_root = PathBuf::from(
        env::var_os("ETHREX_SDK_OZ_CONTRACTS_DIR")
            .expect("ETHREX_SDK_OZ_CONTRACTS_DIR must be set for contracts"),
    );
    let openzeppelin_contracts_primary =
        openzeppelin_contracts_root.join("contracts/proxy/ERC1967/ERC1967Proxy.sol");
    let openzeppelin_contracts_fallback =
        openzeppelin_contracts_root.join("contracts/contracts/proxy/ERC1967/ERC1967Proxy.sol");
    let proxy_contract_path = if openzeppelin_contracts_primary.exists() {
        openzeppelin_contracts_primary
    } else if openzeppelin_contracts_fallback.exists() {
        openzeppelin_contracts_fallback
    } else {
        panic!(
            "ERC1967Proxy.sol not found at {} (primary) or {} (fallback)",
            openzeppelin_contracts_primary.display(),
            openzeppelin_contracts_fallback.display()
        );
    };

    let allow_paths: Vec<&Path> = vec![contracts_path.as_path(), openzeppelin_contracts_root.as_path()];
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
