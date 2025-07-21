//! Build script for the contract deployer binary.
//! This script downloads dependencies and compiles contracts to be embedded as constants in the deployer.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=COMPILE_CONTRACTS");
    println!("cargo:rerun-if-changed=src");

    let out_dir = env::var_os("OUT_DIR").unwrap();
    let output_contracts_path = Path::new(&out_dir).join("contracts");

    // If COMPILE_CONTRACTS is not set, skip and write empty files
    if env::var_os("COMPILE_CONTRACTS").is_none() {
        write_empty_bytecode_files(&output_contracts_path);
        return;
    }

    download_contract_deps(&output_contracts_path);

    // ERC1967Proxy contract.
    compile_contract_to_bytecode(
        &output_contracts_path,
        &output_contracts_path.join("lib/openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts/contracts/proxy/ERC1967/ERC1967Proxy.sol"),
        "ERC1967Proxy",
        false,
        None,
    );

    // SP1VerifierGroth16 contract
    compile_contract_to_bytecode(
        &output_contracts_path,
        &output_contracts_path
            .join("lib/sp1-contracts/contracts/src/v5.0.0/SP1VerifierGroth16.sol"),
        "SP1Verifier",
        false,
        None,
    );

    // Get the openzeppelin contracts remappings
    let remappings = [
        (
            "@openzeppelin/contracts",
            output_contracts_path.join(
                "lib/openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts/contracts",
            ),
        ),
        (
            "@openzeppelin/contracts-upgradeable",
            output_contracts_path.join("lib/openzeppelin-contracts-upgradeable/contracts"),
        ),
    ];

    // L1 contracts
    let l1_contracts = [
        (&Path::new("src/l1/OnChainProposer.sol"), "OnChainProposer"),
        (&Path::new("src/l1/CommonBridge.sol"), "CommonBridge"),
    ];
    for (path, name) in l1_contracts {
        compile_contract_to_bytecode(&output_contracts_path, path, name, false, Some(&remappings));
    }
    // L2 contracts
    let l2_contracts = [
        (&Path::new("src/l2/CommonBridgeL2.sol"), "CommonBridgeL2"),
        (&Path::new("src/l2/L2ToL1Messenger.sol"), "L2ToL1Messenger"),
        (
            &Path::new("src/l2/L2Upgradeable.sol"),
            "UpgradeableSystemContract",
        ),
    ];
    for (path, name) in l2_contracts {
        compile_contract_to_bytecode(&output_contracts_path, path, name, true, Some(&remappings));
    }

    // Based contracts
    compile_contract_to_bytecode(
        &output_contracts_path,
        Path::new("src/l1/based/SequencerRegistry.sol"),
        "SequencerRegistry",
        false,
        Some(&remappings),
    );
    ethrex_l2_sdk::compile_contract(
        &output_contracts_path,
        Path::new("src/l1/based/OnChainProposer.sol"),
        false,
        Some(&remappings),
    )
    .unwrap();

    // To avoid colision with the original OnChainProposer bytecode, we rename it to OnChainProposerBased
    let file_path = output_contracts_path.join("solc_out/OnChainProposer.bin");
    let output_file_path = output_contracts_path.join("solc_out/OnChainProposerBased.bytecode");
    decode_to_bytecode(&file_path, &output_file_path);
}

fn write_empty_bytecode_files(output_contracts_path: &Path) {
    let bytecode_dir = output_contracts_path.join("solc_out");
    fs::create_dir_all(&bytecode_dir).expect("Failed to create solc_out directory");

    let contract_names = [
        "ERC1967Proxy",
        "SP1Verifier",
        "OnChainProposer",
        "CommonBridge",
        "CommonBridgeL2",
        "L2ToL1Messenger",
        "UpgradeableSystemContract",
        "SequencerRegistry",
        "OnChainProposerBased",
    ];

    for name in &contract_names {
        let filename = format!("{name}.bytecode");
        let path = bytecode_dir.join(filename);
        fs::write(&path, []).expect("Failed to write empty bytecode.");
    }
}

/// Clones OpenZeppelin and SP1 contracts into the specified path.
fn download_contract_deps(contracts_path: &Path) {
    fs::create_dir_all(contracts_path.join("lib")).expect("Failed to create contracts/lib dir");

    ethrex_l2_sdk::git_clone(
        "https://github.com/OpenZeppelin/openzeppelin-contracts-upgradeable.git",
        &contracts_path
            .join("lib/openzeppelin-contracts-upgradeable")
            .to_string_lossy(),
        None,
        true,
    )
    .expect("Failed to clone openzeppelin-contracts-upgradeable");

    ethrex_l2_sdk::git_clone(
        "https://github.com/succinctlabs/sp1-contracts.git",
        &contracts_path.join("lib/sp1-contracts").to_string_lossy(),
        None,
        false,
    )
    .expect("Failed to clone sp1-contracts");
}

fn compile_contract_to_bytecode(
    output_dir: &Path,
    contract_path: &Path,
    contract_name: &str,
    runtime_bin: bool,
    remappings: Option<&[(&str, PathBuf)]>,
) {
    println!("Compiling {contract_name} contract");
    ethrex_l2_sdk::compile_contract(output_dir, contract_path, runtime_bin, remappings)
        .expect("Failed to compile contract");
    println!("Successfully compiled {contract_name} contract");

    // Resolve the resulted file path
    let filename = if runtime_bin {
        format!("{contract_name}.bin-runtime")
    } else {
        format!("{contract_name}.bin")
    };
    let file_path = output_dir.join("solc_out").join(&filename);

    // Get the output file path
    let output_file_path = output_dir
        .join("solc_out")
        .join(format!("{contract_name}.bytecode"));

    decode_to_bytecode(&file_path, &output_file_path);

    println!("Successfully generated {contract_name} bytecode");
}

fn decode_to_bytecode(input_file_path: &Path, output_file_path: &Path) {
    let bytecode_hex = fs::read_to_string(input_file_path).expect("Failed to read file");

    let bytecode = hex::decode(bytecode_hex.trim()).expect("Failed to decode bytecode");

    fs::write(output_file_path, bytecode).expect("Failed to write bytecode");
}
