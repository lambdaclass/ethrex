use ethrex_common::{
    Address, Bytes, H256, U256,
    types::{
        AccountInfo, AccountUpdate, Block, BlockHeader, ChainConfig,
        block_execution_witness::{ExecutionWitnessError, ExecutionWitnessResult},
    },
};
use ethrex_vm::{EvmError, VmDatabase};
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeAs, SerializeAs, serde_as};
use std::sync::{Arc, Mutex, MutexGuard};

#[cfg(feature = "l2")]
use ethrex_common::types::blobs_bundle;

/// Private input variables passed into the zkVM execution program.
#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct ProgramInput {
    /// blocks to execute
    pub blocks: Vec<Block>,
    /// database containing all the data necessary to execute
    pub db: ExecutionWitnessResult,
    /// value used to calculate base fee
    pub elasticity_multiplier: u64,
    #[cfg(feature = "l2")]
    /// KZG commitment to the blob data
    #[serde_as(as = "[_; 48]")]
    pub blob_commitment: blobs_bundle::Commitment,
    #[cfg(feature = "l2")]
    /// KZG opening for a challenge over the blob commitment
    #[serde_as(as = "[_; 48]")]
    pub blob_proof: blobs_bundle::Proof,
}

/// JSON serializable program input. This struct is forced to serialize into JSON format.
///
// This is necessary because SP1 uses bincode for serialization into zkVM, which does not play well with
// serde attributes like #[serde(skip)], failing to deserialize with an unrelated error message (this is an old bug).
// As a patch we force serialization into JSON first (which is a format that works well with these attributes).
pub struct JSONProgramInput(pub ProgramInput);

impl Default for ProgramInput {
    fn default() -> Self {
        Self {
            blocks: Default::default(),
            db: Default::default(),
            elasticity_multiplier: Default::default(),
            #[cfg(feature = "l2")]
            blob_commitment: [0; 48],
            #[cfg(feature = "l2")]
            blob_proof: [0; 48],
        }
    }
}

impl Serialize for JSONProgramInput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut encoded = Vec::new();
        serde_json::to_writer(&mut encoded, &self.0).map_err(serde::ser::Error::custom)?;
        serde_with::Bytes::serialize_as(&encoded, serializer)
    }
}
impl<'de> Deserialize<'de> for JSONProgramInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let encoded: Vec<u8> = serde_with::Bytes::deserialize_as(deserializer)?;
        let decoded: ProgramInput =
            serde_json::from_reader(&encoded[..]).map_err(serde::de::Error::custom)?;
        Ok(JSONProgramInput(decoded))
    }
}

/// Public output variables exposed by the zkVM execution program. Some of these are part of
/// the program input.
#[derive(Serialize, Deserialize)]
pub struct ProgramOutput {
    /// initial state trie root hash
    pub initial_state_hash: H256,
    /// final state trie root hash
    pub final_state_hash: H256,
    #[cfg(feature = "l2")]
    /// merkle root of all messages in a batch
    pub l1messages_merkle_root: H256,
    #[cfg(feature = "l2")]
    /// hash of all the privileged transactions made in a batch
    pub privileged_transactions_hash: H256,
    #[cfg(feature = "l2")]
    /// blob commitment versioned hash
    pub blob_versioned_hash: H256,
    /// hash of the last block in a batch
    pub last_block_hash: H256,
    /// chain_id of the network
    pub chain_id: U256,
    /// amount of non-privileged transactions
    pub non_privileged_count: U256,
}

impl ProgramOutput {
    pub fn encode(&self) -> Vec<u8> {
        [
            self.initial_state_hash.to_fixed_bytes(),
            self.final_state_hash.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.l1messages_merkle_root.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.privileged_transactions_hash.to_fixed_bytes(),
            #[cfg(feature = "l2")]
            self.blob_versioned_hash.to_fixed_bytes(),
            self.last_block_hash.to_fixed_bytes(),
            self.chain_id.to_big_endian(),
            self.non_privileged_count.to_big_endian(),
        ]
        .concat()
    }
}

#[derive(Clone)]
pub struct ExecutionWitnessWrapper {
    inner: Arc<Mutex<ExecutionWitnessResult>>,
}

impl ExecutionWitnessWrapper {
    pub fn new(db: ExecutionWitnessResult) -> Self {
        Self {
            inner: Arc::new(Mutex::new(db)),
        }
    }

    pub fn lock_mutex(&self) -> Result<MutexGuard<ExecutionWitnessResult>, ExecutionWitnessError> {
        self.inner
            .lock()
            .map_err(|_| ExecutionWitnessError::Database("Failed to lock DB".to_string()))
    }

    pub fn apply_account_updates(
        &mut self,
        account_updates: &[AccountUpdate],
    ) -> Result<(), ExecutionWitnessError> {
        self.lock_mutex()?.apply_account_updates(account_updates)
    }

    pub fn state_trie_root(&self) -> Result<H256, ExecutionWitnessError> {
        self.lock_mutex()?.state_trie_root()
    }
    pub fn get_first_invalid_block_hash(&self) -> Result<Option<u64>, ExecutionWitnessError> {
        self.lock_mutex()?.get_first_invalid_block_hash()
    }

    pub fn get_block_parent_header(
        &self,
        block_number: u64,
    ) -> Result<BlockHeader, ExecutionWitnessError> {
        self.lock_mutex()?
            .get_block_parent_header(block_number)
            .cloned()
    }
}

impl VmDatabase for ExecutionWitnessWrapper {
    fn get_account_code(&self, code_hash: H256) -> Result<Bytes, EvmError> {
        self.lock_mutex()
            .map_err(|_| EvmError::DB("Failed to lock db".to_string()))?
            .get_account_code(code_hash)
            .map_err(|_| EvmError::DB("Failed to get account code".to_string()))
    }

    fn get_account_info(&self, address: Address) -> Result<Option<AccountInfo>, EvmError> {
        self.lock_mutex()
            .map_err(|_| EvmError::DB("Failed to lock db".to_string()))?
            .get_account_info(address)
            .map_err(|_| EvmError::DB("Failed to get account info".to_string()))
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        self.lock_mutex()
            .map_err(|_| EvmError::DB("Failed to lock db".to_string()))?
            .get_block_hash(block_number)
            .map_err(|_| EvmError::DB("Failed get block hash".to_string()))
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        self.lock_mutex()
            .map_err(|_| EvmError::DB("Failed to lock db".to_string()))?
            .get_chain_config()
            .map_err(|_| EvmError::DB("Failed get chain config".to_string()))
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        self.lock_mutex()
            .map_err(|_| EvmError::DB("Failed to lock db".to_string()))?
            .get_storage_slot(address, key)
            .map_err(|_| EvmError::DB("Failed get storage slot".to_string()))
    }
}
