use ethrex_common::types::Block;
use ethrex_common::types::ChainConfig;
use ethrex_common::types::blobs_bundle;
use ethrex_common::types::block_execution_witness::ExecutionWitness;
use ethrex_config::networks::Network;
use ethrex_rpc::debug::execution_witness::RpcExecutionWitness;
use eyre::Context;
use rkyv::rancor::Error;
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::io::BufReader;
use std::io::Write;
use std::{fs::File, io::BufWriter};

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
    pub witness: RpcExecutionWitness,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    pub network: Option<Network>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    pub chain_config: Option<ChainConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    pub l2_fields: Option<L2Fields>,
}

impl Cache {
    pub fn new(blocks: Vec<Block>, witness: RpcExecutionWitness, network: Option<Network>) -> Self {
        Self {
            blocks,
            witness,
            network,
            chain_config: None,
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
