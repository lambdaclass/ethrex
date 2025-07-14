use ethrex_common::types::ChainConfig;
use ethrex_rpc::{EthClient, types::block_identifier::BlockIdentifier};
use eyre::WrapErr;

use crate::cache::{Cache, load_cache, write_cache};

pub async fn get_blockdata(
    eth_client: EthClient,
    chain_config: ChainConfig,
    block_number: BlockIdentifier,
) -> eyre::Result<Cache> {
    let file_name = format!("cache_{block_number}.json");
    if let Ok(cache) = load_cache(&file_name) {
        return Ok(cache);
    }
    let block = eth_client.get_raw_block(block_number.clone()).await?;

    println!("populating rpc db cache");
    let witness = eth_client.get_witness(block_number, None).await?;
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
    eth_client: EthClient,
    chain_config: ChainConfig,
    from: usize,
    to: usize,
) -> eyre::Result<Cache> {
    let file_name = format!("cache_{from}-{to}.json");
    if let Ok(cache) = load_cache(&file_name) {
        return Ok(cache);
    }
    let mut blocks = Vec::with_capacity(to - from);
    for block_number in from..=to {
        let block = eth_client
            .get_raw_block(BlockIdentifier::Number(block_number.try_into()?))
            .await
            .wrap_err("failed to fetch block")?;
        blocks.push(block);
    }

    let from_identifier = BlockIdentifier::Number(from.try_into()?);
    let to_identifier = BlockIdentifier::Number(to.try_into()?);

    let witness = eth_client
        .get_witness(from_identifier, Some(to_identifier))
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
