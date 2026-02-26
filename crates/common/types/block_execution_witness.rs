use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;

use crate::rkyv_utils::H160Wrapper;
use crate::serde_utils;
use crate::types::{Block, Code, CodeMetadata};
use crate::{
    constants::EMPTY_KECCACK_HASH,
    types::{AccountState, AccountUpdate, BlockHeader, ChainConfig},
};
use ethereum_types::{Address, H256, U256};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::error::RLPDecodeError;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_trie::{EMPTY_TRIE_HASH, Node, Trie, TrieError};
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
    /// This is computed during guest program execution inside the zkVM,
    /// before the stateless validation.
    pub codes_hashed: BTreeMap<H256, Code>,
    /// Map of block numbers to their corresponding block headers.
    /// The block headers are pushed to the zkVM RLP-encoded, and then
    /// decoded and stored in this map during guest program execution,
    /// inside the zkVM.
    pub block_headers: BTreeMap<u64, BlockHeader>,
    /// The accounts state trie containing the necessary state for the guest
    /// program execution.
    pub state_trie: Trie,
    /// The parent block header of the first block in the batch.
    pub parent_block_header: BlockHeader,
    /// The block number of the first block in the batch.
    pub first_block_number: u64,
    /// The chain configuration.
    pub chain_config: ChainConfig,
    /// Map of storage root hashes to their corresponding storage tries.
    pub storage_tries: BTreeMap<Address, Trie>,
    /// Map of account addresses to their corresponding hashed addresses.
    /// This is a convenience map to avoid recomputing the hashed address
    /// multiple times during guest program execution.
    /// It is built on-demand during guest program execution, inside the zkVM.
    pub account_hashes_by_address: BTreeMap<Address, Vec<u8>>,
    /// Map of account addresses to booleans, indicating whose account's storage tries were
    /// verified.
    /// Verification is done by hashing the trie and comparing the root hash with the account's storage root.
    pub verified_storage_roots: BTreeMap<Address, bool>,
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
    /// Root node embedded with the rest of the trie's nodes
    pub state_trie_root: Option<Node>,
    /// Root nodes per account storage embedded with the rest of the trie's nodes
    #[rkyv(with = MapKV<H160Wrapper, Identity>)]
    pub storage_trie_roots: BTreeMap<Address, Node>,
    /// Flattened map of account addresses and storage keys whose values
    /// are needed for stateless execution.
    #[rkyv(with = crate::rkyv_utils::VecVecWrapper)]
    pub keys: Vec<Vec<u8>>,
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

impl TryFrom<ExecutionWitness> for RpcExecutionWitness {
    type Error = TrieError;

