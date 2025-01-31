use ethrex_rpc_client::{db::RpcDB, get_block, get_latest_block_number};
use ethrex_vm::execution_db::ToExecDB;
use ethrex_vm::{execute_block, EvmState};
use std::{fs::File, io::Write};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rpc_url = args.get(1).expect("rpc_url not provided");
    let block_number = get_latest_block_number(&rpc_url).await.unwrap();

    println!("fetching block {block_number} and its parent header");
    let block = get_block(&rpc_url, block_number)
        .await
        .expect("failed to fetch block");
    //dbg!(&block);

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
        let exec_db = rpc_db
            .to_exec_db(&block)
            .expect("failed to build execution db");

        println!("writing db to file db.bin");
        let mut file = File::create("db.bin").expect("failed to create db file");
        file.write_all(
            bincode::serialize(&exec_db)
                .expect("failed to serialize db")
                .as_slice(),
        )
        .expect("failed to write db to file");

        exec_db
    };
    //dbg!(&exec_db);

    let mut evm_state = EvmState::from(exec_db);
    let before = std::time::Instant::now();
    let res = execute_block(&block, &mut evm_state).unwrap();
    let after = std::time::Instant::now();
    dbg!(&res);
    println!("Execution time: {:?}", after - before);
}
