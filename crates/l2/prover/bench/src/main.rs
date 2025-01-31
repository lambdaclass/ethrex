use std::{fs::File, io::Write};

use bench::{
    constants::{CANCUN_CONFIG, MAINNET_CHAIN_ID},
    rpc::{db::RpcDB, get_block, get_latest_block_number},
};
use clap::Parser;
use ethrex_l2::utils::prover::proving_systems::ProverType;
use ethrex_prover_lib::prover::create_prover;
use ethrex_vm::execution_db::ToExecDB;
use zkvm_interface::io::ProgramInput;

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    rpc_url: String,
    #[arg(short, long)]
    block_number: Option<usize>,
}

#[tokio::main]
async fn main() {
    let Args {
        rpc_url,
        block_number,
    } = Args::parse();

    let block_number = match block_number {
        Some(n) => n,
        None => {
            println!("fetching latest block number");
            get_latest_block_number(&rpc_url)
                .await
                .expect("failed to fetch latest block number")
        }
    };

    println!("fetching block {block_number} and its parent header");
    let block = get_block(&rpc_url, block_number)
        .await
        .expect("failed to fetch block");
    let parent_block_header = get_block(&rpc_url, block_number - 1)
        .await
        .expect("failed to fetch block")
        .header;

    let db = if let Ok(file) = File::open("db.bin") {
        println!("db file found");
        bincode::deserialize_from(file).expect("failed to deserialize db from file")
    } else {
        println!("db file not found");

        println!("populating rpc db cache");
        let rpc_db = RpcDB::with_cache(&rpc_url, block_number - 1, &block)
            .await
            .expect("failed to create rpc db");

        println!("pre-executing to build execution db");
        let db = rpc_db
            .to_exec_db(&block)
            .expect("failed to build execution db");

        println!("writing db to file db.bin");
        let mut file = File::create("db.bin").expect("failed to create db file");
        file.write_all(
            bincode::serialize(&db)
                .expect("failed to serialize db")
                .as_slice(),
        )
        .expect("failed to write db to file");

        db
    };

    println!("proving");
    let mut prover = create_prover(ProverType::SP1);
    let receipt = prover
        .prove(ProgramInput {
            block,
            parent_block_header,
            db,
        })
        .expect("proving failed");
    let execution_gas = prover.get_gas().expect("failed to get execution gas");
}
