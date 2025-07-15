//! Build script for the contract deployer binary.
//! This script downloads dependencies and compiles contracts to be embedded as constants in the deployer.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{env, fs, path::Path};

fn main() {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let output_contracts_path = Path::new(&out_dir).join("contracts");
    let contracts_path = Path::new("../../crates/l2/contracts/src");

    download_contract_deps(&output_contracts_path);

    // ERC1967Proxy contract.
    compile_contract(
        &output_contracts_path,
        &output_contracts_path.join("lib/openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts/contracts/proxy/ERC1967/ERC1967Proxy.sol"),
        "ERC1967Proxy",
        false,
        None,
    );

    // SP1VerifierGroth16 contract
    compile_contract(
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
    let remappings: Vec<(&str, &Path)> =
        remappings.iter().map(|(s, p)| (*s, p.as_path())).collect();

    // L1 contracts
    let l1_contracts = [
        (
            &contracts_path.join("src/l1/OnChainProposer.sol"),
            "OnChainProposer",
        ),
        (
            &contracts_path.join("src/l1/CommonBridge.sol"),
            "CommonBridge",
        ),
    ];
    for (path, name) in l1_contracts {
        compile_contract(&output_contracts_path, path, name, false, Some(&remappings));
    }
    // L2 contracts
    let l2_contracts = [
        (
            &contracts_path.join("src/l2/CommonBridgeL2.sol"),
            "CommonBridgeL2",
        ),
        (
            &contracts_path.join("src/l2/L2ToL1Messenger.sol"),
            "L2ToL1Messenger",
        ),
    ];
    for (path, name) in l2_contracts {
        compile_contract(&output_contracts_path, path, name, true, Some(&remappings));
    }

    compile_contract(
        &output_contracts_path,
        &contracts_path.join("src/l2/L2Upgradeable.sol"),
        "UpgradeableSystemContract",
        true,
        Some(&remappings),
    );

    // Based contracts
    compile_contract(
        &output_contracts_path,
        &contracts_path.join("src/l1/based/SequencerRegistry.sol"),
        "SequencerRegistry",
        false,
        Some(&remappings),
    );
    ethrex_l2_sdk::compile_contract(
        &output_contracts_path,
        &contracts_path.join("src/l1/based/OnChainProposer.sol"),
        false,
        Some(&remappings),
    )
    .unwrap();

    // To avoid colision with the original OnChainProposer bytecode, we rename it to OnChainProposerBased
    let original_path = output_contracts_path.join("solc_out/OnChainProposer.bin");
    let bytecode_hex =
        fs::read_to_string(&original_path).expect("Failed to read OnChainProposer.bin");
    let bytecode = hex::decode(bytecode_hex.trim()).expect("Failed to decode bytecode");
    fs::write(
        output_contracts_path.join("solc_out/OnChainProposerBased.bytecode"),
        bytecode,
    )
    .expect("Failed to write renamed bytecode");

    println!("cargo::rerun-if-changed=build.rs");
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

fn compile_contract(
    output_dir: &Path,
    contract_path: &Path,
    contract_name: &str,
    runtime_bin: bool,
    remappings: Option<&[(&str, &Path)]>,
) {
    println!("Compiling {contract_name} contract");
    ethrex_l2_sdk::compile_contract(output_dir, contract_path, runtime_bin, remappings)
        .expect("Failed to compile {contract_name}");
    println!("Successfully compiled {contract_name} contract");
    decode_to_bytecode(output_dir, contract_name, runtime_bin);
    println!("Successfully generated {contract_name} bytecode");
}

fn decode_to_bytecode(output_dir: &Path, contract: &str, runtime_bin: bool) {
    let filename = if runtime_bin {
        format!("{contract}.bin-runtime")
    } else {
        format!("{contract}.bin")
    };

    let bytecode_hex = fs::read_to_string(output_dir.join("solc_out").join(&filename))
        .expect("Failed to read {filename}");

    let bytecode =
        hex::decode(bytecode_hex.trim()).expect("Failed to decode bytecode for {contract}");

    fs::write(
        output_dir
            .join("solc_out")
            .join(format!("{contract}.bytecode")),
        bytecode,
    )
    .expect("Failed to write bytecode for {contract}");
}
