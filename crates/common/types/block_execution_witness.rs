use std::collections::HashMap;

use crate::serde_utils;
use crate::{
    H160,
    constants::EMPTY_KECCACK_HASH,
    types::{AccountInfo, AccountState, AccountUpdate, BlockHeader, ChainConfig},
};
use bytes::Bytes;
use ethereum_types::{Address, U256};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_trie::{EMPTY_TRIE_HASH, Node, Trie};
use keccak_hash::H256;
use serde::{Deserialize, Deserializer, Serialize, de};
use sha3::{Digest, Keccak256};

/// In-memory execution witness database for single batch execution data.
///
/// This is mainly used to store the relevant state data for executing a single batch and then
/// feeding the DB into a zkVM program to prove the execution.
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionWitnessResult {
    /* reth compatible fields */
    #[serde(serialize_with = "serde_utils::bytes::vec::serialize")]
    pub state: Vec<Bytes>,
    #[serde(serialize_with = "serde_utils::bytes::vec::serialize")]
    pub keys: Vec<Bytes>,
    #[serde(serialize_with = "serde_utils::bytes::vec::serialize")]
    pub codes: Vec<Bytes>,
    #[serde(serialize_with = "serde_utils::bytes::vec::serialize")]
    pub headers: Vec<Bytes>,

    /* Our fields */
    // Indexed by code hash
    // Used evm bytecodes
    #[serde(skip)]
    pub codes_map: HashMap<H256, Bytes>,
    // Pruned state MPT
    #[serde(skip)]
    pub state_trie: Option<Trie>,
    // Indexed by account
    // Pruned storage MPT
    #[serde(skip)]
    pub storage_tries: Option<HashMap<Address, Trie>>,
    // Block headers needed for BLOCKHASH opcode
    #[serde(skip)]
    pub block_headers: HashMap<u64, BlockHeader>,
    // Chain config
    #[serde(skip)]
    pub chain_config: ChainConfig, // TODO: Remove this from this struct. To do this, we need ProgramInput to have the chain ID as a field.
}

#[derive(thiserror::Error, Debug)]
pub enum ExecutionWitnessError {
    #[error("Failed to rebuild tries: {0}")]
    RebuildTrie(String),
    #[error("Failed to apply account updates {0}")]
    ApplyAccountUpdates(String),
    #[error("DB error: {0}")]
    Database(String),
    #[error("No block headers stored, should at least store parent header")]
    NoBlockHeaders,
    #[error("Non-contiguous block headers (there's a gap in the block headers list)")]
    NoncontiguousBlockHeaders,
    #[error("Unreachable code reached: {0}")]
    Unreachable(String),
}

impl ExecutionWitnessResult {
    pub fn rebuild_tries(
        &mut self,
        first_header: &BlockHeader,
    ) -> Result<(), ExecutionWitnessError> {
        let parent_header = self.get_block_parent_header(first_header.number)?;

        let state_trie = Self::rebuild_trie(parent_header.state_root, &self.state)?;

        // Keys can either be account addresses or storage slots. They have different sizes,
        // so we filter them by size. The from_slice method panics if the input has the wrong size.
        let addresses: Vec<Address> = self
            .keys
            .iter()
            .filter(|k| k.len() == Address::len_bytes())
            .map(|k| Address::from_slice(k))
            .collect();

        let storage_tries: HashMap<Address, Trie> = HashMap::from_iter(
            addresses
                .iter()
                .filter_map(|addr| {
                    Some((
                        *addr,
                        Self::rebuild_storage_trie(addr, &state_trie, &self.state)?,
                    ))
                })
                .collect::<Vec<(Address, Trie)>>(),
        );

        self.state_trie = Some(state_trie);
        self.storage_tries = Some(storage_tries);

        Ok(())
    }

    pub fn rebuild_trie(
        initial_state: H256,
        state: &[Bytes],
    ) -> Result<Trie, ExecutionWitnessError> {
        let mut initial_node = None;

        for node in state.iter() {
            // If the node is empty we skip it
            if node == &vec![128_u8] {
                continue;
            }
            let x = Node::decode_raw(node).map_err(|_| {
                ExecutionWitnessError::RebuildTrie("Invalid state trie node in witness".to_string())
            })?;
            let hash = x.compute_hash().finalize();
            if hash == initial_state {
                initial_node = Some(node.clone());
                break;
            }
        }

        Trie::from_nodes(
            initial_node.map(|b| b.to_vec()).as_ref(),
            &state.iter().map(|b| b.to_vec()).collect::<Vec<_>>(),
        )
        .map_err(|e| ExecutionWitnessError::RebuildTrie(format!("Failed to build state trie {e}")))
    }

