use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;

use crate::rkyv_utils::H256Wrapper;
use crate::serde_utils;
use crate::types::{Block, Code, CodeMetadata};
use crate::{
    constants::EMPTY_KECCACK_HASH,
    types::{AccountState, AccountUpdate, BlockHeader, ChainConfig},
};
use ethereum_types::{Address, H256, U256};
use ethrex_crypto::Crypto;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::error::RLPDecodeError;
use rkyv::with::{Identity, MapKV};
use serde::{Deserialize, Serialize};

/// State produced by the guest program execution inside the zkVM. It is
/// essentially built from the `ExecutionWitness`.
/// This state is used during the stateless validation of the zkVM execution.
/// Some data is prepared before the stateless validation, and some data is
/// built on-demand during the stateless validation.
/// This struct must be instantiated, filled, and consumed inside the zkVM.
pub struct GuestProgramState {
    /// Map of code hashes to their corresponding bytecode.
    pub codes_hashed: BTreeMap<H256, Code>,
    /// Map of block numbers to their corresponding block headers.
    pub block_headers: BTreeMap<u64, BlockHeader>,
    /// The parent block header of the first block in the batch.
    pub parent_block_header: BlockHeader,
    /// The block number of the first block in the batch.
    pub first_block_number: u64,
    /// The chain configuration.
    pub chain_config: ChainConfig,
    /// Map of account addresses to their corresponding hashed addresses.
    pub account_hashes_by_address: BTreeMap<Address, H256>,
}

/// Witness data produced by the client and consumed by the guest program
/// inside the zkVM.
///
/// It is essentially an `RpcExecutionWitness` but it also contains `ChainConfig`,
/// and `first_block_number`.
#[derive(
    Default, Serialize, Deserialize, rkyv::Serialize, rkyv::Deserialize, rkyv::Archive, Clone,
)]
pub struct ExecutionWitness {
    // Contract bytecodes needed for stateless execution.
    #[rkyv(with = crate::rkyv_utils::VecVecWrapper)]
    pub codes: Vec<Vec<u8>>,
    /// RLP-encoded block headers needed for stateless execution.
    #[rkyv(with = crate::rkyv_utils::VecVecWrapper)]
    pub block_headers_bytes: Vec<Vec<u8>>,
    /// The block number of the first block
    pub first_block_number: u64,
    // The chain config.
    pub chain_config: ChainConfig,
    /// Serialized state trie nodes (RLP-encoded).
    /// Replaces the former `state_trie_root: Option<Node>` field.
    #[rkyv(with = crate::rkyv_utils::VecVecWrapper)]
    pub state_trie_nodes: Vec<Vec<u8>>,
    /// Serialized storage trie nodes per account (keyed by keccak256 of account address).
    #[rkyv(with = MapKV<H256Wrapper, Identity>)]
    pub storage_trie_nodes: BTreeMap<H256, Vec<Vec<u8>>>,
}

/// RPC-friendly representation of an execution witness.
///
/// This is the format returned by the `debug_executionWitness` RPC method.
/// The trie nodes are pre-serialized (via `encode_subtrie`) to avoid
/// expensive traversal on every RPC request.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RpcExecutionWitness {
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub state: Vec<Bytes>,
    #[serde(
        default,
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub keys: Vec<Bytes>,
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub codes: Vec<Bytes>,
    #[serde(
        serialize_with = "serde_utils::bytes::vec::serialize",
        deserialize_with = "serde_utils::bytes::vec::deserialize"
    )]
    pub headers: Vec<Bytes>,
}

