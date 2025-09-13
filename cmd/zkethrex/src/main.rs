use std::path::Path;

use clap::Parser;
use ere_dockerized::{EreDockerizedCompiler, EreDockerizedzkVM};
use ethrex_config::networks::{
    HOLESKY_CHAIN_ID, HOODI_CHAIN_ID, MAINNET_CHAIN_ID, Network, PublicNetwork, SEPOLIA_CHAIN_ID,
};
use ethrex_rpc::{
    EthClient, debug::execution_witness::execution_witness_from_rpc_chain_config,
    types::block_identifier::BlockIdentifier,
};
use guest_program::input::ProgramInput;
use zkvm_interface::{Compiler, Input, zkVM};

use crate::cli::{Action, Options};

mod cli;

const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

#[tokio::main]
async fn main() {
    let Options {
        zkvm,
        resource,
        action,
        block,
    } = Options::parse();

    let zkvm = zkvm.into();

    // Compile a guest program
    println!("Compiling guest program for {zkvm:?}...");
    let workspace_dir = Path::new(CARGO_MANIFEST_DIR)
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let guest_relative_dir = Path::new("crates/l2/prover/src/guest_program/src");
    let program = EreDockerizedCompiler::new(zkvm, workspace_dir)
        .unwrap()
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
