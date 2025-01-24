use bench::{
    constants::{CANCUN_CONFIG, MAINNET_CHAIN_ID},
    rpc::{db::RpcDB, get_block, get_latest_block_number},
};
use clap::Parser;
use ethrex_vm::{execution_db::ExecutionDB, spec_id};

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

    println!("populating rpc db cache");
    let rpc_db = RpcDB::with_cache(&rpc_url, block_number - 1, &block)
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
