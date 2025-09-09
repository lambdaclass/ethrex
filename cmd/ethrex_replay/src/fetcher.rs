use ethrex_rpc::{
    EthClient,
    types::block_identifier::{BlockIdentifier, BlockTag},
};
use std::time::{Duration, SystemTime};
use tracing::{debug, info, warn};

use crate::cache::{Cache, ReplayInput};
use ethrex_config::networks::Network;

#[cfg(feature = "l2")]
use crate::cache::L2Fields;
#[cfg(feature = "l2")]
use ethrex_rpc::debug::execution_witness::execution_witness_from_rpc_chain_config;

/// Gets necessary data to run a block with replay.
/// If it's cached it gets it that way, otherwise it gets it from an RPC endpoint and then caches it.
pub async fn get_blockdata(
    eth_client: EthClient,
    network: Network,
    block_number: BlockIdentifier,
) -> eyre::Result<ReplayInput> {
    // Get block number to compare requested block with last block in the chain
    let latest_block_number = eth_client.get_block_number().await?.as_u64();

    let requested_block_number = match block_number {
        BlockIdentifier::Number(some_number) => some_number,
        BlockIdentifier::Tag(BlockTag::Latest) => latest_block_number,
        BlockIdentifier::Tag(_) => unimplemented!("Only latest block tag is supported"),
    };

    info!(
        "Retrieving execution data for block {requested_block_number} ({} block behind latest)",
        latest_block_number - requested_block_number
    );

    let chain_config = network.get_genesis()?.config;

    // See if we have information for that block and network cached already.
    let file_name = format!("cache_{network}_{requested_block_number}.json");
    if let Ok(cache) = Cache::load(&file_name).inspect_err(|e| warn!("Failed to load cache: {e}")) {
        info!("Getting block {requested_block_number} data from cache");
        let input = cache.into_replay_input()?;
        return Ok(input);
    }

    debug!("Validating RPC chain ID");
    let chain_id = eth_client.get_chain_id().await?;
    if chain_id != chain_config.chain_id.into() {
        return Err(eyre::eyre!(
            "Rpc endpoint returned a different chain id than the one set by --network"
        ));
    }

    // Get block
    debug!("Getting block data from RPC for block {requested_block_number}");
    let block_retrieval_start_time = SystemTime::now();
    let block = eth_client
        .get_raw_block(BlockIdentifier::Number(requested_block_number))
        .await?;
    let block_retrieval_duration = block_retrieval_start_time.elapsed().unwrap_or_else(|e| {
        panic!("SystemTime::elapsed failed: {e}");
    });
    debug!(
        "Got block {requested_block_number} in {}",
        format_duration(block_retrieval_duration)
    );

    // Get witness
    debug!("Getting execution witness from RPC for block {requested_block_number}");
    let execution_witness_retrieval_start_time = SystemTime::now();
    let witness_rpc = match eth_client
        .get_witness(BlockIdentifier::Number(requested_block_number), None)
        .await
    {
        Ok(witness_rpc) => witness_rpc,
        Err(e) => {
            warn!("Failed to get witness from RPC: {e}");
            return Err(eyre::eyre!("Unimplemented: Retry with eth_getProofs"));
        }
    };
    let execution_witness_retrieval_duration =
        execution_witness_retrieval_start_time.elapsed().unwrap();
    debug!(
        "Got execution witness for block {requested_block_number} in {}",
        format_duration(execution_witness_retrieval_duration)
    );

    // Cache obtained information
    let cache = Cache::new(block, witness_rpc.clone(), network);
    Cache::write(&cache, &file_name).expect("failed to write cache");

    Ok(cache.into_replay_input()?)
}

/// This is only for L2, where there can be one witness for multiple blocks. Only compatible with Ethrex L2.
/// This doesn't implement a cache.
/// TODO: Maybe it could be a loop with get_blockdata and then we can merge the witnesses into one
#[cfg(feature = "l2")]
async fn fetch_rangedata_from_client(
    eth_client: EthClient,
    network: Network,
    from: u64,
    to: u64,
) -> eyre::Result<ReplayInput> {
    info!("Validating RPC chain ID");

    let chain_config = network.get_genesis()?.config; // TODO: remove unwrap

    let chain_id = eth_client.get_chain_id().await?;

    if chain_id != chain_config.chain_id.into() {
        return Err(eyre::eyre!(
            "Rpc endpoint returned a different chain id than the one set by --network"
        ));
    }

    let mut blocks = Vec::with_capacity((to - from + 1) as usize);

    info!(
        "Retrieving execution data for blocks {from} to {to} ({} blocks in total)",
        to - from + 1
    );

    let block_retrieval_start_time = SystemTime::now();

    for block_number in from..=to {
        let block = eth_client
            .get_raw_block(BlockIdentifier::Number(block_number))
            .await
            .wrap_err("failed to fetch block")?;
        blocks.push(block);
    }

    let block_retrieval_duration = block_retrieval_start_time.elapsed().unwrap_or_else(|e| {
        panic!("SystemTime::elapsed failed: {e}");
    });

    info!(
        "Got blocks {from} to {to} in {}",
        format_duration(block_retrieval_duration)
    );

    let from_identifier = BlockIdentifier::Number(from);

    let to_identifier = BlockIdentifier::Number(to);

    info!("Getting execution witness from RPC for blocks {from} to {to}");

    let execution_witness_retrieval_start_time = SystemTime::now();

    let witness_rpc = eth_client
        .get_witness(from_identifier, Some(to_identifier))
        .await
        .wrap_err("Failed to get execution witness for range")?;

    let witness = execution_witness_from_rpc_chain_config(witness_rpc.clone(), chain_config, from)
        .expect("Failed to convert witness");

    let execution_witness_retrieval_duration = execution_witness_retrieval_start_time
        .elapsed()
        .unwrap_or_else(|e| {
            panic!("SystemTime::elapsed failed: {e}");
        });

    info!(
        "Got execution witness for blocks {from} to {to} in {}",
        format_duration(execution_witness_retrieval_duration)
    );

    let replay_input = ReplayInput {
        blocks,
        witness,
        l2_fields: None, // This will be filled out later if it's an L2 batch.
    };

    Ok(replay_input)
}

#[cfg(feature = "l2")]
pub async fn get_batchdata(
    rollup_client: EthClient,
    network: Network,
    batch_number: u64,
) -> eyre::Result<Cache> {
    info!("Getting batch data from RPC");

    let rpc_batch = get_batch_by_number(&rollup_client, batch_number).await?;

    let mut input = fetch_rangedata_from_client(
        rollup_client,
        network.get_genesis()?.config,
        rpc_batch.batch.first_block,
        rpc_batch.batch.last_block,
    )
    .await?;

    // If the l2 node is in validium it does not return blobs to prove
    input.l2_fields = Some(L2Fields {
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

    Ok(replay_input)
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
