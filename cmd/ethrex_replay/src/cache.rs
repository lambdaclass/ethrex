use ethrex_common::types::Block;
use ethrex_common::types::blobs_bundle;
use ethrex_common::types::block_execution_witness::ExecutionWitness;
use eyre::Context;
use rkyv::rancor::Error;
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::io::Write;
use std::{fs::File, io::BufWriter};

#[serde_as]
#[derive(Serialize, Deserialize, RSerialize, RDeserialize, Archive, Clone)]
pub struct L2Fields {
    #[serde_as(as = "[_; 48]")]
    pub blob_commitment: blobs_bundle::Commitment,
    #[serde_as(as = "[_; 48]")]
    pub blob_proof: blobs_bundle::Proof,
}

#[derive(Serialize, Deserialize, RSerialize, RDeserialize, Archive, Clone)]
pub struct Cache {
    pub blocks: Vec<Block>,
    pub witness: ExecutionWitness,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    pub l2_fields: Option<L2Fields>,
}

impl Cache {
    pub fn new(blocks: Vec<Block>, witness: ExecutionWitness) -> Self {
        Self {
            blocks,
            witness,
            l2_fields: None,
        }
    }
}

pub fn load_cache(file_name: &str) -> eyre::Result<Cache> {
    let file_data = std::fs::read(file_name)?;
    let cache =
        rkyv::from_bytes::<Cache, Error>(&file_data).wrap_err("Failed to deserialize with rkyv")?;
    Ok(cache)
}

pub fn write_cache(cache: &Cache, file_name: &str) -> eyre::Result<()> {
    if cache.blocks.is_empty() {
        return Err(eyre::Error::msg("cache can't be empty"));
    }
    let mut file = BufWriter::new(File::create(file_name)?);
    let bytes = rkyv::to_bytes::<Error>(cache).wrap_err("Failed to serialize with rkyv")?;
    file.write_all(&bytes)
        .wrap_err("Failed to write binary data")
}