    // This funciton is an option because we expect it to fail sometimes, and we just want to filter it
    pub fn rebuild_storage_trie(address: &H160, trie: &Trie, state: &[Bytes]) -> Option<Trie> {
        let account_state_rlp = trie.get(&hash_address(address)).ok()??;

        let account_state = AccountState::decode(&account_state_rlp).ok()?;

        if account_state.storage_root == *EMPTY_TRIE_HASH {
            return None;
        }

        Self::rebuild_trie(account_state.storage_root, state).ok()
    }

    pub fn apply_account_updates(
        &mut self,
        account_updates: &[AccountUpdate],
    ) -> Result<(), ExecutionWitnessError> {
        let (Some(state_trie), Some(storage_tries_map)) =
            (self.state_trie.as_mut(), self.storage_tries.as_mut())
        else {
            return Err(ExecutionWitnessError::ApplyAccountUpdates(
                "Tried to apply account updates before rebuilding the tries".to_string(),
            ));
        };

        for update in account_updates.iter() {
            let hashed_address = hash_address(&update.address);
            if update.removed {
                // Remove account from trie
                state_trie
                    .remove(hashed_address)
                    .expect("failed to remove from trie");
            } else {
                // Add or update AccountState in the trie
                // Fetch current state or create a new state to be inserted
                let mut account_state = match state_trie
                    .get(&hashed_address)
                    .expect("failed to get account state from trie")
                {
                    Some(encoded_state) => AccountState::decode(&encoded_state)
                        .expect("failed to decode account state"),
                    None => AccountState::default(),
                };
                if let Some(info) = &update.info {
                    account_state.nonce = info.nonce;
                    account_state.balance = info.balance;
                    account_state.code_hash = info.code_hash;
                    // Store updated code in DB
                    if let Some(code) = &update.code {
                        self.codes_map.insert(info.code_hash, code.clone());
                    }
                }
                // Store the added storage in the account's storage trie and compute its new root
                if !update.added_storage.is_empty() {
                    let storage_trie =
                        storage_tries_map.entry(update.address).or_insert_with(|| {
                            Trie::from_nodes(None, &[]).expect("failed to create empty trie")
                        });

                    for (storage_key, storage_value) in &update.added_storage {
                        let hashed_key = hash_key(storage_key);
                        if storage_value.is_zero() {
                            storage_trie
                                .remove(hashed_key)
                                .expect("failed to remove key");
                        } else {
                            storage_trie
                                .insert(hashed_key, storage_value.encode_to_vec())
                                .expect("failed to insert in trie");
                        }
                    }
                    account_state.storage_root = storage_trie.hash_no_commit();
                }
                state_trie
                    .insert(hashed_address, account_state.encode_to_vec())
                    .expect("failed to insert into storage");
            }
        }
        Ok(())
    }

    pub fn state_trie_root(&self) -> Result<H256, ExecutionWitnessError> {
        let state_trie = self
            .state_trie
            .as_ref()
            .ok_or(ExecutionWitnessError::RebuildTrie(
                "Tried to get state trie root before rebuilding tries".to_string(),
            ))?;

        Ok(state_trie.hash_no_commit())
    }

    /// Returns Some(block_number) if the hash for block_number is not the parent
    /// hash of block_number + 1. None if there's no such hash.
    ///
    /// Keep in mind that the last block hash (which is a batch's parent hash)
    /// can't be validated against the next header, because it has no successor.
    pub fn get_first_invalid_block_hash(&self) -> Result<Option<u64>, ExecutionWitnessError> {
        // Enforces there's at least one block header, so windows() call doesn't panic.
        if self.block_headers.is_empty() {
            return Err(ExecutionWitnessError::NoBlockHeaders);
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
                return Err(ExecutionWitnessError::Unreachable(
                    "block header window len is < 2".to_string(),
                ));
            };
            if *next_number != *number + 1 {
                return Err(ExecutionWitnessError::NoncontiguousBlockHeaders);
            }
            if next_header.parent_hash != header.hash() {
                return Ok(Some(*number));
            }
        }

