use crate::cache::{load_cache, write_cache, Cache};
use crate::rpc::{db::RpcDB, get_block, get_latest_block_number};
use ethrex_common::types::ChainConfig;
use eyre::{ContextCompat, WrapErr};

pub async fn or_latest(maybe_number: Option<usize>, rpc_url: &str) -> eyre::Result<usize> {
    Ok(match maybe_number {
        Some(v) => v,
        None => get_latest_block_number(rpc_url).await?,
    })
}

pub async fn get_blockdata(
    rpc_url: String,
    chain_config: ChainConfig,
    block_number: usize,
) -> eyre::Result<Cache> {
    if let Ok(cache) = load_cache(block_number) {
        return Ok(cache);
    }
    let block = get_block(&rpc_url, block_number)
        .await
        .wrap_err("failed to fetch block")?;

    println!("populating rpc db cache");
    let rpc_db = RpcDB::with_cache(&rpc_url, chain_config, block_number - 1, &block)
        .await
        .wrap_err("failed to create rpc db")?;

    let db = rpc_db
        .to_exec_db(&block)
        .wrap_err("failed to build execution db")?;

    let mut block_headers = Vec::new();
    let oldest_required_block_number = db
        .block_hashes
        .keys()
        .min()
        .wrap_err("no block hashes required (should at least contain parent hash)")?;
    // from oldest required to parent:
    for number in *oldest_required_block_number..block.header.number {
        let number: usize = number
            .try_into()
            .wrap_err("failed to convert block number to usize from u64")?;
        let header = get_block(&rpc_url, number)
            .await
            .wrap_err("failed to fetch block")?
            .header;
        block_headers.push(header);
    }

    let cache = Cache {
        block,
        block_headers,
        db,
    };
    write_cache(&cache).expect("failed to write cache");
    Ok(cache)
}
