//! Build script for the L2 SDK crate.
//! This script downloads dependencies and compiles contracts to be embedded as constants in the SDK.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::env;
use std::fs;
use std::io;
use std::path::Path;

use ethrex_sdk_contract_utils::git_clone;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    let out_dir = env::var_os("OUT_DIR").unwrap();
    let contracts_path = Path::new(&out_dir).join("contracts");
    std::fs::create_dir_all(contracts_path.join("lib")).expect("Failed to create contracts/lib");

    let oz_target = contracts_path.join("lib/openzeppelin-contracts-upgradeable");
    if let Ok(pre_fetched_path) = env::var("ETHREX_SDK_OPENZEPPELIN_DIR") {
        let pre_fetched = Path::new(&pre_fetched_path);
        if oz_target.exists() {
            fs::remove_dir_all(&oz_target).expect("Failed to clear existing OpenZeppelin snapshot");
        }
        copy_dir_all(pre_fetched, &oz_target)
            .expect("Failed to copy OpenZeppelin snapshot into build output");
    } else {
        git_clone(
            "https://github.com/OpenZeppelin/openzeppelin-contracts-upgradeable.git",
            oz_target.to_str().expect("Failed to convert path to str"),
            Some("release-v5.4"),
            true,
        )
        .unwrap();
    }

    // Compile the ERC1967Proxy contract
    let proxy_contract_path = contracts_path.join("lib/openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts/contracts/proxy/ERC1967/ERC1967Proxy.sol");
    ethrex_sdk_contract_utils::compile_contract(
        &contracts_path,
        &proxy_contract_path,
        false,
        false,
        None,
        &[&contracts_path],
    )
    .expect("failed to compile ERC1967Proxy contract");

    let contract_bytecode_hex =
        std::fs::read_to_string(contracts_path.join("solc_out/ERC1967Proxy.bin"))
            .expect("failed to read ERC1967Proxy bytecode");
    let contract_bytecode = hex::decode(contract_bytecode_hex.trim())
        .expect("failed to hex-decode ERC1967Proxy bytecode");

    std::fs::write(
        contracts_path.join("solc_out/ERC1967Proxy.bytecode"),
        contract_bytecode,
    )
    .expect("failed to write ERC1967Proxy bytecode");
}

fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else if ty.is_file() {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
