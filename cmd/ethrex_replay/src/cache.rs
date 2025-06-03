use std::{
    fs::File,
    io::{BufReader, BufWriter},
};

use ethrex_common::types::{Block, BlockHeader, ChainConfig};
use ethrex_rpc::types::block_execution_witness::ExecutionWitnessResult;
use ethrex_vm::ProverDB;

use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize};

use crate::fetcher::to_exec_db_from_witness;

#[derive(Serialize)]
pub struct Cache {
    pub blocks: Vec<Block>,
    pub parent_block_header: BlockHeader,
    pub witness: ExecutionWitnessResult,
    pub chain_config: ChainConfig,
    #[serde(skip)]
    pub db: ProverDB,
}

impl<'de> Deserialize<'de> for Cache {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct CacheWithoutDb {
            blocks: Vec<Block>,
            parent_block_header: BlockHeader,
            witness: ExecutionWitnessResult,
            pub chain_config: ChainConfig,
        }

        let CacheWithoutDb {
            blocks,
            parent_block_header,
            witness,
            chain_config,
        } = CacheWithoutDb::deserialize(deserializer)?;

        // Recreate the db using the deserialized data
        let db = to_exec_db_from_witness(chain_config, &witness)
            .map_err(|e| D::Error::custom(format!("Failed to rebuild prover db {e}")))?;

        Ok(Cache {
            blocks,
            parent_block_header,
            witness,
            chain_config,
            db,
        })
    }
}

pub fn load_cache(block_number: usize) -> eyre::Result<Cache> {
    let file_name = format!("cache_{}.json", block_number);
    let file = BufReader::new(File::open(file_name)?);
    Ok(serde_json::from_reader(file)?)
}

pub fn write_cache(cache: &Cache) -> eyre::Result<()> {
    if cache.blocks.len() != 1 {
        return Err(eyre::Error::msg("trying to save a multi-block cache"));
    }
    let file_name = format!("cache_{}.json", cache.blocks[0].header.number);
    let file = BufWriter::new(File::create(file_name)?);
    Ok(serde_json::to_writer(file, cache)?)
}

pub fn load_cache_batch(from: usize, to: usize) -> eyre::Result<Cache> {
    let file_name = format!("cache_{}-{}.json", from, to);
    let file = BufReader::new(File::open(file_name)?);
    Ok(serde_json::from_reader(file)?)
}

pub fn write_cache_batch(cache: &Cache) -> eyre::Result<()> {
    let from = cache
        .blocks
        .first()
        .ok_or(eyre::Error::msg("cache is empty"))?
        .header
        .number;
    let to = cache
        .blocks
        .last()
        .ok_or(eyre::Error::msg("cache is empty"))?
        .header
        .number;
    let file_name = format!("cache_{}-{}.json", from, to);
    let file = BufWriter::new(File::create(file_name)?);
    Ok(serde_json::to_writer(file, cache)?)
}
