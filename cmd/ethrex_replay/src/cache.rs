use ethrex_common::types::Block;
use ethrex_common::types::blobs_bundle;
use ethrex_common::types::block_execution_witness::ExecutionWitness;
use eyre::Context;
use eyre::OptionExt;
use rkyv::rancor::Error;
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::io::Write;
use std::{fs::File, io::BufWriter};

use crate::cli::network_from_chain_id;

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

pub fn get_block_cache_file_name(chain_id: u64, from: u64, to: Option<u64>, l2: bool) -> String {
    let network = network_from_chain_id(chain_id, l2);

    if let Some(to) = to {
        format!("cache_{network}_{from}-{to}.bin")
    } else {
        format!("cache_{network}_{from}.bin")
    }
}

#[cfg(feature = "l2")]
pub fn get_batch_cache_file_name(batch_number: u64) -> String {
    format!("cache_batch_{batch_number}.bin")
}

pub fn load_cache(file_name: &str) -> eyre::Result<Cache> {
    let file_data = std::fs::read(file_name)?;
    let cache =
        rkyv::from_bytes::<Cache, Error>(&file_data).wrap_err("Failed to deserialize with rkyv")?;
    Ok(cache)
}

pub fn write_cache(cache: &Cache, l2: bool) -> eyre::Result<()> {
    let file_name = get_block_cache_file_name(
        cache.witness.chain_config.chain_id,
        cache
            .blocks
            .first()
            .ok_or_eyre("tried writing cache for no blocks")?
            .header
            .number,
        if cache.blocks.len() == 1 {
            None
        } else {
            Some(
                cache
                    .blocks
                    .last()
                    .ok_or_eyre("tried writing cache for no blocks")?
                    .header
                    .number,
            )
        },
        l2,
    );

    if cache.blocks.is_empty() {
        return Err(eyre::Error::msg("cache can't be empty"));
    }

    let mut file = BufWriter::new(File::create(file_name)?);

    let bytes = rkyv::to_bytes::<Error>(cache).wrap_err("Failed to serialize with rkyv")?;

    file.write_all(&bytes)
        .wrap_err("Failed to write binary data")
}
