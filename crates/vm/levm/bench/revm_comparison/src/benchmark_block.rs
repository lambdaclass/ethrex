use ethrex_rpc_client::constants::CANCUN_CONFIG;
use ethrex_rpc_client::db::RpcDB;
use ethrex_rpc_client::{get_block, get_latest_block_number};
use ethrex_vm::execution_db::ExecutionDB;
use ethrex_vm::{execute_block, spec_id, EvmState};
use revm::db::CacheDB;
use std::{fs::File, io::Write};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rpc_url = args.get(1).expect("rpc_url not provided");
    let block_number = get_latest_block_number(rpc_url).await.unwrap();

    println!("fetching block {block_number} and its parent header");
    let block = get_block(rpc_url, block_number)
        .await
        .expect("failed to fetch block");
    //dbg!(&block);

    let chain_config = CANCUN_CONFIG;
    let rpc_db = if let Some(db) = RpcDB::deserialize_from_file("db.bin") {
        println!("db file found");
        db
    } else {
        println!("db file not found");

        println!("populating rpc db cache");
        let rpc_db = RpcDB::with_cache(rpc_url, block_number - 1, &block)
            .await
            .expect("failed to create rpc db");

        println!("pre-executing to build execution db");
        let cache_db = ExecutionDB::pre_execute(
            &block,
            chain_config.chain_id,
            spec_id(&chain_config, block.header.timestamp),
            rpc_db,
        )
        .unwrap();
        let rpc_db = cache_db.db;

        println!("writing db to file db.bin");
        rpc_db
            .serialize_to_file("db.bin")
            .expect("failed to serialize db");

        rpc_db
    };
    let store = rpc_db
        .to_in_memory_store(block.clone(), &chain_config)
        .expect("failed to build execution db");

    dbg!(&store);

    let mut evm_state = EvmState::from(store);
    let before = std::time::Instant::now();
    let res = execute_block(&block, &mut evm_state).unwrap();
    let after = std::time::Instant::now();
    dbg!(&res);
    println!("Execution time: {:?}", after - before);
}
