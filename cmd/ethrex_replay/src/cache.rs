use ethrex_common::types::Block;
use ethrex_common::types::ChainConfig;
use ethrex_common::types::blobs_bundle;
use ethrex_config::networks::Network;
use ethrex_rpc::debug::execution_witness::RpcExecutionWitness;
use eyre::OptionExt;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use std::io::BufReader;
use std::{fs::File, io::BufWriter};

#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct L2Fields {
    #[serde_as(as = "[_; 48]")]
    pub blob_commitment: blobs_bundle::Commitment,
    #[serde_as(as = "[_; 48]")]
    pub blob_proof: blobs_bundle::Proof,
}
/// Structure holding input data needed to execute or prove blocks.
/// Optional fields are included only when relevant (e.g. L2 or custom chain).
#[derive(Serialize, Deserialize)]
pub struct Cache {
    /// Blocks to execute / prove.
    pub blocks: Vec<Block>,
    /// State data required to run those blocks.
    pub witness: RpcExecutionWitness,
    /// L1 network identifier.
    /// For L1 chains, this is used to retrieve the chain configuration from the repository.
    /// For L2 chains, the chain configuration is passed directly via `chain_config` instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    pub network: Option<Network>,
    /// Chain configuration.
    /// For L2 chains, this is used directly as we might not have the chain in our repository.
    /// For custom chains, this allows using a configuration different from the repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    pub chain_config: Option<ChainConfig>,
    /// L2 specific fields (blob data).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    pub l2_fields: Option<L2Fields>,
}

impl Cache {
    pub fn get_first_block_number(&self) -> eyre::Result<u64> {
        self.blocks
            .iter()
            .map(|block| block.header.number)
            .min()
            .ok_or_eyre("Cache should contain at least one block number.")
    }

    pub fn get_chain_config(&self) -> eyre::Result<ChainConfig> {
        if let Some(config) = self.chain_config {
            return Ok(config);
        }
        if let Some(network) = &self.network {
            return network
                .get_genesis()
                .map(|genesis| genesis.config)
                .map_err(|e| eyre::eyre!("Failed to get genesis config: {}", e));
        }
        Err(eyre::eyre!(
            "Cache doesn't have network nor config, this shouldn't happen"
        ))
    }

    pub fn new(blocks: Vec<Block>, witness: RpcExecutionWitness, network: Option<Network>) -> Self {
        Self {
            blocks,
            witness,
            network,
            chain_config: None,
            l2_fields: None,
        }
    }
    pub fn load_cache(file_name: &str) -> eyre::Result<Self> {
        let file = BufReader::new(File::open(file_name)?);
        Ok(serde_json::from_reader(file)?)
    }

    pub fn write_cache(&self, file_name: &str) -> eyre::Result<()> {
        if self.blocks.is_empty() {
            return Err(eyre::Error::msg("cache can't be empty"));
        }
        let file = BufWriter::new(File::create(file_name)?);
        Ok(serde_json::to_writer_pretty(file, self)?)
    }
}
