use ethrex_common::types::Block;
use ethrex_common::types::blobs_bundle;
use ethrex_common::types::block_execution_witness::{
    ExecutionWitnessError, ExecutionWitnessResult,
};
use ethrex_config::networks::Network;
use ethrex_rpc::debug::execution_witness::{
    RpcExecutionWitness, execution_witness_from_rpc_chain_config,
};
use eyre::Context;
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::io::Write;
use std::{fs::File, io::BufWriter};

#[serde_as]
#[derive(Serialize, Deserialize, RSerialize, RDeserialize, Archive)]
pub struct L2Fields {
    #[serde_as(as = "[_; 48]")]
    pub blob_commitment: blobs_bundle::Commitment,
    #[serde_as(as = "[_; 48]")]
    pub blob_proof: blobs_bundle::Proof,
}

/// Used for storing information in files
/// We used the Witness RPC instead of the Witness because the latter changes frequently and the former is more stable.
/// We use this only for L1 blocks
#[derive(Serialize, Deserialize)]
pub struct Cache {
    pub block: Block,
    pub witness_rpc: RpcExecutionWitness,
    pub network: Network,
}

#[derive(Archive, RSerialize, RDeserialize)]
pub struct ReplayInput {
    pub blocks: Vec<Block>,
    pub witness: ExecutionWitnessResult,
    pub l2_fields: Option<L2Fields>,
}

impl Cache {
    pub fn new(block: Block, witness_rpc: RpcExecutionWitness, network: Network) -> Self {
        Self {
            block,
            witness_rpc,
            network,
        }
    }

    pub fn into_witness(&self) -> Result<ExecutionWitnessResult, ExecutionWitnessError> {
        execution_witness_from_rpc_chain_config(
            self.witness_rpc.clone(),
            self.network.get_genesis().unwrap().config, //TODO: Remove unwrap?
            self.block.header.number,
        )
    }

    pub fn into_replay_input(&self) -> Result<ReplayInput, ExecutionWitnessError> {
        let witness = self.into_witness()?;
        Ok(ReplayInput {
            blocks: vec![self.block.clone()],
            witness,
            l2_fields: None,
        })
    }

    pub fn load(file_name: &str) -> eyre::Result<Self> {
        let file_data = std::fs::read(file_name)?;
        let cache = serde_json::from_slice::<Self>(&file_data)
            .wrap_err("Failed to deserialize with serde_json")?;
        Ok(cache)
    }

    pub fn write(&self, file_name: &str) -> eyre::Result<()> {
        let mut file = BufWriter::new(File::create(file_name)?);
        let pretty_json = serde_json::to_string_pretty(self)
            .wrap_err("Failed to serialize with serde_json (pretty)")?;
        file.write_all(pretty_json.as_bytes())
            .wrap_err("Failed to write pretty JSON data")
    }
}

pub fn write_cache(cache: &Cache, file_name: &str) -> eyre::Result<()> {
    let mut file = BufWriter::new(File::create(file_name)?);
    let pretty_json = serde_json::to_string_pretty(cache)
        .wrap_err("Failed to serialize with serde_json (pretty)")?;
    file.write_all(pretty_json.as_bytes())
        .wrap_err("Failed to write pretty JSON data")
}
