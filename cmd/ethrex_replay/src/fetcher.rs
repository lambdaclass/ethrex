use ethrex_config::networks::Network;
use ethrex_levm::vm::VMType;
use ethrex_rpc::{
    EthClient,
    clients::{EthClientError, eth::errors::GetWitnessError},
    types::block_identifier::{BlockIdentifier, BlockTag},
};
use eyre::{OptionExt, WrapErr};
use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};
use tracing::{debug, info, warn};

use crate::{
    cache::{Cache, get_block_cache_file_name},
    cli::{EthrexReplayOptions, setup_rpc},
    rpc::db::RpcDB,
};

#[cfg(feature = "l2")]
use crate::cache::L2Fields;
#[cfg(feature = "l2")]
use crate::cache::get_batch_cache_file_name;

pub async fn get_blockdata(
    opts: EthrexReplayOptions,
    block: Option<u64>,
) -> eyre::Result<(Cache, Network)> {
    if opts.cached {
        let network = opts
            .network
            .clone()
            .ok_or_eyre("Network must be specified in cached mode")?;
        let requested_block_number =
            block.ok_or_eyre("Block number must be specified in cached mode")?;

        let file_name = get_block_cache_file_name(&network, requested_block_number, None);
        info!("Getting block {requested_block_number} data from cache");
        let cache = Cache::load(&opts.cache_dir, &file_name).map_err(|e| {
            eyre::eyre!("Cache wasn't found for block {requested_block_number}: {e}")
        })?;
        Ok((cache, network))
    } else {
        let (eth_client, rpc_network) = setup_rpc(&opts).await?;
        if let Some(network) = &opts.network {
            if network != &rpc_network {
                return Err(eyre::eyre!(
                    "Specified network ({}) does not match RPC network ({})",
                    network,
                    rpc_network
                ));
            }
        }
        let block_identifier = match block {
            Some(n) => BlockIdentifier::Number(n),
            None => BlockIdentifier::Tag(BlockTag::Latest),
        };
        let cache = get_blockdata_rpc(
            eth_client,
            rpc_network.clone(),
            block_identifier,
            opts.cache_dir.clone(),
        )
        .await?;

        // Always write the cache after fetching from RPC.
        // It will be deleted later if not needed.
        cache.write()?;

        Ok((cache, rpc_network))
    }
}

/// Retrieves data from RPC
async fn get_blockdata_rpc(
    eth_client: EthClient,
    network: Network,
    block_identifier: BlockIdentifier,
    cache_dir: PathBuf,
) -> eyre::Result<Cache> {
    let latest_block_number = eth_client.get_block_number().await?.as_u64();

    let requested_block_number = match block_identifier {
        BlockIdentifier::Number(some_number) => some_number,
        BlockIdentifier::Tag(BlockTag::Latest) => latest_block_number,
        BlockIdentifier::Tag(_) => unimplemented!("Only latest block tag is supported"),
    };

    info!(
        "Retrieving execution data for block {requested_block_number} ({} block behind latest)",
        latest_block_number - requested_block_number
    );

    let chain_config = network.get_genesis()?.config;

    let file_name = get_block_cache_file_name(&network, requested_block_number, None);
    if let Ok(cache) =
        Cache::load(&cache_dir, &file_name).inspect_err(|e| warn!("Failed to load cache: {e}"))
    {
        info!("Getting block {requested_block_number} data from cache");
        return Ok(cache);
    }

    debug!("Validating RPC chain ID");

    let chain_id = eth_client.get_chain_id().await?;

    if chain_id != chain_config.chain_id.into() {
        return Err(eyre::eyre!(
            "Rpc endpoint returned a different chain id than the one set by --network"
        ));
    }

    debug!("Getting block data from RPC for block {requested_block_number}");

    let block_retrieval_start_time = SystemTime::now();

    let rpc_block = eth_client
        .get_block_by_number(BlockIdentifier::Number(requested_block_number), true)
        .await
        .wrap_err("Failed to retrieve requested block")?;

    let block = rpc_block
        .try_into()
        .map_err(|e| eyre::eyre!("{}", e))
        .wrap_err("Failed to convert from rpc block to block")?;

    let block_retrieval_duration = block_retrieval_start_time.elapsed().unwrap_or_else(|e| {
        panic!("SystemTime::elapsed failed: {e}");
    });

    debug!(
        "Got block {requested_block_number} in {}",
        format_duration(&block_retrieval_duration)
    );

    debug!("Getting execution witness from RPC for block {requested_block_number}");

    let execution_witness_retrieval_start_time = SystemTime::now();

    let witness_rpc = match eth_client
        .get_witness(BlockIdentifier::Number(requested_block_number), None)
        .await
    {
        Ok(witness) => witness,
        Err(EthClientError::GetWitnessError(GetWitnessError::RPCError(_))) => {
            warn!("debug_executionWitness endpoint not implemented, using fallback eth_getProof");

            #[cfg(feature = "l2")]
            let vm_type = VMType::L2;
            #[cfg(not(feature = "l2"))]
            let vm_type = VMType::L1;

            info!(
                "Caching callers and recipients state for block {}",
                requested_block_number
            );
            let rpc_db = RpcDB::with_cache(
                eth_client
                    .urls
                    .first()
                    .ok_or_eyre("No RPC URLs configured")?
                    .as_str(),
                chain_config,
                requested_block_number,
                &block,
                vm_type,
            )
            .await
            .wrap_err("failed to create rpc db")?;

            info!(
                "Pre executing block {}. This may take a while.",
                requested_block_number
            );
            let rpc_db = rpc_db
                .to_execution_witness(&block)
                .wrap_err("failed to build execution db")?;
            info!(
                "Finished building execution witness for block {}",
                requested_block_number
            );
            rpc_db
        }
        Err(e) => {
            return Err(eyre::eyre!(format!(
                "Unexpected response from debug_executionWitness: {e}"
            )));
        }
    };

    let execution_witness_retrieval_duration = execution_witness_retrieval_start_time
        .elapsed()
        .unwrap_or_else(|e| {
            panic!("SystemTime::elapsed failed: {e}");
        });

    debug!(
        "Got execution witness for block {requested_block_number} in {}",
        format_duration(&execution_witness_retrieval_duration)
    );

    Ok(Cache::new(
        vec![block],
        witness_rpc,
        chain_config,
        cache_dir,
    ))
}

