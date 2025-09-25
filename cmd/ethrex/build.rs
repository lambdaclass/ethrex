#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::{
    env, fs,
    path::{Path, PathBuf},
};
use vergen_git2::{Emitter, Git2Builder, RustcBuilder};
// This build code is needed to add some env vars in order to construct the node client version
// VERGEN_RUSTC_HOST_TRIPLE to get the build OS
// VERGEN_RUSTC_SEMVER to get the rustc version
// VERGEN_GIT_BRANCH to get the git branch name
// VERGEN_GIT_SHA to get the git commit hash

// This script downloads dependencies and compiles contracts to be embedded as constants in the deployer.

const L2_GENESIS_PATH: &str = "../../fixtures/genesis/l2.json";

const DETERMINISTIC_DEPLOYMENT_CODE: [u8; 69] = [
    0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xe0, 0x36, 0x01, 0x60, 0x00, 0x81, 0x60, 0x20, 0x82, 0x37, 0x80, 0x35, 0x82, 0x82, 0x34, 0xf5,
    0x80, 0x15, 0x15, 0x60, 0x39, 0x57, 0x81, 0x82, 0xfd, 0x5b, 0x80, 0x82, 0x52, 0x50, 0x50, 0x50,
    0x60, 0x14, 0x60, 0x0c, 0xf3,
];

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=COMPILE_CONTRACTS");
    println!("cargo:rerun-if-changed=../../crates/l2/contracts/src");

    // Export build OS and rustc version as environment variables
    let rustc = RustcBuilder::default()
        .semver(true)
        .host_triple(true)
        .build()?;

    // Export git commit hash and branch name as environment variables
    let git2 = Git2Builder::default().branch(true).sha(true).build()?;

    Emitter::default()
        .add_instructions(&rustc)?
        .add_instructions(&git2)?
        .emit()?;

    download_script();

    // If COMPILE_CONTRACTS is not set, skip
    if env::var_os("COMPILE_CONTRACTS").is_some() {
        let out_dir = env::var_os("OUT_DIR").unwrap();
        update_genesis_file(L2_GENESIS_PATH.as_ref(), Path::new(&out_dir))?;
    }

    Ok(())
}