impl From<ExecutionWitness> for RpcExecutionWitness {
    fn from(value: ExecutionWitness) -> Self {
        let state: Vec<Bytes> = value
            .state_trie_nodes
            .into_iter()
            .chain(
                value
                    .storage_trie_nodes
                    .into_values()
                    .flat_map(|nodes| nodes.into_iter()),
            )
            .map(Bytes::from)
            .collect();
        Self {
            state,
            keys: Vec::new(),
            codes: value.codes.into_iter().map(Bytes::from).collect(),
            headers: value
                .block_headers_bytes
                .into_iter()
                .map(Bytes::from)
                .collect(),
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum GuestProgramStateError {
    #[error("Failed to rebuild tries: {0}")]
    RebuildTrie(String),
    #[error("Failed to apply account updates {0}")]
    ApplyAccountUpdates(String),
    #[error("DB error: {0}")]
    Database(String),
    #[error("No block headers stored, should at least store parent header")]
    NoBlockHeaders,
    #[error("Parent block header of block {0} was not found")]
    MissingParentHeaderOf(u64),
    #[error("Non-contiguous block headers (there's a gap in the block headers list)")]
    NoncontiguousBlockHeaders,
    #[error("Trie error: {0}")]
    Trie(String),
    #[error("RLP Decode: {0}")]
    RLPDecode(#[from] RLPDecodeError),
    #[error("Unreachable code reached: {0}")]
    Unreachable(String),
    #[error("Custom error: {0}")]
    Custom(String),
}

impl GuestProgramState {
    pub fn from_witness(
        value: ExecutionWitness,
        _crypto: &dyn Crypto,
    ) -> Result<Self, GuestProgramStateError> {
        let block_headers: BTreeMap<u64, BlockHeader> = value
            .block_headers_bytes
            .into_iter()
            .map(|bytes| BlockHeader::decode(bytes.as_ref()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                GuestProgramStateError::Custom(format!("Failed to decode block headers: {}", e))
            })?
            .into_iter()
            .map(|header| (header.number, header))
            .collect();

        let parent_number =
            value
                .first_block_number
                .checked_sub(1)
                .ok_or(GuestProgramStateError::Custom(
                    "First block number cannot be zero".to_string(),
                ))?;

        let parent_header = block_headers.get(&parent_number).cloned().ok_or(
            GuestProgramStateError::MissingParentHeaderOf(value.first_block_number),
        )?;

        // TODO: codes needs Crypto to hash, but this path is only used in the zkVM guest.
        // Stubbed until trie integration is restored.
        let codes_hashed = BTreeMap::new();
        let _ = value.codes;

        Ok(GuestProgramState {
            codes_hashed,
            block_headers,
            parent_block_header: parent_header,
            first_block_number: value.first_block_number,
            chain_config: value.chain_config,
            account_hashes_by_address: BTreeMap::new(),
        })
    }
}

impl GuestProgramState {
    pub fn apply_account_updates(
        &mut self,
        _account_updates: &[AccountUpdate],
        _crypto: &dyn Crypto,
    ) -> Result<(), GuestProgramStateError> {
        todo!("GuestProgramState::apply_account_updates: requires MPT/binary-trie integration")
    }

    pub fn state_trie_root(&self, _crypto: &dyn Crypto) -> Result<H256, GuestProgramStateError> {
        todo!("GuestProgramState::state_trie_root: requires trie integration")
    }

    pub fn get_first_invalid_block_hash(
        &self,
        crypto: &dyn Crypto,
    ) -> Result<Option<u64>, GuestProgramStateError> {
        if self.block_headers.is_empty() {
            return Err(GuestProgramStateError::NoBlockHeaders);
        };

        let mut block_headers: Vec<_> = self.block_headers.iter().collect();
        block_headers.sort_by_key(|(number, _)| *number);

        for window in block_headers.windows(2) {
            let (Some((number, header)), Some((next_number, next_header))) =
                (window.first().cloned(), window.get(1).cloned())
            else {
                return Err(GuestProgramStateError::Unreachable(
                    "block header window len is < 2".to_string(),
                ));
            };
            if *next_number != *number + 1 {
                return Err(GuestProgramStateError::NoncontiguousBlockHeaders);
            }

            if next_header.parent_hash != header.compute_block_hash(crypto) {
                return Ok(Some(*number));
            }
        }

        Ok(None)
    }

    pub fn get_block_parent_header(
        &self,
        block_number: u64,
    ) -> Result<&BlockHeader, GuestProgramStateError> {
        self.block_headers
            .get(&block_number.saturating_sub(1))
            .ok_or(GuestProgramStateError::MissingParentHeaderOf(block_number))
    }

    pub fn get_account_state(
        &mut self,
        _address: Address,
        _crypto: &dyn Crypto,
    ) -> Result<Option<AccountState>, GuestProgramStateError> {
        todo!("GuestProgramState::get_account_state: requires trie integration")
    }

    pub fn get_block_hash(
        &self,
        block_number: u64,
        crypto: &dyn Crypto,
    ) -> Result<H256, GuestProgramStateError> {
        self.block_headers
            .get(&block_number)
            .map(|header| header.compute_block_hash(crypto))
            .ok_or_else(|| {
                GuestProgramStateError::Database(format!(
                    "Block hash not found for block number {block_number}"
                ))
            })
    }

    pub fn get_storage_slot(
        &mut self,
        _address: Address,
        _key: H256,
        _crypto: &dyn Crypto,
    ) -> Result<Option<U256>, GuestProgramStateError> {
        todo!("GuestProgramState::get_storage_slot: requires trie integration")
    }

    pub fn get_chain_config(&self) -> Result<ChainConfig, GuestProgramStateError> {
        Ok(self.chain_config)
    }

    pub fn get_account_code(&self, code_hash: H256) -> Result<Code, GuestProgramStateError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Code::default());
        }
        match self.codes_hashed.get(&code_hash) {
            Some(code) => Ok(code.clone()),
            None => {
                println!(
                    "Missing bytecode for hash {} in witness. Defaulting to empty code.",
                    hex::encode(code_hash)
                );
                Ok(Code::default())
            }
        }
    }

    pub fn get_code_metadata(
        &self,
        code_hash: H256,
    ) -> Result<CodeMetadata, GuestProgramStateError> {
        use crate::constants::EMPTY_KECCACK_HASH;

        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(CodeMetadata { length: 0 });
        }
        match self.codes_hashed.get(&code_hash) {
            Some(code) => Ok(CodeMetadata {
                length: code.bytecode.len() as u64,
            }),
            None => {
                println!(
                    "Missing bytecode for hash {} in witness. Defaulting to empty code metadata.",
                    hex::encode(code_hash)
                );
                Ok(CodeMetadata { length: 0 })
            }
        }
    }

