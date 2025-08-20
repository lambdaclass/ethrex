use ethrex_common::types::Block;
use ethrex_common::types::blobs_bundle;
use ethrex_vm::prover_db::PreExecutionState;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::{
    fs::File,
    io::{BufReader, BufWriter},
};

#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct L2Fields {
    #[serde_as(as = "[_; 48]")]
    pub blob_commitment: blobs_bundle::Commitment,
    #[serde_as(as = "[_; 48]")]
    pub blob_proof: blobs_bundle::Proof,
}

#[derive(Serialize, Deserialize)]
pub struct Cache {
    pub blocks: Vec<Block>,
    pub pre_execution_state: PreExecutionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    pub l2_fields: Option<L2Fields>,
}

impl Cache {
    pub fn new(blocks: Vec<Block>, pre_execution_state: PreExecutionState) -> Self {
        Self {
            blocks,
            pre_execution_state,
            l2_fields: None,
        }
    }
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
    Ok(serde_json::to_writer_pretty(file, cache)?)
}
