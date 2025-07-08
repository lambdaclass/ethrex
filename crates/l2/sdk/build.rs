//! Build script for the L2 SDK crate.
//! This script downloads dependencies and compiles contracts to be embedded as constants in the SDK.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let contracts_path = Path::new(&out_dir).join("contracts");

    get_contract_dependencies(&contracts_path);

    // Compile the ERC1967Proxy contract
    let proxy_contract_path = "lib/openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts/contracts/proxy/ERC1967/ERC1967Proxy.sol";
    ethrex_sdk_contract_utils::compile_contract(&contracts_path, proxy_contract_path, false)
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

    println!("cargo::rerun-if-changed=build.rs");
}

/// Get contract dependencies.
/// If the `CONTRACTS_PATH` environment variable is set, copy the contracts from that path.
/// Otherwise, download the dependencies using `ethrex_sdk_contract_utils`.
/// This is needed because we are unable to run `git clone` when building the SDK in a TEE environment.
fn get_contract_dependencies(contracts_path: &Path) {
    if let Some(source_path) = env::var_os("CONTRACTS_PATH") {
        std::fs::create_dir_all(contracts_path)
            .expect("failed to create contracts output directory");

        let status = Command::new("cp")
            .args([
                "-r",
                &source_path.to_string_lossy(),
                &contracts_path.to_string_lossy(),
            ])
            .status()
            .expect("failed to run cp -r");

        if !status.success() {
            eprintln!("`cp` command failed with status: {}", status);
            std::process::exit(1);
        }
    } else {
        ethrex_sdk_contract_utils::download_contract_deps(contracts_path)
            .expect("failed to download contract dependencies");
    }
}
