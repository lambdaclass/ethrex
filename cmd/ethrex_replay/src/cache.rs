use std::{
    fs::File,
    io::{BufReader, BufWriter},
};

use ethrex_common::types::{Block, block_execution_witness::ExecutionWitnessResult};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Cache {
    pub blocks: Vec<Block>,
    pub witness: ExecutionWitnessResult,
}

pub fn load_cache(file_name: &str) -> eyre::Result<Cache> {
    let file = BufReader::new(File::open(file_name)?);
    Ok(serde_json::from_reader(file)?)
}

pub fn write_cache(cache: &Cache, file_name: &str) -> eyre::Result<()> {
    if cache.blocks.is_empty() {
        return Err(eyre::Error::msg("cache can't be empty"));
    }
    let file = BufWriter::new(File::create(file_name)?);
    Ok(serde_json::to_writer(file, cache)?)
}
