use bench::rpc::{get_account, get_block};
use clap::Parser;
use ethrex_vm::{execution_db::touched_state::get_touched_state, SpecId};
use futures_util::future::join_all;

const MAINNET_CHAIN_ID: u64 = 0x1;
const MAINNET_SPEC_ID: SpecId = SpecId::CANCUN;

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

    // fetch block
    let block = get_block(&rpc_url, &block_number)
        .await
        .expect("failed to fetch block");

    // get all accounts and storage keys touched during execution of block
    let touched_state = get_touched_state(&block, MAINNET_CHAIN_ID, MAINNET_SPEC_ID)
        .expect("failed to get touched state");

    // fetch all accounts and storage touched
    let _accounts = join_all(
        touched_state
            .iter()
            .map(|(address, _)| get_account(&rpc_url, &block_number, address)),
    )
    .await
    .into_iter()
    .collect::<Result<Vec<_>, String>>()
    .expect("failed to fetch accounts");
    // TODO: storage

    // 4. create prover program input and execute. Measure time.
    // 5. invoke rsp and execute too. Save in cache
}
