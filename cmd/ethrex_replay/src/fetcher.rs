use std::time::{Duration, SystemTime};

use ethrex_common::types::ChainConfig;
use ethrex_rpc::{
    EthClient,
    types::block_identifier::{BlockIdentifier, BlockTag},
};
use eyre::WrapErr;
use tracing::info;

use crate::cache::{Cache, L2Fields, load_cache, write_cache};

pub async fn get_blockdata(
    eth_client: EthClient,
    chain_config: ChainConfig,
    block_number: BlockIdentifier,
) -> eyre::Result<Cache> {
    let requested_block_number = match block_number {
        BlockIdentifier::Number(some_number) => some_number,
        BlockIdentifier::Tag(BlockTag::Latest) => eth_client.get_block_number().await?.as_u64(),
        BlockIdentifier::Tag(_) => unimplemented!("Only latest block tag is supported"),
    };

    let file_name = format!("cache_{requested_block_number}.json");

    if let Ok(cache) = load_cache(&file_name, chain_config) {
        info!("Getting block {requested_block_number} data from cache");
        return Ok(cache);
    }

    info!("Validating RPC chain ID");

    let chain_id = eth_client.get_chain_id().await?;

    if chain_id != chain_config.chain_id.into() {
        return Err(eyre::eyre!(
            "Rpc endpoint returned a different chain id than the one set by --network"
        ));
    }

    info!("Getting execution witness from RPC for block {requested_block_number}");

    let execution_witness_retrieval_start_time = SystemTime::now();

    let mut witness = match eth_client.get_witness(block_number.clone(), None).await {
        Ok(witness) => witness,
        Err(_e) => {
            todo!("Retry with eth_getProofs")
        }
    };

    let execution_witness_retrieval_duration = execution_witness_retrieval_start_time
        .elapsed()
        .unwrap_or_else(|e| {
            panic!("SystemTime::elapsed failed: {e}");
        });

    info!(
        "Got execution witness for block {requested_block_number} in {}",
        format_duration(execution_witness_retrieval_duration)
    );

    // TODO: Make sure other ExecutionWitness users use the correct chain config.
    witness.chain_config = chain_config;

    info!("Getting block data from RPC for block {requested_block_number}");

    let block_retrieval_start_time = SystemTime::now();

    let block = eth_client
        .get_raw_block(BlockIdentifier::Number(requested_block_number))
        .await?;

    let block_retrieval_duration = block_retrieval_start_time.elapsed().unwrap_or_else(|e| {
        panic!("SystemTime::elapsed failed: {e}");
    });

    info!(
        "Got block {requested_block_number} in {}",
        format_duration(block_retrieval_duration)
    );

    info!("Caching block {requested_block_number}");

    let block_cachh_start_time = SystemTime::now();

    let cache = Cache::new(vec![block], witness);

    write_cache(&cache, &file_name).expect("failed to write cache");

    let block_cache_duration = block_cachh_start_time.elapsed().unwrap_or_else(|e| {
        panic!("SystemTime::elapsed failed: {e}");
    });

    info!(
        "Cached block {requested_block_number} in {}",
        format_duration(block_cache_duration)
    );

    Ok(cache)
}

async fn fetch_rangedata_from_client(
    eth_client: EthClient,
    chain_config: ChainConfig,
    from: usize,
    to: usize,
) -> eyre::Result<Cache> {
    let mut blocks = Vec::with_capacity(to - from + 1);
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

    let cache = Cache::new(blocks, witness);
    Ok(cache)
}

pub async fn get_rangedata(
    eth_client: EthClient,
    chain_config: ChainConfig,
    from: usize,
    to: usize,
) -> eyre::Result<Cache> {
    let file_name = format!("cache_{from}-{to}.json");
    if let Ok(cache) = load_cache(&file_name, chain_config) {
        info!("Getting block range data from cache");
        return Ok(cache);
    }
    info!("Getting block range data from RPC");
    let cache = fetch_rangedata_from_client(eth_client, chain_config, from, to).await?;

    write_cache(&cache, &file_name).expect("failed to write cache");

    Ok(cache)
}

pub async fn get_batchdata(
    eth_client: EthClient,
    chain_config: ChainConfig,
    batch_number: u64,
) -> eyre::Result<Cache> {
    let file_name = format!("cache_batch_{batch_number}.json");
    if let Ok(cache) = load_cache(&file_name, chain_config) {
        info!("Getting batch data from cache");
        return Ok(cache);
    }
    info!("Getting batch data from RPC");

    let rpc_batch = eth_client.get_batch_by_number(batch_number).await?;

    let mut cache = fetch_rangedata_from_client(
        eth_client,
        chain_config,
        rpc_batch.batch.first_block as usize,
        rpc_batch.batch.last_block as usize,
    )
    .await?;

    // If the l2 node is in validium it does not return blobs to prove
    cache.l2_fields = Some(L2Fields {
        blob_commitment: *rpc_batch
            .batch
            .blobs_bundle
            .commitments
            .first()
            .unwrap_or(&[0_u8; 48]),
        blob_proof: *rpc_batch
            .batch
            .blobs_bundle
            .proofs
            .first()
            .unwrap_or(&[0_u8; 48]),
    });

    write_cache(&cache, &file_name).expect("failed to write cache");

    Ok(cache)
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    let milliseconds = duration.subsec_millis();
    
    if minutes == 0 {
        return format!("{seconds:02}s {milliseconds:03}ms");
    }

    format!("{minutes:02}m {seconds:02}s")
}
