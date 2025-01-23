use std::collections::HashMap;

use bench::{
    constants::{CANCUN_CONFIG, MAINNET_CHAIN_ID, MAINNET_SPEC_ID, RPC_RATE_LIMIT},
    rpc::{asynch::*, db::RpcDB, Account, NodeRLP},
};
use clap::Parser;
use ethrex_core::{Address, U256};
use ethrex_l2::utils::prover::proving_systems::ProverType;
use ethrex_prover_lib::prover::{create_prover, Prover};
use ethrex_vm::{execution_db::ExecutionDB, spec_id};
use futures_util::future::join_all;
use tokio_utils::RateLimiter;
use zkvm_interface::io::ProgramInput;

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    rpc_url: String,
    #[arg(short, long)]
    block_number: usize,
}

#[tokio::main]
async fn main() {
    let Args {
        rpc_url,
        block_number,
    } = Args::parse();

    println!("fetching block {block_number} and its parent header");
    let block = get_block(&rpc_url, block_number)
        .await
        .expect("failed to fetch block");
    let parent_block_header = get_block(&rpc_url, block_number - 1)
        .await
        .expect("failed to fetch block")
        .header;

    println!("populating rpc db cache");
    let rpc_db = RpcDB::with_callers(&rpc_url, block_number - 1, &block)
        .await
        .expect("failed to create rpc db");

    println!("pre-executing to build execution db");
    ExecutionDB::pre_execute(
        &block,
        MAINNET_CHAIN_ID,
        spec_id(&CANCUN_CONFIG, block.header.timestamp),
        rpc_db,
    )
    .unwrap();
    // let touched_state = get_touched_state(&block, MAINNET_CHAIN_ID, MAINNET_SPEC_ID)
    //.expect("failed to get touched state");

    // println!("building program input");
    // let storages: HashMap<Address, HashMap<U256, U256>> = storages
    //     .into_iter()
    //     .filter_map(|(address, storage)| {
    //         if !storage.is_empty() {
    //             Some((address, storage.into_iter().collect()))
    //         } else {
    //             None
    //         }
    //     })
    //     .collect();

    // let account_proofs = {
    //     let root_node = if !account_proofs.is_empty() {
    //         Some(account_proofs.swap_remove(0))
    //     } else {
    //         None
    //     };
    //     (root_node, account_proofs)
    // };

    // let storages_proofs: HashMap<Address, (Option<NodeRLP>, Vec<NodeRLP>)> = storages_proofs
    //     .into_iter()
    //     .map(|(address, mut proofs)| {
    //         (address, {
    //             let root_node = if !proofs.is_empty() {
    //                 Some(proofs.swap_remove(0))
    //             } else {
    //                 None
    //             };
    //             (root_node, proofs)
    //         })
    //     })
    //     .collect();

    // let db = todo!();
    // // let db = ExecutionDB::new(
    // //     accounts,
    // //     storages,
    // //     codes,
    // //     account_proofs,
    // //     storages_proofs,
    // //     CANCUN_CONFIG,
    // // )
    // // .expect("failed to create execution db");

    // println!("proving");
    // let mut prover = create_prover(ProverType::SP1);
    // let receipt = prover
    //     .prove(ProgramInput {
    //         block,
    //         parent_block_header,
    //         db,
    //     })
    //     .expect("proving failed");
    // let execution_gas = prover.get_gas().expect("failed to get execution gas");
}