fn download_script() {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let output_contracts_path = Path::new(&out_dir).join("contracts");
    println!(
        "Compiling contracts to: {}",
        output_contracts_path.display()
    );
    let contracts_path = Path::new("../../crates/l2/contracts/src");

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
        &[&output_contracts_path]
    );

    // SP1VerifierGroth16 contract
    compile_contract_to_bytecode(
        &output_contracts_path,
        &output_contracts_path
            .join("lib/sp1-contracts/contracts/src/v5.0.0/SP1VerifierGroth16.sol"),
        "SP1Verifier",
        false,
        None,
        &[&output_contracts_path],
    );

    let remappings = [(
        "@openzeppelin/contracts",
        output_contracts_path.join("lib/openzeppelin-contracts/contracts"),
    )];

    compile_contract_to_bytecode(
        &output_contracts_path,
        &output_contracts_path.join("lib/create2deployer/contracts/Create2Deployer.sol"),
        "Create2Deployer",
        true,
        Some(&remappings),
        &[contracts_path],
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
        (
            &Path::new("../../crates/l2/contracts/src/l1/OnChainProposer.sol"),
            "OnChainProposer",
        ),
        (
            &Path::new("../../crates/l2/contracts/src/l1/CommonBridge.sol"),
            "CommonBridge",
        ),
    ];
    for (path, name) in l1_contracts {
        compile_contract_to_bytecode(
            &output_contracts_path,
            path,
            name,
            false,
            Some(&remappings),
            &[contracts_path],
        );
    }
    // L2 contracts
    let l2_contracts = [
        (
            &Path::new("../../crates/l2/contracts/src/l2/CommonBridgeL2.sol"),
            "CommonBridgeL2",
        ),
        (
            &Path::new("../../crates/l2/contracts/src/l2/L2ToL1Messenger.sol"),
            "L2ToL1Messenger",
        ),
        (
            &Path::new("../../crates/l2/contracts/src/l2/L2Upgradeable.sol"),
            "UpgradeableSystemContract",
        ),
    ];
    for (path, name) in l2_contracts {
        compile_contract_to_bytecode(
            &output_contracts_path,
            path,
            name,
            true,
            Some(&remappings),
            &[contracts_path],
        );
    }

    // Based contracts
    compile_contract_to_bytecode(
        &output_contracts_path,
        Path::new("../../crates/l2/contracts/src/l1/based/SequencerRegistry.sol"),
        "SequencerRegistry",
        false,
        Some(&remappings),
        &[contracts_path],
    );
    ethrex_l2_sdk::compile_contract(
        &output_contracts_path,
        Path::new("../../crates/l2/contracts/src/l1/based/OnChainProposer.sol"),
        false,
        Some(&remappings),
        &[contracts_path],
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

/// Clones OpenZeppelin, SP1 contracts and create2deployer into the specified path.
fn download_contract_deps(contracts_path: &Path) {
    fs::create_dir_all(contracts_path.join("lib")).expect("Failed to create contracts/lib dir");

    ethrex_l2_sdk::git_clone(
        "https://github.com/OpenZeppelin/openzeppelin-contracts-upgradeable.git",
        &contracts_path
            .join("lib/openzeppelin-contracts-upgradeable")
            .to_string_lossy(),
        Some("release-v5.4"),
        true,
    )
    .expect("Failed to clone openzeppelin-contracts-upgradeable");

    // Using version 4.9 for create2deployer
    ethrex_l2_sdk::git_clone(
        "https://github.com/OpenZeppelin/openzeppelin-contracts.git",
        &contracts_path
            .join("lib/openzeppelin-contracts")
            .to_string_lossy(),
        Some("release-v4.9"),
        None,
        true,
    )
    .expect("Failed to clone openzeppelin-contracts");

    ethrex_l2_sdk::git_clone(
        "https://github.com/succinctlabs/sp1-contracts.git",
        &contracts_path.join("lib/sp1-contracts").to_string_lossy(),
        None,
        None,
        false,
    )
    .expect("Failed to clone sp1-contracts");

    ethrex_l2_sdk::git_clone(
        "https://github.com/pcaversaccio/create2deployer",
        &contracts_path.join("lib/create2deployer").to_string_lossy(),
        None,
        None,
        true,
    )
    .expect("Failed to clone create2deployer");
}

fn compile_contract_to_bytecode(
    output_dir: &Path,
    contract_path: &Path,
    contract_name: &str,
    runtime_bin: bool,
    remappings: Option<&[(&str, PathBuf)]>,
    allow_paths: &[&Path],
) {
    println!("Compiling {contract_name} contract");
    ethrex_l2_sdk::compile_contract(
        output_dir,
        contract_path,
        runtime_bin,
        remappings,
        allow_paths,
    )
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

use std::collections::HashMap;

use bytes::Bytes;
use ethrex_common::types::{Genesis, GenesisAccount};
use ethrex_common::{Address, H160, U256};
use ethrex_l2_sdk::{
    COMMON_BRIDGE_L2_ADDRESS, CREATE2DEPLOYER_ADDRESS, DETERMINISTIC_DEPLOYMENT_PROXY_ADDRESS,
    L2_TO_L1_MESSENGER_ADDRESS, SAFE_SINGLETON_FACTORY_ADDRESS, address_to_word, get_erc1967_slot,
};
use genesis_tool::genesis::write_genesis_as_json;

#[allow(clippy::enum_variant_names)]
#[derive(Debug, thiserror::Error)]
pub enum SystemContractsUpdaterError {
    #[error("Failed to deploy contract: {0}")]
    FailedToDecodeRuntimeCode(#[from] hex::FromHexError),
    #[error("Failed to serialize modified genesis: {0}")]
    FailedToSerializeModifiedGenesis(#[from] serde_json::Error),
    #[error("Failed to write modified genesis file: {0}")]
    FailedToWriteModifiedGenesisFile(#[from] std::io::Error),
    #[error("Failed to read path: {0}")]
    InvalidPath(String),
    #[error(
        "Contract bytecode not found. Make sure to compile the updater with `COMPILE_CONTRACTS` set."
    )]
    BytecodeNotFound,
}

/// Bytecode of the CommonBridgeL2 contract.
fn common_bridge_l2_runtime(out_dir: &Path) -> Vec<u8> {
    let path = out_dir.join("contracts/solc_out/CommonBridgeL2.bytecode");
    fs::read(path).expect("Failed to read bytecode file")
}

/// Bytecode of the L2ToL1Messenger contract.
fn l2_to_l1_messenger_runtime(out_dir: &Path) -> Vec<u8> {
    let path = out_dir.join("contracts/solc_out/L2ToL1Messenger.bytecode");
    fs::read(path).expect("Failed to read bytecode file")
}
/// Bytecode of the Create2Deployer contract.
fn create2deployer_runtime(out_dir: &Path) -> Vec<u8> {
    let path = out_dir.join("contracts/solc_out/Create2Deployer.bytecode");
    fs::read(path).expect("Failedto read bytecode file")
}

/// Bytecode of the L2Upgradeable contract.
fn l2_upgradeable_runtime(out_dir: &Path) -> Vec<u8> {
    let path = out_dir.join("contracts/solc_out/UpgradeableSystemContract.bytecode");
    fs::read(path).expect("Failed to read bytecode file")
}

/// Address authorized to perform system contract upgrades
/// 0x000000000000000000000000000000000000f000
pub const ADMIN_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xf0, 0x00,
]);

/// Mask used to derive the initial implementation address
/// 0x0000000000000000000000000000000000001000
pub const IMPL_MASK: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x10, 0x00,
]);