#[cfg(feature = "l2")]
use ethrex_common::types::ChainConfig;

#[cfg(feature = "l2")]
async fn fetch_rangedata_from_client(
    eth_client: EthClient,
    chain_config: ChainConfig,
    from: u64,
    to: u64,
    dir: PathBuf,
) -> eyre::Result<Cache> {
    info!("Validating RPC chain ID");

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
        let rpc_block = eth_client
            .get_block_by_number(BlockIdentifier::Number(block_number), true)
            .await
            .wrap_err(format!("failed to fetch block {block_number}"))?;

        let block = rpc_block
            .try_into()
            .map_err(|e| eyre::eyre!("Failed to convert rpc block to block: {}", e))
            .wrap_err("Failed to convert from rpc block to block")?;
        blocks.push(block);
    }

    let block_retrieval_duration = block_retrieval_start_time.elapsed().unwrap_or_else(|e| {
        panic!("SystemTime::elapsed failed: {e}");
    });

    info!(
        "Got blocks {from} to {to} in {}",
        format_duration(&block_retrieval_duration)
    );

    let from_identifier = BlockIdentifier::Number(from);

    let to_identifier = BlockIdentifier::Number(to);

    info!("Getting execution witness from RPC for blocks {from} to {to}");

    let execution_witness_retrieval_start_time = SystemTime::now();

    let witness_rpc = eth_client
        .get_witness(from_identifier, Some(to_identifier))
        .await
        .wrap_err("Failed to get execution witness for range")?;

    let execution_witness_retrieval_duration = execution_witness_retrieval_start_time
        .elapsed()
        .unwrap_or_else(|e| {
            panic!("SystemTime::elapsed failed: {e}");
        });

    info!(
        "Got execution witness for blocks {from} to {to} in {}",
        format_duration(&execution_witness_retrieval_duration)
    );

    let cache = Cache::new(blocks, witness_rpc, chain_config, dir);

    Ok(cache)
}

#[cfg(feature = "l2")]
pub async fn get_batchdata(
    rollup_client: EthClient,
    network: Network,
    batch_number: u64,
    cache_dir: PathBuf,
) -> eyre::Result<Cache> {
    use ethrex_l2_rpc::clients::get_batch_by_number;

    let file_name = get_batch_cache_file_name(batch_number);
    if let Ok(cache) = Cache::load(&cache_dir, &file_name) {
        info!("Getting batch data from cache");
        return Ok(cache);
    }
    info!("Getting batch data from RPC");

    let rpc_batch = get_batch_by_number(&rollup_client, batch_number).await?;

    let mut cache = fetch_rangedata_from_client(
        rollup_client,
        network.get_genesis()?.config,
        rpc_batch.batch.first_block,
        rpc_batch.batch.last_block,
        cache_dir,
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

    cache.write()?;

    Ok(cache)
}

fn format_duration(duration: &Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    let milliseconds = duration.subsec_millis();

    if hours > 0 {
        return format!("{hours:02}h {minutes:02}m {seconds:02}s {milliseconds:03}ms");
    }

    if minutes == 0 {
        return format!("{seconds:02}s {milliseconds:03}ms");
    }

    format!("{minutes:02}m {seconds:02}s")
}
