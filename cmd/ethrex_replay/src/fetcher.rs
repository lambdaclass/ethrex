use ethrex_common::types::ChainConfig;
use eyre::WrapErr;

use crate::{
    cache::{Cache, load_cache, write_cache},
    rpc::{get_block, get_latest_block_number, get_witness, get_witness_range},
};

pub async fn or_latest(maybe_number: Option<usize>, rpc_url: &str) -> eyre::Result<usize> {
    Ok(match maybe_number {
        Some(v) => v,
        None => get_latest_block_number(rpc_url).await?,
    })
}

pub async fn get_blockdata(
    rpc_url: &str,
    chain_config: ChainConfig,
    block_number: usize,
) -> eyre::Result<Cache> {
    let file_name = format!("cache_{}.json", block_number);
    if let Ok(cache) = load_cache(&file_name) {
        return Ok(cache);
    }
    let block = get_block(rpc_url, block_number)
        .await
        .wrap_err("failed to fetch block")?;

    println!("populating rpc db cache");
    let witness = get_witness(rpc_url, block_number)
        .await
        .wrap_err("Failed to get execution witness")?;
    if witness.chain_config.chain_id != chain_config.chain_id {
        return Err(eyre::eyre!(
            "Rpc endpoint returned a different chain id than the one set by --network"
        ));
    }

    let cache = Cache {
        blocks: vec![block],
        witness,
    };
    write_cache(&cache, &file_name).expect("failed to write cache");
    Ok(cache)
}

pub async fn get_rangedata(
    rpc_url: &str,
    chain_config: ChainConfig,
    from: usize,
    to: usize,
) -> eyre::Result<Cache> {
    let file_name = format!("cache_{}-{}.json", from, to);
    if let Ok(cache) = load_cache(&file_name) {
        return Ok(cache);
    }
    let mut blocks = Vec::with_capacity(to - from);
    for block_number in from..=to {
        let block = get_block(rpc_url, block_number)
            .await
            .wrap_err("failed to fetch block")?;
        blocks.push(block);
    }

    let witness = get_witness_range(rpc_url, from, to)
        .await
        .wrap_err("Failed to get execution witness for range")?;
    if witness.chain_config.chain_id != chain_config.chain_id {
        return Err(eyre::eyre!(
            "Rpc endpoint returned a different chain id than the one set by --network"
        ));
    }

    let cache = Cache { blocks, witness };

    write_cache(&cache, &file_name).expect("failed to write cache");

    Ok(cache)
}
