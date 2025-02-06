use clap::Parser;
use ethrex_rpc_client::db::RpcDB;
use ethrex_rpc_client::{get_block, get_latest_block_number};
use ethrex_vm::execution_db::ToExecDB;
use ethrex_vm::{execute_block, EvmState};
use revm::primitives::hex;
use revm::Database;
use std::hash::Hash;
use std::{fs::File, io::Write};

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

    let exec_db = if let Ok(file) = File::open("db.bin") {
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

    let mut evm_state = EvmState::from(exec_db);
    cfg_if::cfg_if! {
        if #[cfg(feature = "levm")] {
            let before = std::time::Instant::now();
            let (receipts, _updates) = execute_block(&block, &mut evm_state).unwrap();
            let after = std::time::Instant::now();

            let last_receipt = receipts.last().unwrap();
            let hashed_receipt = hex::encode(last_receipt.encode_inner());
            println!("Execution time: {:?}", after - before);
            println!("Receipt hash: 0x{}", hashed_receipt);
        } else {
            let before = std::time::Instant::now();
            let receipts = execute_block(&block, &mut evm_state).unwrap();
            let after = std::time::Instant::now();

            let last_receipt = receipts.last().unwrap();
            let hashed_receipt = hex::encode(last_receipt.encode_inner());
            println!("Execution time: {:?}", after - before);
            println!("Receipt hash: 0x{}", hashed_receipt);
        }
    }
}
