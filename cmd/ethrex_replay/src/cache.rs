use ethrex_common::types::blobs_bundle;
use ethrex_common::types::{Block, block_execution_witness::ExecutionWitnessResult};
use serde_with::serde_as;
use std::{
    fs::File,
    io::{BufReader, BufWriter},
};

use serde::{Deserialize, Serialize};

#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct Cache {
    pub blocks: Vec<Block>,
    pub witness: ExecutionWitnessResult,
    // L2 specific fields
    #[serde_as(as = "Option<[_; 48]>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob_commitment: Option<blobs_bundle::Commitment>,
    #[serde_as(as = "Option<[_; 48]>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob_proof: Option<blobs_bundle::Commitment>,
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
