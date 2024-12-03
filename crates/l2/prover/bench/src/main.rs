use std::collections::HashMap;

use bench::rpc::{get_account, get_block};
use clap::Parser;
use ethrex_vm::{execution_db::touched_state::get_touched_state, SpecId};
use futures_util::future::join_all;
use tokio_utils::RateLimiter;

const MAINNET_CHAIN_ID: u64 = 0x1;
const MAINNET_SPEC_ID: SpecId = SpecId::CANCUN;

const RPC_RATE_LIMIT: usize = 100; // requests per second

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

    println!("fetching block {block_number}");
    let block = get_block(&rpc_url, &block_number)
        .await
        .expect("failed to fetch block");

    println!("pre-executing transactions to get touched state");
    let touched_state = get_touched_state(&block, MAINNET_CHAIN_ID, MAINNET_SPEC_ID)
        .expect("failed to get touched state");

    println!("fetching touched state values");
    let mut accounts = HashMap::new();
    let mut storages = HashMap::new();

    let rate_limiter = RateLimiter::new(std::time::Duration::from_secs(1));
    let mut fetched_accs = 0;
    for request_chunk in touched_state.chunks(RPC_RATE_LIMIT) {
        // retrieve account state and its storage by fetching account proof
        let account_and_storage_futures =
            request_chunk.iter().map(|(address, storage_keys)| async {
                let request = get_account(
                    &rpc_url,
                    &block_number,
                    &address.clone(),
                    &storage_keys.clone(),
                )
                .await?;
                Ok(((*address, request.0), (*address, request.1)))
            });

        let account_and_storage = rate_limiter
            .throttle(|| async { join_all(account_and_storage_futures).await })
            .await
            .into_iter()
            .collect::<Result<(Vec<_>, Vec<_>), String>>()
            .expect("failed to fetch accounts and storage");

        let (account, storage) = account_and_storage;
        accounts.extend(account);
        storages.extend(
            storage.into_iter().map(|(address, storage)| {
                (address, storage.into_iter().collect::<HashMap<_, _>>())
            }),
        );

        fetched_accs += request_chunk.len();
        println!(
            "fetched {} accounts of {}",
            fetched_accs,
            touched_state.len()
        );
    }

    // 4. create prover program input and execute. Measure time.
    // 5. invoke rsp and execute too. Save in cache
}