fn add_with_proxy(
    genesis: &mut Genesis,
    address: Address,
    code: Vec<u8>,
    out_dir: &Path,
) -> Result<(), SystemContractsUpdaterError> {
    let impl_address = address ^ IMPL_MASK;

    if code.is_empty() {
        return Err(SystemContractsUpdaterError::BytecodeNotFound);
    }

    genesis.alloc.insert(
        impl_address,
        GenesisAccount {
            code: Bytes::from(code),
            storage: HashMap::new(),
            balance: U256::zero(),
            nonce: 1,
        },
    );

    let mut storage = HashMap::new();
    storage.insert(
        get_erc1967_slot("eip1967.proxy.implementation"),
        address_to_word(impl_address),
    );
    storage.insert(
        get_erc1967_slot("eip1967.proxy.admin"),
        address_to_word(ADMIN_ADDRESS),
    );
    genesis.alloc.insert(
        address,
        GenesisAccount {
            code: Bytes::from(l2_upgradeable_runtime(out_dir)),
            storage,
            balance: U256::zero(),
            nonce: 1,
        },
    );
    Ok(())
}

pub fn update_genesis_file(
    l2_genesis_path: &Path,
    out_dir: &Path,
) -> Result<(), SystemContractsUpdaterError> {
    let mut genesis = read_genesis_file(l2_genesis_path.to_str().ok_or(
        SystemContractsUpdaterError::InvalidPath(
            "Failed to convert l2 genesis path to string".to_string(),
        ),
    )?);

    add_with_proxy(
        &mut genesis,
        COMMON_BRIDGE_L2_ADDRESS,
        common_bridge_l2_runtime(out_dir),
        out_dir,
    )?;

    add_with_proxy(
        &mut genesis,
        L2_TO_L1_MESSENGER_ADDRESS,
        l2_to_l1_messenger_runtime(out_dir),
        out_dir,
    )?;

    add_deterministic_deployers(&mut genesis, out_dir);

    write_genesis_as_json(genesis, Path::new(l2_genesis_path)).map_err(std::io::Error::other)?;

    Ok(())
}

fn add_deterministic_deployers(genesis: &mut Genesis, out_dir: &Path) {
    genesis.alloc.insert(
        DETERMINISTIC_DEPLOYMENT_PROXY_ADDRESS,
        GenesisAccount {
            code: Bytes::from_static(&DETERMINISTIC_DEPLOYMENT_CODE),
            storage: HashMap::new(),
            balance: U256::zero(),
            nonce: 1,
        },
    );

    genesis.alloc.insert(
        SAFE_SINGLETON_FACTORY_ADDRESS,
        GenesisAccount {
            code: Bytes::from_static(&DETERMINISTIC_DEPLOYMENT_CODE),
            storage: HashMap::new(),
            balance: U256::zero(),
            nonce: 1,
        },
    );

    genesis.alloc.insert(
        CREATE2DEPLOYER_ADDRESS,
        GenesisAccount {
            code: Bytes::from(create2deployer_runtime(out_dir)),
            storage: HashMap::new(),
            balance: U256::zero(),
            nonce: 1,
        },
    );
}

// From cmd/ethrex
pub fn read_genesis_file(genesis_file_path: &str) -> Genesis {
    let genesis_file = std::fs::File::open(genesis_file_path).expect("Failed to open genesis file");
    _genesis_file(genesis_file).expect("Failed to decode genesis file")
}

// From cmd/ethrex/decode.rs
fn _genesis_file(file: File) -> Result<Genesis, serde_json::Error> {
    let genesis_reader = BufReader::new(file);
    serde_json::from_reader(genesis_reader)
}
