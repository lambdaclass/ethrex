use std::{path::Path, time::Duration};

use clap::Parser;
use ere_dockerized::{EreDockerizedCompiler, EreDockerizedzkVM};
use ethrex_config::networks::{
    HOLESKY_CHAIN_ID, HOODI_CHAIN_ID, MAINNET_CHAIN_ID, Network, PublicNetwork, SEPOLIA_CHAIN_ID,
};
use ethrex_rpc::{
    EthClient,
    debug::execution_witness::execution_witness_from_rpc_chain_config,
    types::block_identifier::{BlockIdentifier, BlockTag},
};
use guest_program::input::ProgramInput;
use zkvm_interface::{Compiler, Input, zkVM};

use crate::{
    cli::{Action, Options},
    report::Report,
    slack::try_send_report_to_slack,
};

mod cli;
mod report;
mod slack;

const CARGO_MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

#[tokio::main]
async fn main() {
    let Options {
        zkvm: _zkvm,
        resource,
        action,
        block,
        endless,
        slack_webhook_url,
    } = Options::parse();

    if endless && !matches!(block, BlockIdentifier::Tag(_)) {
        panic!("--endless can only be used with block tags (e.g. --block latest)");
    }

    let zkvm = _zkvm.clone().into();

    // Compile a guest program
    println!("Compiling guest program for {_zkvm:?}...");
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
    println!("{_zkvm} guest program compiled successfully.");

    // Create zkVM instance
    println!("Creating {_zkvm} instance...");
    let zkvm = EreDockerizedzkVM::new(zkvm, program, resource.clone().into()).unwrap();
    println!("{_zkvm} instance created successfully.");

    loop {
        // Prepare inputs
        println!("Preparing inputs...");
        let mut inputs = Input::new();
        let input = get_program_input(block.clone()).await;
        inputs.write_bytes(
            rkyv::to_bytes::<rkyv::rancor::Error>(&input)
                .unwrap()
                .to_vec(),
        );
        println!("Inputs prepared successfully.");

        // Execute program
        println!("Executing program...");
        let execution_result = zkvm.execute(&inputs);
        let mut report = Report {
            zkvm: _zkvm.clone(),
            resource: resource.clone(),
            action: Action::Execute,
            network: Network::PublicNetwork(PublicNetwork::Mainnet), // Temporary hardcode to Mainnet
            block: input.blocks[0].clone(),
            execution_result,
            proving_result: None,
        };
        println!("{report}");

        if let Action::Prove = action {
            // Generate proof
            let proving_result = zkvm.prove(&inputs);
            report.proving_result = Some(proving_result);
            report.action = Action::Prove;
            println!("{report}");
        }

        try_send_report_to_slack(report, slack_webhook_url.clone())
            .await
            .unwrap();

        if !endless {
            break;
        }
    }
}

async fn get_program_input(mut block_identifier: BlockIdentifier) -> ProgramInput {
    let eth_client = EthClient::new("http://157.180.1.98:8545").unwrap(); // Temporary hardcode to a public node

    // Replay EthProofs latest blocks.
    if let BlockIdentifier::Tag(BlockTag::Latest) = block_identifier {
        let mut latest_block_number = eth_client.get_block_number().await.unwrap().as_u64();

        while latest_block_number % 100 != 0 {
            tokio::time::sleep(Duration::from_secs(12)).await;

            latest_block_number = eth_client.get_block_number().await.unwrap().as_u64();
        }

        block_identifier = BlockIdentifier::Number(latest_block_number);
    }

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
