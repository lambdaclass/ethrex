use std::collections::{HashMap, HashSet};
use std::hash::RandomState;

use crate::cache::{load_cache, load_cache_batch, write_cache, write_cache_batch, Cache};
use crate::rpc::{db::RpcDB, get_block, get_latest_block_number};
use ethrex_common::types::ChainConfig;
use ethrex_common::{Address, H256};
use eyre::WrapErr;

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
    if let Ok(cache) = load_cache(block_number) {
        return Ok(cache);
    }
    let block = get_block(rpc_url, block_number)
        .await
        .wrap_err("failed to fetch block")?;

    let parent_block_header = get_block(rpc_url, block_number - 1)
        .await
        .wrap_err("failed to fetch block")?
        .header;

    println!("populating rpc db cache");
    let rpc_db = RpcDB::with_cache(rpc_url, chain_config, block_number - 1, &block)
        .await
        .wrap_err("failed to create rpc db")?;

    let db = rpc_db
        .to_exec_db(&block)
        .wrap_err("failed to build execution db")?;

    let cache = Cache {
        blocks: vec![block],
        parent_block_header,
        db,
    };
    write_cache(&cache).expect("failed to write cache");
    Ok(cache)
}

pub async fn get_rangedata(
    rpc_url: &str,
    chain_config: ChainConfig,
    from: usize,
    to: usize,
) -> eyre::Result<Cache> {
    if let Ok(cache) = load_cache_batch(from, to) {
        return Ok(cache);
    }
    let mut blocks = Vec::with_capacity(to - from);
    for block_number in from..=to {
        let data = get_blockdata(rpc_url, chain_config, block_number).await?;
        blocks.push(data);
    }
    let first_block = &blocks[0].blocks[0];
    let rpc_db = RpcDB::new(rpc_url, chain_config, from - 1);
    let mut used: HashMap<Address, HashSet<H256>> = HashMap::new();
    for block_data in blocks.iter() {
        for account in block_data.db.accounts.keys() {
            used.entry(*account).or_default();
        }
        for (account, storage) in block_data.db.storage.iter() {
            let slots = used.entry(*account).or_default();
            slots.extend(storage.keys());
        }
    }
    let to_fetch: Vec<(Address, Vec<H256>)> = used
        .into_iter()
        .map(|(address, storages)| (address, storages.into_iter().collect()))
        .collect();
    rpc_db.load_accounts(&to_fetch).await?;
    let mut proverdb = rpc_db.to_exec_db(first_block)?;
    proverdb.block_hashes = blocks
        .iter()
        .flat_map(|cache| cache.db.block_hashes.clone())
        .collect();
    for block_data in blocks.iter() {
        proverdb
            .state_proofs
            .1
            .extend(block_data.db.state_proofs.1.clone());
        for (account, proofs) in block_data.db.storage_proofs.iter() {
            let entry = proverdb.storage_proofs.entry(*account).or_default();
            entry.1.extend(proofs.1.clone());
        }
    }
    dedup_proofs(&mut proverdb.state_proofs.1);
    for (_, proofs) in proverdb.storage_proofs.iter_mut() {
        dedup_proofs(&mut proofs.1);
    }
    let cache = Cache {
        blocks: blocks.iter().map(|cache| cache.blocks[0].clone()).collect(),
        parent_block_header: blocks[0].parent_block_header.clone(),
        db: proverdb,
    };
    write_cache_batch(&cache)?;
    Ok(cache)
}

fn dedup_proofs(proofs: &mut Vec<Vec<u8>>) {
    let mut seen: HashSet<Vec<u8>, RandomState> = HashSet::from_iter(proofs.drain(..));
    *proofs = seen.drain().collect();
}
