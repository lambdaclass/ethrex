//! Build script for the contract deployer binary.
//! This script downloads dependencies and compiles contracts to be embedded as constants in the SDK.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::env;
use std::path::Path;

fn main() {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let contracts_path = Path::new(&out_dir).join("contracts");

    download_contract_deps(&contracts_path);

    // Compile the ERC1967Proxy contract;
    compile_contract_to_bytecode(
        &contracts_path,
        "lib/openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts/contracts/proxy/ERC1967",
        "ERC1967Proxy",
        None,
    );

    // Compile the SP1VerifierGroth16 contract
    compile_contract_to_bytecode(
        &contracts_path,
        "lib/sp1-contracts/contracts/src/v5.0.0",
        "SP1VerifierGroth16",
        None,
    );

    // Get the openzeppelin contracts remappings
    let remappings = vec![
        (
            "@openzeppelin/contracts",
            &contracts_path.join(
                "lib/openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts/contracts",
            ),
        ),
        (
            "@openzeppelin/contracts-upgradeable",
            &contracts_path.join("lib/openzeppelin-contracts-upgradeable/contracts"),
        ),
    ];

    compile_contract_to_bytecode(
        &contracts_path,
        "src/l1",
        "OnChainProposer",
        Some(&remappings),
    );
    compile_contract_to_bytecode(&contracts_path, "src/l1", "CommonBridge", Some(&remappings));

    // Compile based contracts
    // Based OnChainProposer is renamed to OnChainProposerBased
    compile_contract_to_bytecode(
        &contracts_path,
        "src/l1/based",
        "SequencerRegistry",
        Some(&remappings),
    );
    ethrex_l2_sdk::compile_contract(
        &contracts_path,
        "src/l1/based/OnChainProposer.sol",
        false,
        Some(&remappings),
    )
    .unwrap();
    let contract_bytecode_hex = std::fs::read_to_string(
        contracts_path
            .join("solc_out")
            .join(format!("OnChainProposer.bin")),
    )
    .unwrap();
    let contract_bytecode = hex::decode(contract_bytecode_hex.trim()).unwrap();

    std::fs::write(
        contracts_path
            .join("solc_out")
            .join(format!("OnChainProposerBased.bytecode",)),
        contract_bytecode,
    )
    .unwrap();

    println!("cargo::rerun-if-changed=build.rs");
}

/// Clones OpenZeppelin and SP1 contracts into the specified path.
fn download_contract_deps(contracts_path: &Path) {
    std::fs::create_dir_all(contracts_path.join("lib")).unwrap();

    ethrex_l2_sdk::git_clone(
        "https://github.com/OpenZeppelin/openzeppelin-contracts-upgradeable.git",
        contracts_path
            .join("lib/openzeppelin-contracts-upgradeable")
            .to_str()
            .unwrap(),
        None,
        true,
    )
    .unwrap();

    ethrex_l2_sdk::git_clone(
        "https://github.com/succinctlabs/sp1-contracts.git",
        contracts_path.join("lib/sp1-contracts").to_str().unwrap(),
        None,
        false,
    )
    .unwrap();
}

fn compile_contract_to_bytecode(
    general_contracts_path: &Path,
    contract_path: &str,
    contract_name: &str,
    remappings: Option<&[(&str, &Path)]>,
) {
    ethrex_l2_sdk::compile_contract(
        general_contracts_path,
        &format!("{contract_path}/{contract_name}.sol"),
        false,
        remappings,
    )
    .unwrap();
    decode_to_bytecode(&general_contracts_path, contract_name);
    println!("Successfully compiled {contract_name} contract");
}

fn decode_to_bytecode(contracts_path: &Path, contract: &str) {
    let contract_bytecode_hex = std::fs::read_to_string(
        contracts_path
            .join("solc_out")
            .join(format!("{contract}.bin")),
    )
    .unwrap();
    let contract_bytecode = hex::decode(contract_bytecode_hex.trim()).unwrap();

    std::fs::write(
        contracts_path
            .join("solc_out")
            .join(format!("{contract}.bytecode",)),
        contract_bytecode,
    )
    .unwrap();
}