        Ok(None)
    }

    pub fn get_block_parent_header(
        &self,
        block_number: u64,
    ) -> Result<&BlockHeader, ExecutionWitnessError> {
        self.block_headers
            .get(&block_number.saturating_sub(1))
            .ok_or(ExecutionWitnessError::NoBlockHeaders)
    }

    pub fn get_account_info(
        &self,
        address: Address,
    ) -> Result<Option<AccountInfo>, ExecutionWitnessError> {
        let state_trie = self
            .state_trie
            .as_ref()
            .ok_or(ExecutionWitnessError::Database(
                "ExecutionWitness: Tried to get state trie before rebuilding tries".to_string(),
            ))?;

        let hashed_address = hash_address(&address);
        let Ok(Some(encoded_state)) = state_trie.get(&hashed_address) else {
            return Ok(None);
        };
        let state = AccountState::decode(&encoded_state).map_err(|_| {
            ExecutionWitnessError::Database("Failed to get decode account from trie".to_string())
        })?;

        Ok(Some(AccountInfo {
            balance: state.balance,
            code_hash: state.code_hash,
            nonce: state.nonce,
        }))
    }

    pub fn get_block_hash(&self, block_number: u64) -> Result<H256, ExecutionWitnessError> {
        self.block_headers
            .get(&block_number)
            .map(|header| header.hash())
            .ok_or_else(|| {
                ExecutionWitnessError::Database(format!(
                    "Block hash not found for block number {block_number}"
                ))
            })
    }

    pub fn get_storage_slot(
        &self,
        address: Address,
        key: H256,
    ) -> Result<Option<U256>, ExecutionWitnessError> {
        let storage_tries_map =
            self.storage_tries
                .as_ref()
                .ok_or(ExecutionWitnessError::Database(
                    "ExecutionWitness: Tried to get storage slot before rebuilding tries"
                        .to_string(),
                ))?;

        let Some(storage_trie) = storage_tries_map.get(&address) else {
            return Ok(None);
        };
        let hashed_key = hash_key(&key);
        if let Some(encoded_key) = storage_trie
            .get(&hashed_key)
            .map_err(|e| ExecutionWitnessError::Database(e.to_string()))?
        {
            U256::decode(&encoded_key)
                .map_err(|_| {
                    ExecutionWitnessError::Database("failed to read storage from trie".to_string())
                })
                .map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn get_chain_config(&self) -> Result<ChainConfig, ExecutionWitnessError> {
        Ok(self.chain_config)
    }

    pub fn get_account_code(&self, code_hash: H256) -> Result<bytes::Bytes, ExecutionWitnessError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Bytes::new());
        }
        match self.codes_map.get(&code_hash) {
            Some(code) => Ok(code.clone()),
            None => Err(ExecutionWitnessError::Database(format!(
                "Could not find code for hash {code_hash}"
            ))),
        }
    }
}

// TODO: Make an RpcExecutionWitness struct to decouple the RPC response from the execution witness result.
impl<'de> Deserialize<'de> for ExecutionWitnessResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let response = HashMap::<String, Vec<String>>::deserialize(deserializer)?;

        let state: Vec<Bytes> = response
            .get("state")
            .ok_or_else(|| de::Error::custom("Missing or invalid 'state' field"))?
            .iter()
            .map(|str| {
                hex::decode(str.trim_start_matches("0x"))
                    .map_err(|e| <D::Error as de::Error>::custom(e.to_string()))
                    .map(Into::into)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(de::Error::custom)?;

        let keys: Vec<Bytes> = response
            .get("keys")
            .ok_or_else(|| de::Error::custom("Missing or invalid 'keys' field"))?
            .iter()
            .map(|str| {
                hex::decode(str.trim_start_matches("0x"))
                    .map_err(|e| <D::Error as de::Error>::custom(e.to_string()))
                    .map(Into::into)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(de::Error::custom)?;

        let codes: Vec<Bytes> = response
            .get("codes")
            .ok_or_else(|| de::Error::custom("Missing or invalid 'codes' field"))?
            .iter()
            .map(|str| {
                hex::decode(str.trim_start_matches("0x"))
                    .map_err(|e| <D::Error as de::Error>::custom(e.to_string()))
                    .map(Into::into)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(de::Error::custom)?;

        let headers: Vec<Bytes> = response
            .get("headers")
            .ok_or_else(|| de::Error::custom("Missing or invalid 'headers' field"))?
            .iter()
            .map(|str| {
                hex::decode(str.trim_start_matches("0x"))
                    .map_err(|e| <D::Error as de::Error>::custom(e.to_string()))
                    .map(Into::into)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(de::Error::custom)?;

        let codes_map = codes
            .iter()
            .map(|code| (keccak_hash::keccak(code), code.clone()))
            .collect::<HashMap<_, _>>();

        let block_headers = headers
            .iter()
            .map(Bytes::as_ref)
            .map(BlockHeader::decode)
            .collect::<Result<Vec<_>, _>>()
            .map_err(de::Error::custom)?
            .iter()
            .map(|header| (header.number, header.clone()))
            .collect::<HashMap<_, _>>();

        Ok(Self {
            state,
            keys,
            codes,
            headers,
            codes_map,
            state_trie: None,
            storage_tries: None,
            block_headers,
            chain_config: ChainConfig::default(),
        })
    }
}

fn hash_address(address: &Address) -> Vec<u8> {
    Keccak256::new_with_prefix(address.to_fixed_bytes())
        .finalize()
        .to_vec()
}

pub fn hash_key(key: &H256) -> Vec<u8> {
    Keccak256::new_with_prefix(key.to_fixed_bytes())
        .finalize()
        .to_vec()
}
