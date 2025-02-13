use std::{fs::File, io::Write};

use bench::{
    cache::{load_cache, write_cache, Cache},
    rpc::{db::RpcDB, get_block, get_latest_block_number},
};
use clap::Parser;
use ethrex_l2::utils::prover::proving_systems::ProverType;
use ethrex_prover_lib::prover::create_prover;
use ethrex_vm::execution_db::ToExecDB;
use zkvm_interface::io::ProgramInput;

#[cfg(not(any(feature = "sp1", feature = "risc0")))]
compile_error!("Choose prover backends (sp1, risc0).");

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    rpc_url: String,
    #[arg(short, long)]
    block_number: Option<usize>,
    #[arg(short, long)]
    prove: bool,
}

#[tokio::main]
async fn main() {
    let Args {
        rpc_url,
        block_number,
        prove,
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

    let Cache {
        block,
        parent_block_header,
        db,
    } = match load_cache(block_number) {
        Ok(cache) => cache,
        Err(err) => {
            println!("failed to load cache for block {block_number}: {err}");

            println!("fetching block {block_number} and its parent header");
            let block = get_block(&rpc_url, block_number)
                .await
                .expect("failed to fetch block");

            let parent_block_header = get_block(&rpc_url, block_number - 1)
                .await
                .expect("failed to fetch block")
                .header;

            println!("populating rpc db cache");
            let rpc_db = RpcDB::with_cache(&rpc_url, block_number - 1, &block)
                .await
                .expect("failed to create rpc db");

            println!("pre-executing to build execution db");
            let db = rpc_db
                .to_exec_db(&block)
                .expect("failed to build execution db");

            let cache = Cache {
                block,
                parent_block_header,
                db,
            };
            write_cache(&cache).expect("failed to write cache");
            cache
        }
    };

    #[cfg(feature = "sp1")]
    let mut prover = create_prover(ProverType::SP1);
    #[cfg(feature = "risc0")]
    let mut prover = create_prover(ProverType::RISC0);

    let now = std::time::Instant::now();
    if prove {
        println!("proving");
        prover
            .prove(ProgramInput {
                block,
                parent_block_header,
                db,
            })
            .expect("proving failed");
    } else {
        println!("executing");
        prover
            .execute(ProgramInput {
                block,
                parent_block_header,
                db,
            })
            .expect("proving failed");
    }
    let elapsed = now.elapsed().as_secs();
    println!(
        "finished in {} minutes for block {}",
        elapsed / 60,
        block_number
    );

    // get_gas() is unimplemented for SP1
    // let gas = prover.get_gas().expect("failed to get execution gas");
    // println!("total gas consumption: {gas}");
}
