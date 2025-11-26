//! Build script for the L2 SDK crate.
//! This script downloads dependencies and compiles contracts to be embedded as constants in the SDK.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use ethrex_sdk_contract_utils::git_clone;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR not set"));
    let contracts_path = out_dir.join("contracts");
    fs::create_dir_all(contracts_path.join("lib")).expect("failed to create contracts/lib");

    let oz_target = contracts_path.join("lib/openzeppelin-contracts-upgradeable");
    let oz_upgradable_env_path = env::var_os("ETHREX_SDK_OZ_UPGRADABLE_CONTRACTS_DIR")
        .map(PathBuf::from)
        .filter(|path| path.exists());
    let oz_env_path = env::var_os("ETHREX_SDK_OZ_CONTRACTS_DIR").map(PathBuf::from);
    let oz_source_root = oz_upgradable_env_path.clone().unwrap_or_else(|| {
        clone_openzeppelin(&oz_target);
        oz_target.clone()
    });

    let upgradeable_primary =
        oz_source_root.join("lib/openzeppelin-contracts/contracts/proxy/ERC1967/ERC1967Proxy.sol");
    let upgradeable_fallback = oz_source_root.join(
        "lib/openzeppelin-contracts/contracts/contracts/proxy/ERC1967/ERC1967Proxy.sol",
    );
    let proxy_contract_path = if upgradeable_primary.exists() {
        upgradeable_primary
    } else if upgradeable_fallback.exists() {
        upgradeable_fallback
    } else {
        panic!(
            "ERC1967Proxy.sol not found at {} (primary) or {} (fallback)",
            upgradeable_primary.display(),
            upgradeable_fallback.display()
        );
    };

    let mut allow_paths: Vec<&Path> = vec![contracts_path.as_path(), oz_source_root.as_path()];
    if let Some(pre_fetched) = oz_upgradable_env_path.as_deref() {
        allow_paths.push(pre_fetched);
    }
    if let Some(std_root) = oz_env_path.as_deref() {
        allow_paths.push(std_root);
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
