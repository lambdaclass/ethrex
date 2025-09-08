use std::path::Path;

use ere_dockerized::{EreDockerizedCompiler, EreDockerizedzkVM, ErezkVM};
use ethrex_config::networks::{
    HOLESKY_CHAIN_ID, HOODI_CHAIN_ID, MAINNET_CHAIN_ID, Network, PublicNetwork, SEPOLIA_CHAIN_ID,
};
use ethrex_rpc::{
    EthClient, debug::execution_witness::execution_witness_from_rpc_chain_config,
    types::block_identifier::BlockIdentifier,
};
use guest_program::input::ProgramInput;
use zkvm_interface::{Compiler, Input, ProverResourceType, zkVM};

const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

#[tokio::main]
async fn main() {
    let zkvm = ErezkVM::SP1;

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
    println!("Guest program compiled successfully.");

    // Create zkVM instance
    println!("Creating zkVM instance...");
    let resource = ProverResourceType::Cpu;
    let zkvm = EreDockerizedzkVM::new(zkvm, program, resource).unwrap();
    println!("zkVM instance created successfully.");

    // Prepare inputs
    println!("Preparing inputs...");
    let mut inputs = Input::new();
    inputs.write_bytes(
        rkyv::to_bytes::<rkyv::rancor::Error>(&get_program_input().await)
            .unwrap()
            .to_vec(),
    );
    println!("Inputs prepared successfully.");

    // Execute program
    println!("Executing program...");
    let (public_values, execution_report) = zkvm.execute(&inputs).unwrap();
    println!("{execution_report:#?}");

    // // Generate proof
    // let (public_values, proof, proving_report) = zkvm.prove(&inputs).unwrap();
    // println!("Proof generated in: {:#?}", proving_report.proving_time);

    // // Verify proof
    // let public_values = zkvm.verify(&proof).unwrap();
    // println!("Proof verified successfully!");
}

async fn get_program_input() -> ProgramInput {
    let eth_client = EthClient::new("http://157.180.1.98:8545").unwrap();

    let canonical_head = eth_client.get_block_number().await.unwrap().as_u64();

    println!("https://etherscan.io/block/{canonical_head}");

    let block = eth_client
        .get_raw_block(BlockIdentifier::Number(canonical_head))
        .await
        .unwrap();

    let execution_witness = {
        let rpc_execution_witness = eth_client
            .get_witness(BlockIdentifier::Number(canonical_head), None)
            .await
            .unwrap();

        let chain_id = eth_client.get_chain_id().await.unwrap().as_u64();

        let chain_config = network_from_chain_id(chain_id, false)
            .get_genesis()
            .unwrap()
            .config;

        execution_witness_from_rpc_chain_config(rpc_execution_witness, chain_config, canonical_head)
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
