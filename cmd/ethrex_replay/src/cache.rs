use std::{
    fs::File,
    io::{BufReader, BufWriter},
};

use ethrex_common::types::{Block, BlockHeader};
use ethrex_vm::ProverDB;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Cache {
    pub blocks: Vec<Block>,
    pub block_headers: Vec<BlockHeader>,
    pub db: ProverDB,
}

pub fn load_cache(block_number: usize) -> eyre::Result<Cache> {
    let file_name = format!("cache_{}.json", block_number);
    let file = BufReader::new(File::open(file_name)?);
    Ok(serde_json::from_reader(file)?)
}

pub fn write_cache(cache: &Cache) -> eyre::Result<()> {
    if cache.blocks.is_empty() {
        return Err(eyre::Error::msg("cache can't be empty"));
    }
    if cache.blocks.len() > 1 {
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
