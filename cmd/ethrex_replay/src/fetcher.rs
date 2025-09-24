use std::time::{Duration, SystemTime};

use ethrex_common::Address;
use ethrex_common::types::ChainConfig;
use ethrex_config::networks::Network;
use ethrex_levm::vm::VMType;
use ethrex_rpc::{
    EthClient,
    clients::{EthClientError, eth::errors::GetWitnessError},
    types::block_identifier::{BlockIdentifier, BlockTag},
};
use eyre::WrapErr;
use tracing::{debug, info, warn};

use crate::{
    cache::{Cache, get_block_cache_file_name},
    rpc::db::RpcDB,
};

#[cfg(feature = "l2")]
use crate::cache::L2Fields;
#[cfg(feature = "l2")]
use crate::cache::get_batch_cache_file_name;

pub async fn get_blockdata(
    eth_client: EthClient,
    network: Network,
    block_number: BlockIdentifier,
    fee_vault: Option<Address>,
) -> eyre::Result<Cache> {
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

    let file_name = get_block_cache_file_name(&network, requested_block_number, None);

    if let Ok(cache) = Cache::load(&file_name).inspect_err(|e| warn!("Failed to load cache: {e}")) {
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
        format_duration(block_retrieval_duration)
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
                requested_block_number - 1
            );
            let rpc_db = RpcDB::with_cache(
                eth_client.urls.first().unwrap().as_str(),
                chain_config,
                (requested_block_number - 1).try_into()?,
                &block,
                vm_type,
            )
            .await
            .wrap_err("failed to create rpc db")?;

            info!(
                "Pre executing block {}. This may take a while.",
                requested_block_number - 1
            );
            let rpc_db = rpc_db
                .to_execution_witness(&block, fee_vault)
                .wrap_err("failed to build execution db")?;
            info!(
                "Finished building execution witness for block {}",
                requested_block_number - 1
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
        format_duration(execution_witness_retrieval_duration)
    );

    Ok(Cache::new(vec![block], witness_rpc, Some(network)))
}

async fn fetch_rangedata_from_client(
    eth_client: EthClient,
    chain_config: ChainConfig,
    from: u64,
    to: u64,
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

    let execution_witness_retrieval_duration = execution_witness_retrieval_start_time
        .elapsed()
        .unwrap_or_else(|e| {
            panic!("SystemTime::elapsed failed: {e}");
        });

    info!(
        "Got execution witness for blocks {from} to {to} in {}",
        format_duration(execution_witness_retrieval_duration)
    );

    let network = Network::try_from(chain_config.chain_id).map_err(|e| {
        eyre::Error::msg(format!("Failed to determine network from chain ID: {}", e))
    })?;

    let cache = Cache::new(blocks, witness_rpc, Some(network));

    Ok(cache)
}

pub async fn get_rangedata(
    eth_client: EthClient,
    network: Network,
    from: u64,
    to: u64,
) -> eyre::Result<Cache> {
    let chain_config = network.get_genesis()?.config;

    let file_name = get_block_cache_file_name(&network, from, Some(to));

    if let Ok(cache) = Cache::load(&file_name) {
        info!("Getting block range data from cache");
        return Ok(cache);
    }

    info!("Getting block range data from RPC");

    let cache = fetch_rangedata_from_client(eth_client, chain_config, from, to).await?;

    cache.write()?;

    Ok(cache)
}

#[cfg(feature = "l2")]
pub async fn get_batchdata(
    rollup_client: EthClient,
    network: Network,
    batch_number: u64,
    fee_vault: Option<Address>,
) -> eyre::Result<Cache> {
    use ethrex_l2_rpc::clients::get_batch_by_number;

    let file_name = get_batch_cache_file_name(batch_number);
    if let Ok(cache) = Cache::load_cache(&file_name) {
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
        fee_vault,
    });

    cache.write_cache(&file_name)?;

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
