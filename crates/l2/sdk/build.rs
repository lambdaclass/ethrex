//! Build script for the L2 SDK crate.
//! This script downloads dependencies and compiles contracts to be embedded as constants in the SDK.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::{env, fs, path::PathBuf};

use ethrex_sdk_contract_utils::git_clone;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    let contracts_path =
        PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR not set")).join("contracts");
    fs::create_dir_all(contracts_path.join("lib")).expect("failed to create contracts/lib");

    let openzeppelin_contracts_root = env::var_os("ETHREX_SDK_OZ_CONTRACTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let target = contracts_path.join("lib/openzeppelin-contracts");
            git_clone(
                "https://github.com/OpenZeppelin/openzeppelin-contracts.git",
                target.to_str().expect("failed to convert git target path"),
                Some("release-v5.4"),
                true,
            )
            .expect("failed to clone openzeppelin-contracts repo");
            target
        });

    let proxy_contract_path =
        openzeppelin_contracts_root.join("contracts/proxy/ERC1967/ERC1967Proxy.sol");
    assert!(
        proxy_contract_path.exists(),
        "ERC1967Proxy.sol not found at {}; try using the contracts/contracts/ path if needed",
        proxy_contract_path.display()
    );

    let allow_paths = vec![
        contracts_path.as_path(),
        openzeppelin_contracts_root.as_path(),
    ];
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