    fn try_from(value: ExecutionWitness) -> Result<Self, Self::Error> {
        let mut nodes = Vec::new();
        if let Some(state_trie_root) = value.state_trie_root {
            state_trie_root.encode_subtrie(&mut nodes)?;
        }
        for node in value.storage_trie_roots.values() {
            node.encode_subtrie(&mut nodes)?;
        }
        Ok(Self {
            state: nodes.into_iter().map(Bytes::from).collect(),
            keys: value.keys.into_iter().map(Bytes::from).collect(),
            codes: value.codes.into_iter().map(Bytes::from).collect(),
            headers: value
                .block_headers_bytes
                .into_iter()
                .map(Bytes::from)
                .collect(),
        })
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
    Trie(#[from] TrieError),
    #[error("RLP Decode: {0}")]
    RLPDecode(#[from] RLPDecodeError),
    #[error("Unreachable code reached: {0}")]
    Unreachable(String),
    #[error("Custom error: {0}")]
    Custom(String),
}

impl TryFrom<ExecutionWitness> for GuestProgramState {
    type Error = GuestProgramStateError;

    fn try_from(value: ExecutionWitness) -> Result<Self, Self::Error> {
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

        // hash state trie nodes
        let state_trie = if let Some(state_trie_root) = value.state_trie_root {
            Trie::new_temp_with_root(state_trie_root.into())
        } else {
            Trie::new_temp()
        };
        state_trie.hash_no_commit();

        let mut storage_tries = BTreeMap::new();
        for (address, storage_trie_root) in value.storage_trie_roots {
            // hash storage trie nodes
            let storage_trie = Trie::new_temp_with_root(storage_trie_root.into());
            storage_trie.hash_no_commit();
            storage_tries.insert(address, storage_trie);
        }

        // hash codes
        // TODO: codes here probably needs to be Vec<Code>, rather than recomputing here. This requires rkyv implementation.
        let codes_hashed = value
            .codes
            .into_iter()
            .map(|code| {
                let code = Code::from_bytecode(code.into());
                (code.hash, code)
            })
            .collect();

        let ethrex_guest_program_state = GuestProgramState {
            codes_hashed,
            state_trie,
            storage_tries,
            block_headers,
            parent_block_header: parent_header,
            first_block_number: value.first_block_number,
            chain_config: value.chain_config,
            account_hashes_by_address: BTreeMap::new(),
            verified_storage_roots: BTreeMap::new(),
        };

        Ok(ethrex_guest_program_state)
    }
}

impl GuestProgramState {
    /// Helper function to apply account updates to the execution witness
    /// It updates the state trie and storage tries with the given account updates
    /// Returns an error if the updates cannot be applied
    pub fn apply_account_updates(
        &mut self,
        account_updates: &[AccountUpdate],
    ) -> Result<(), GuestProgramStateError> {
        for update in account_updates.iter() {
            let hashed_address = self
                .account_hashes_by_address
                .entry(update.address)
                .or_insert_with(|| hash_address(&update.address));

            if update.removed {
                // Remove account from trie
                self.state_trie.remove(hashed_address)?;
            } else {
                // Add or update AccountState in the trie
                // Fetch current state or create a new state to be inserted
                let mut account_state = match self.state_trie.get(hashed_address)? {
                    Some(encoded_state) => AccountState::decode(&encoded_state)?,
                    None => AccountState::default(),
                };
                if update.removed_storage {
                    account_state.storage_root = *EMPTY_TRIE_HASH;
                }
                if let Some(info) = &update.info {
                    account_state.nonce = info.nonce;
                    account_state.balance = info.balance;
                    account_state.code_hash = info.code_hash;
                    // Store updated code in DB
                    if let Some(code) = &update.code {
                        self.codes_hashed.insert(info.code_hash, code.clone());
                    }
                }
                // Store the added storage in the account's storage trie and compute its new root
                if !update.added_storage.is_empty() {
                    let storage_trie = self.storage_tries.entry(update.address).or_default();

                    // Inserts must come before deletes, otherwise deletes might require extra nodes
                    // Example:
                    // If I have a branch node [A, B] and want to delete A and insert C
                    // I will need to have B only if the deletion happens first
                    let (deletes, inserts): (Vec<_>, Vec<_>) = update
                        .added_storage
                        .iter()
                        .map(|(k, v)| (hash_key(k), v))
                        .partition(|(_k, v)| v.is_zero());

                    for (hashed_key, storage_value) in inserts {
                        storage_trie.insert(hashed_key, storage_value.encode_to_vec())?;
                    }

                    for (hashed_key, _) in deletes {
                        storage_trie.remove(&hashed_key)?;
                    }

                    let storage_root = storage_trie.hash_no_commit();
                    account_state.storage_root = storage_root;
                }

                self.state_trie
                    .insert(hashed_address.clone(), account_state.encode_to_vec())?;
            }
        }
        Ok(())
    }

    /// Returns the root hash of the state trie
    /// Returns an error if the state trie is not built yet
    pub fn state_trie_root(&self) -> Result<H256, GuestProgramStateError> {
        Ok(self.state_trie.hash_no_commit())
    }

    /// Returns Some(block_number) if the hash for block_number is not the parent
    /// hash of block_number + 1. None if there's no such hash.
    ///
    /// Keep in mind that the last block hash (which is a batch's parent hash)
    /// can't be validated against the next header, because it has no successor.
    pub fn get_first_invalid_block_hash(&self) -> Result<Option<u64>, GuestProgramStateError> {
        // Enforces there's at least one block header, so windows() call doesn't panic.
        if self.block_headers.is_empty() {
            return Err(GuestProgramStateError::NoBlockHeaders);
        };

        // Sort in ascending order
        let mut block_headers: Vec<_> = self.block_headers.iter().collect();
        block_headers.sort_by_key(|(number, _)| *number);

        // Validate hashes
        for window in block_headers.windows(2) {
            let (Some((number, header)), Some((next_number, next_header))) =
                (window.first().cloned(), window.get(1).cloned())
            else {
                // windows() returns an empty iterator in this case.
                return Err(GuestProgramStateError::Unreachable(
                    "block header window len is < 2".to_string(),
                ));
            };
            if *next_number != *number + 1 {
                return Err(GuestProgramStateError::NoncontiguousBlockHeaders);
            }

            if next_header.parent_hash != header.hash() {
                return Ok(Some(*number));
            }
        }

        Ok(None)
    }

    /// Retrieves the parent block header for the specified block number
    /// Searches within `self.block_headers`
    pub fn get_block_parent_header(
        &self,
        block_number: u64,
    ) -> Result<&BlockHeader, GuestProgramStateError> {
        self.block_headers
            .get(&block_number.saturating_sub(1))
            .ok_or(GuestProgramStateError::MissingParentHeaderOf(block_number))
    }

    /// Retrieves the account state from the state trie.
    /// Returns an error if decoding the account state fails.
    pub fn get_account_state(
        &mut self,
        address: Address,
    ) -> Result<Option<AccountState>, GuestProgramStateError> {
        let hashed_address = self
            .account_hashes_by_address
            .entry(address)
            .or_insert_with(|| hash_address(&address));

        let Ok(Some(encoded_state)) = self.state_trie.get(hashed_address) else {
            return Ok(None);
        };
        let state = AccountState::decode(&encoded_state).map_err(|_| {
            GuestProgramStateError::Database("Failed to get decode account from trie".to_string())
        })?;

        Ok(Some(state))
    }

    /// Fetches the block hash for a specific block number.
    /// Looks up `self.block_headers` and computes the hash if it is not already computed.
    pub fn get_block_hash(&self, block_number: u64) -> Result<H256, GuestProgramStateError> {
        self.block_headers
            .get(&block_number)
            .map(|header| header.hash())
            .ok_or_else(|| {
                GuestProgramStateError::Database(format!(
                    "Block hash not found for block number {block_number}"
                ))
            })
    }

    /// Retrieves a storage slot value for an account in its storage trie.
    pub fn get_storage_slot(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<Option<U256>, GuestProgramStateError> {
        let hashed_key = hash_key(&key);
        let Some(storage_trie) = self.get_valid_storage_trie(address)? else {
            return Ok(None);
        };
        if let Some(encoded_key) = storage_trie
            .get(&hashed_key)
            .map_err(|e| GuestProgramStateError::Database(e.to_string()))?
        {
            U256::decode(&encoded_key)
                .map_err(|_| {
                    GuestProgramStateError::Database("failed to read storage from trie".to_string())
                })
                .map(Some)
        } else {
            Ok(None)
        }
    }

    /// Retrieves the chain configuration for the execution witness.
    pub fn get_chain_config(&self) -> Result<ChainConfig, GuestProgramStateError> {
        Ok(self.chain_config)
    }

    /// Retrieves the account code for a specific account.
    /// Returns an Err if the code is not found.
    pub fn get_account_code(&self, code_hash: H256) -> Result<Code, GuestProgramStateError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Code::default());
        }
        match self.codes_hashed.get(&code_hash) {
            Some(code) => Ok(code.clone()),
            None => {
                // We do this because what usually happens is that the Witness doesn't have the code we asked for but it is because it isn't relevant for that particular case.
                // In client implementations there are differences and it's natural for some clients to access more/less information in some edge cases.
                // Sidenote: logger doesn't work inside SP1, that's why we use println!
                println!(
                    "Missing bytecode for hash {} in witness. Defaulting to empty code.", // If there's a state root mismatch and this prints we have to see if it's the cause or not.
                    hex::encode(code_hash)
                );
                Ok(Code::default())
            }
        }
    }

    /// Retrieves code metadata (length) for a specific code hash.
    /// This is an optimized path for EXTCODESIZE opcode.
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
                // Same as get_account_code - default to empty for missing bytecode
                println!(
                    "Missing bytecode for hash {} in witness. Defaulting to empty code metadata.",
                    hex::encode(code_hash)
                );
                Ok(CodeMetadata { length: 0 })
            }
        }
    }

    /// When executing multiple blocks in the L2 it happens that the headers in block_headers correspond to the same block headers that we have in the blocks array. The main goal is to hash these only once and set them in both places.
    /// We also initialize the remaining block headers hashes. If they are set, we check their validity.
    pub fn initialize_block_header_hashes(
        &self,
        blocks: &[Block],
    ) -> Result<(), GuestProgramStateError> {
        let mut block_numbers_in_common = BTreeSet::new();
        for block in blocks {
            let hash = block.header.compute_block_hash();
            set_hash_or_validate(&block.header, hash)?;

            let number = block.header.number;
            if let Some(header) = self.block_headers.get(&number) {
                block_numbers_in_common.insert(number);
                set_hash_or_validate(header, hash)?;
            }
        }

        for header in self.block_headers.values() {
            if block_numbers_in_common.contains(&header.number) {
                // We have already set this hash in the previous step
                continue;
            }
            let hash = header.compute_block_hash();
            set_hash_or_validate(header, hash)?;
        }

        Ok(())
    }

    pub fn get_valid_storage_trie(
        &mut self,
        address: Address,
    ) -> Result<Option<&Trie>, GuestProgramStateError> {
        let is_storage_verified = *self.verified_storage_roots.get(&address).unwrap_or(&false);
        if is_storage_verified {
            Ok(self.storage_tries.get(&address))
        } else {
            let Some(storage_root) = self.get_account_state(address)?.map(|a| a.storage_root)
            else {
                // empty account
                return Ok(None);
            };
            let storage_trie = match self.storage_tries.get(&address) {
                None if storage_root == *EMPTY_TRIE_HASH => return Ok(None),
                Some(trie) if trie.hash_no_commit() == storage_root => trie,
                _ => {
                    return Err(GuestProgramStateError::Custom(format!(
                        "invalid storage trie for account {address}"
                    )));
                }
            };
            self.verified_storage_roots.insert(address, true);
            Ok(Some(storage_trie))
        }
    }
}

fn hash_address(address: &Address) -> Vec<u8> {
    keccak_hash(address.to_fixed_bytes()).to_vec()
}

pub fn hash_key(key: &H256) -> Vec<u8> {
    keccak_hash(key.to_fixed_bytes()).to_vec()
}

/// Initializes hash of header or validates the hash is correct in case it's already set
/// Note that header doesn't need to be mutable because the hash is a OnceCell
fn set_hash_or_validate(header: &BlockHeader, hash: H256) -> Result<(), GuestProgramStateError> {
    // If it's already set the .set() method will return the current value
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