    pub fn initialize_block_header_hashes(
        &self,
        blocks: &[Block],
        crypto: &dyn Crypto,
    ) -> Result<(), GuestProgramStateError> {
        let mut block_numbers_in_common = BTreeSet::new();
        for block in blocks {
            let hash = block.header.compute_block_hash(crypto);
            set_hash_or_validate(&block.header, hash)?;

            let number = block.header.number;
            if let Some(header) = self.block_headers.get(&number) {
                block_numbers_in_common.insert(number);
                set_hash_or_validate(header, hash)?;
            }
        }

        for header in self.block_headers.values() {
            if block_numbers_in_common.contains(&header.number) {
                continue;
            }
            let hash = header.compute_block_hash(crypto);
            set_hash_or_validate(header, hash)?;
        }

        Ok(())
    }

    pub fn get_valid_storage_trie(
        &mut self,
        _address: Address,
        _crypto: &dyn Crypto,
    ) -> Result<Option<()>, GuestProgramStateError> {
        todo!("GuestProgramState::get_valid_storage_trie: requires trie integration")
    }
}

pub fn hash_key(key: &H256, crypto: &dyn Crypto) -> Vec<u8> {
    crypto.keccak256(&key.to_fixed_bytes()).to_vec()
}

/// Initializes hash of header or validates the hash is correct in case it's already set
fn set_hash_or_validate(header: &BlockHeader, hash: H256) -> Result<(), GuestProgramStateError> {
    if let Err(prev_hash) = header.hash.set(hash)
        && prev_hash != hash
    {
        return Err(GuestProgramStateError::Custom(format!(
            "Block header hash was previously set for {} with the wrong value. It should be set correctly or left unset.",
            header.number
        )));
    }
    Ok(())
}
