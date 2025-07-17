//! Build script for the L2 SDK crate.
//! This script downloads dependencies and compiles contracts to be embedded as constants in the SDK.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::env;
use std::path::Path;

fn main() {
    println!("cargo::rerun-if-env-changed=COMPILE_CONTRACTS");
    println!("cargo::rerun-if-changed=build.rs");

    let out_dir = env::var_os("OUT_DIR").unwrap();
    let contracts_path = Path::new(&out_dir).join("contracts");

    // If COMPILE_CONTRACTS is not set, skip the compilation step.
    if env::var_os("COMPILE_CONTRACTS").is_none() {
        // Write an empty bytecode file to indicate that contracts are not compiled.
        std::fs::create_dir_all(contracts_path.join("solc_out"))
            .expect("failed to create contracts output directory");
        std::fs::write(contracts_path.join("solc_out/ERC1967Proxy.bytecode"), [])
            .expect("failed to write ERC1967Proxy bytecode");
        return;
    }

    ethrex_sdk_contract_utils::download_contract_deps(&contracts_path)
        .expect("failed to download contract dependencies");

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
}
