use std::path::Path;

use clap::{Parser, ValueEnum};
use ere_dockerized::{EreDockerizedCompiler, EreDockerizedzkVM, ErezkVM};
use ethrex_config::networks::{
    HOLESKY_CHAIN_ID, HOODI_CHAIN_ID, MAINNET_CHAIN_ID, Network, PublicNetwork, SEPOLIA_CHAIN_ID,
};
use ethrex_rpc::{
    EthClient,
    debug::execution_witness::execution_witness_from_rpc_chain_config,
    types::block_identifier::{BlockIdentifier, BlockTag},
};
use guest_program::input::ProgramInput;
use zkvm_interface::{Compiler, Input, ProverResourceType, zkVM};

const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

#[derive(Parser)]
pub struct Options {
    #[arg(long, value_enum)]
    zkvm: ZKVM,
    #[arg(long, value_enum)]
    resource: Resource,
    #[arg(long, value_enum)]
    action: Action,
    #[arg(long, value_parser = parse_block_identifier, default_value = "latest", help = "Block identifier (number or tag: earliest, finalized, safe, latest, pending)")]
    block: BlockIdentifier,
}

fn parse_block_identifier(s: &str) -> Result<BlockIdentifier, String> {
    if let Ok(num) = s.parse::<u64>() {
        Ok(BlockIdentifier::Number(num))
    } else {
        match s {
            "earliest" => Ok(BlockIdentifier::Tag(BlockTag::Earliest)),
            "finalized" => Ok(BlockIdentifier::Tag(BlockTag::Finalized)),
            "safe" => Ok(BlockIdentifier::Tag(BlockTag::Safe)),
            "latest" => Ok(BlockIdentifier::Tag(BlockTag::Latest)),
            "pending" => Ok(BlockIdentifier::Tag(BlockTag::Pending)),
            _ => Err(format!("Invalid block identifier: {s}")),
        }
    }
}

#[derive(Clone, Debug, ValueEnum)]
pub enum ZKVM {
    Jolt,
    Nexus,
    OpenVM,
    Pico,
    Risc0,
    SP1,
    Ziren,
    Zisk,
}

impl From<ZKVM> for ErezkVM {
    fn from(value: ZKVM) -> Self {
        match value {
            ZKVM::Jolt => ErezkVM::Jolt,
            ZKVM::Nexus => ErezkVM::Nexus,
            ZKVM::OpenVM => ErezkVM::OpenVM,
            ZKVM::Pico => ErezkVM::Pico,
            ZKVM::Risc0 => ErezkVM::Risc0,
            ZKVM::SP1 => ErezkVM::SP1,
            ZKVM::Ziren => ErezkVM::Ziren,
            ZKVM::Zisk => ErezkVM::Zisk,
        }
    }
}

#[derive(Clone, Debug, ValueEnum)]
pub enum Resource {
    CPU,
    GPU,
}

impl From<Resource> for ProverResourceType {
    fn from(value: Resource) -> Self {
        match value {
            Resource::CPU => ProverResourceType::Cpu,
            Resource::GPU => ProverResourceType::Gpu,
        }
    }
}

#[derive(Clone, Debug, ValueEnum)]
pub enum Action {
    Execute,
    Prove,
}

#[tokio::main]
async fn main() {
    let Options {
        zkvm,
        resource,
        action,
        block,
    } = Options::parse();

    println!(
        "Replaying https://etherscan.io/block/{block:?} using {zkvm:?} on {resource:?} ({action:?})"
    );

    let zkvm = zkvm.into();

    // Compile a guest program
    println!("Compiling guest program for {zkvm:?}...");
    let workspace_dir = Path::new(CARGO_MANIFEST_DIR)
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let guest_relative_dir = Path::new("crates/l2/prover/src/guest_program/src");
    let compiler = EreDockerizedCompiler::new(zkvm, workspace_dir).unwrap();
    let program = compiler
        .compile(
            &workspace_dir
                .join(guest_relative_dir)
                .join(zkvm.to_string()),
        )
        .unwrap();
    println!("{zkvm} guest program compiled successfully.");

    // Create zkVM instance
    println!("Creating {zkvm} instance...");
    let zkvm = EreDockerizedzkVM::new(zkvm, program, resource.into()).unwrap();
    println!("{} instance created successfully.", zkvm.zkvm());

    // Prepare inputs
    println!("Preparing inputs...");
    let mut inputs = Input::new();
    let input = get_program_input(block).await;
    inputs.write_bytes(
        rkyv::to_bytes::<rkyv::rancor::Error>(&input)
            .unwrap()
            .to_vec(),
    );
    println!("Inputs prepared successfully.");

    // Execute program
    println!("Executing program...");
    let (_public_values, execution_report) = zkvm.execute(&inputs).unwrap();
    println!("{execution_report:#?}");

    if let Action::Prove = action {
        // Generate proof
        let (_public_values, _proof, proving_report) = zkvm.prove(&inputs).unwrap();
        println!("Proof generated in: {:#?}", proving_report.proving_time);
    }
}

async fn get_program_input(block_identifier: BlockIdentifier) -> ProgramInput {
    let eth_client = EthClient::new("http://157.180.1.98:8545").unwrap();

    let block = eth_client
        .get_raw_block(block_identifier.clone())
        .await
        .unwrap();

    println!("https://etherscan.io/block/{}", block.header.number);

    let execution_witness = {
        let rpc_execution_witness = eth_client
            .get_witness(block_identifier, None)
            .await
            .unwrap();

        let chain_id = eth_client.get_chain_id().await.unwrap().as_u64();

        let chain_config = network_from_chain_id(chain_id, false)
            .get_genesis()
            .unwrap()
            .config;

        execution_witness_from_rpc_chain_config(
            rpc_execution_witness,
            chain_config,
            block.header.number,
        )
        .unwrap()
    };

    ProgramInput {
        blocks: vec![block],
        db: execution_witness,
        elasticity_multiplier: 2,
        // The L2 specific fields (blob_commitment, blob_proof)
        // will be filled by Default::default() if the 'l2' feature of
        // 'zkvm_interface' is active (due to workspace compilation).
        // If 'zkvm_interface' is compiled without 'l2' (e.g. standalone build),
        // these fields won't exist in ProgramInput, and ..Default::default()
        // will correctly not try to fill them.
        // A better solution would involve rethinking the `l2` feature or the
        // inclusion of this crate in the workspace.
        ..Default::default()
    }
}

fn network_from_chain_id(chain_id: u64, l2: bool) -> Network {
    match chain_id {
        MAINNET_CHAIN_ID => Network::PublicNetwork(PublicNetwork::Mainnet),
        HOLESKY_CHAIN_ID => Network::PublicNetwork(PublicNetwork::Holesky),
        HOODI_CHAIN_ID => Network::PublicNetwork(PublicNetwork::Hoodi),
        SEPOLIA_CHAIN_ID => Network::PublicNetwork(PublicNetwork::Sepolia),
        _ => {
            if l2 {
                Network::LocalDevnetL2
            } else {
                Network::LocalDevnet
            }
        }
    }
}
