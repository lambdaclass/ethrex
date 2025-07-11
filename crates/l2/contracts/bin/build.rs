//! Build script for the contract deployer binary.
//! This script downloads dependencies and compiles contracts to be embedded as constants in the deployer.
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
        &contracts_path.join("lib/openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts/contracts/proxy/ERC1967"),
        "ERC1967Proxy",
        false,
        None,
    );

    // Compile the SP1VerifierGroth16 contract
    ethrex_l2_sdk::compile_contract(
        &contracts_path,
        &contracts_path.join("lib/sp1-contracts/contracts/src/v5.0.0/SP1VerifierGroth16.sol"),
        false,
        None,
    )
    .unwrap();
    println!("Successfully compiled SP1VerifierGroth16 contract");
    decode_to_bytecode(&contracts_path, "SP1Verifier", false);

    // Get the openzeppelin contracts remappings
    let remappings_raw = vec![
        (
            "@openzeppelin/contracts",
            contracts_path.join(
                "lib/openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts/contracts",
            ),
        ),
        (
            "@openzeppelin/contracts-upgradeable",
            contracts_path.join("lib/openzeppelin-contracts-upgradeable/contracts"),
        ),
    ];
    let remappings: Vec<(&str, &Path)> = remappings_raw
        .iter()
        .map(|(s, p)| (*s, p.as_path()))
        .collect();

    // Compile the L1 contracts
    compile_contract_to_bytecode(
        &contracts_path,
        Path::new("src/l1"),
        "OnChainProposer",
        false,
        Some(&remappings),
    );
    compile_contract_to_bytecode(
        &contracts_path,
        Path::new("src/l1"),
        "CommonBridge",
        false,
        Some(&remappings),
    );

    // Compile the L2 contracts
    compile_contract_to_bytecode(
        &contracts_path,
        &Path::new("src/l2"),
        "CommonBridgeL2",
        true,
        Some(&remappings),
    );
    compile_contract_to_bytecode(
        &contracts_path,
        &Path::new("src/l2"),
        "L2ToL1Messenger",
        true,
        Some(&remappings),
    );

    // Compile based contracts
    compile_contract_to_bytecode(
        &contracts_path,
        &Path::new("src/l1/based"),
        "SequencerRegistry",
        false,
        Some(&remappings),
    );
    ethrex_l2_sdk::compile_contract(
        &contracts_path,
        &Path::new("src/l1/based/OnChainProposer.sol"),
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

    // To avoid colision with the original OnChainProposer bytecode, we rename it to OnChainProposerBased
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
    output_dir: &Path,
    contract_path: &Path,
    contract_name: &str,
    runtime_bin: bool,
    remappings: Option<&[(&str, &Path)]>,
) {
    println!("Compiling {contract_name} contract");
    ethrex_l2_sdk::compile_contract(
        output_dir,
        &contract_path.join(&format!("{contract_name}.sol")),
        runtime_bin,
        remappings,
    )
    .unwrap();
    println!("Successfully compiled {contract_name} contract");
    decode_to_bytecode(output_dir, contract_name, runtime_bin);
    println!("Successfully generated {contract_name} bytecode");
}

fn decode_to_bytecode(contracts_path: &Path, contract: &str, runtime_bin: bool) {
    let contract_bytecode_hex = if runtime_bin {
        std::fs::read_to_string(
            contracts_path
                .join("solc_out")
                .join(format!("{contract}.bin-runtime")),
        )
        .unwrap()
    } else {
        std::fs::read_to_string(
            contracts_path
                .join("solc_out")
                .join(format!("{contract}.bin")),
        )
        .unwrap()
    };
    let contract_bytecode = hex::decode(contract_bytecode_hex.trim()).unwrap();

    std::fs::write(
        contracts_path
            .join("solc_out")
            .join(format!("{contract}.bytecode",)),
        contract_bytecode,
    )
    .unwrap();
}
