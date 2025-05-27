use std::fs::File;

use clap::Parser;
use ethrex_common::types::ELASTICITY_MULTIPLIER;
use ethrex_prover_bench::{
    cache::{load_cache, write_cache, Cache},
    rpc::{db::RpcDB, get_block, get_latest_block_number},
};
use ethrex_prover_lib::execute;
use serde_json::json;
use zkvm_interface::io::ProgramInput;

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

    let now = std::time::Instant::now();
    let gas_used = block.header.gas_used as f64;
    if prove {
        println!("proving");
        ethrex_prover_lib::prove(ProgramInput {
            blocks: vec![block],
            parent_block_header,
            db,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
        })
        .expect("proving failed");
    } else {
        println!("executing");
        execute(ProgramInput {
            blocks: vec![block],
            parent_block_header,
            db,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
        })
        .expect("proving failed");
    }
    let elapsed = now.elapsed().as_secs();
    println!(
        "finished in {} minutes for block {} with gas {}",
        elapsed / 60,
        block_number,
        gas_used
    );

    write_benchmark_file(gas_used, elapsed as f64);
}

fn write_benchmark_file(gas_used: f64, elapsed: f64) {
    let rate = gas_used / 1e6 / elapsed;

    let backend = if cfg!(feature = "sp1") {
        "SP1"
    } else if cfg!(feature = "risc0") {
        "Risc0"
    } else if cfg!(feature = "pico") {
        "Pico"
    } else {
        unreachable!();
    };

    let processor = if cfg!(feature = "ci") {
        "RTX A6000"
    } else if cfg!(feature = "gpu") {
        "GPU"
    } else {
        "CPU"
    };

    let benchmark_json = &json!([{
        "name": format!("{backend}, {}", processor),
        "unit": "Mgas/s",
        "value": rate
    }]);
    let file = File::create("bench_latest.json").expect("failed to create bench_latest.json");
    serde_json::to_writer(file, benchmark_json).expect("failed to write to bench_latest.json");
}
