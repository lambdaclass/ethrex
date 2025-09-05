use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use crate::types::Block;
use crate::{
    H160,
    constants::EMPTY_KECCACK_HASH,
    types::{AccountInfo, AccountState, AccountUpdate, BlockHeader, ChainConfig},
    utils::decode_hex,
};
use bytes::Bytes;
use ethereum_types::{Address, H256, U256};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_trie::{NodeHash, NodeRLP, Trie};
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::de::{SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha3::{Digest, Keccak256};

/// In-memory execution witness database for single batch execution data.
///
/// This is mainly used to store the relevant state data for executing a single batch and then
/// feeding the DB into a zkVM program to prove the execution.
#[derive(Serialize, Deserialize, Default, RSerialize, RDeserialize, Archive)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionWitnessResult {
    // Indexed by code hash
    // Used evm bytecodes
    #[serde(
        serialize_with = "serialize_code",
        deserialize_with = "deserialize_code"
    )]
    #[rkyv(with=rkyv::with::MapKV<crate::rkyv_utils::H256Wrapper, crate::rkyv_utils::BytesWrapper>)]
    pub codes: BTreeMap<H256, Bytes>,
    // Pruned state MPT
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    pub state_trie: Option<Trie>,
    // Storage tries accessed by account address
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    pub storage_tries: BTreeMap<Address, Trie>,
    // Block headers needed for BLOCKHASH opcode
    pub block_headers: BTreeMap<u64, BlockHeader>,
    // Parent block header to get the initial state root
    pub parent_block_header: BlockHeader,
    // Chain config
    pub chain_config: ChainConfig,
    /// This maps node hashes to their corresponding RLP-encoded nodes.
    /// It is used to rebuild the state trie and storage tries.
    /// This is precomputed during ExecutionWitness construction to avoid
    /// recomputing it when rebuilding tries.
    #[rkyv(with=rkyv::with::MapKV<crate::rkyv_utils::H256Wrapper, rkyv::with::AsBox>)]
    pub state_nodes: BTreeMap<H256, NodeRLP>,
    /// This is a convenience map to track which accounts and storage slots were touched during execution.
    /// It maps an account address to a vector of all storage slots that were accessed for that account.
    /// This is needed for building `RpcExecutionWitness`.
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    pub touched_account_storage_slots: BTreeMap<Address, Vec<H256>>,
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    pub account_hashes_by_address: BTreeMap<Address, Vec<u8>>,
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
    #[error("Parent block header of block {0} was not found")]
    MissingParentHeaderOf(u64),
    #[error("Non-contiguous block headers (there's a gap in the block headers list)")]
    NoncontiguousBlockHeaders,
    #[error("Unreachable code reached: {0}")]
    Unreachable(String),
    #[error("Custom error: {0}")]
    Custom(String),
}

impl ExecutionWitnessResult {
    /// Use the state nodes to build the state trie and store them in `self.state_trie`
    /// This function will fail if the state trie cannot be rebuilt.
    pub fn rebuild_state_trie(&mut self) -> Result<(), ExecutionWitnessError> {
        if self.state_trie.is_some() {
            return Ok(());
        }

        let state_trie = Trie::from_nodes(
            NodeHash::Hashed(self.parent_block_header.state_root),
            &self.state_nodes,
        )
        .map_err(|e| {
            ExecutionWitnessError::RebuildTrie(format!("Failed to build state trie {e}"))
        })?;

        self.state_trie = Some(state_trie);

        Ok(())
    }

    /// Helper function to rebuild the storage trie for a given account address
    /// Returns if root is not empty, an Option with the rebuilt trie
    // This function is an option because we expect it to fail sometimes, and we just want to filter it
    pub fn rebuild_storage_trie(&mut self, address: &H160) -> Option<Trie> {
        let account_hash = self
            .account_hashes_by_address
            .entry(*address)
            .or_insert_with(|| hash_address(address));

        let account_state_rlp = self.state_trie.as_ref()?.get(account_hash).ok()??;

        let account_state = AccountState::decode(&account_state_rlp).ok()?;

        Trie::from_nodes(
            NodeHash::Hashed(account_state.storage_root),
            &self.state_nodes,
        )
        .ok()
    }

    /// Helper function to apply account updates to the execution witness
    /// It updates the state trie and storage tries with the given account updates
    /// Returns an error if the updates cannot be applied
    pub fn apply_account_updates(
        &mut self,
        account_updates: &[AccountUpdate],
    ) -> Result<(), ExecutionWitnessError> {
        let (Some(state_trie), storage_tries) = (self.state_trie.as_mut(), &mut self.storage_tries)
        else {
            return Err(ExecutionWitnessError::ApplyAccountUpdates(
                "Tried to apply account updates before rebuilding the tries".to_string(),
            ));
        };

        for update in account_updates.iter() {
            let hashed_address = self
                .account_hashes_by_address
                .entry(update.address)
                .or_insert_with(|| hash_address(&update.address));

            if update.removed {
                // Remove account from trie
                state_trie
                    .remove(hashed_address)
                    .expect("failed to remove from trie");
            } else {
                // Add or update AccountState in the trie
                // Fetch current state or create a new state to be inserted
                let mut account_state = match state_trie
                    .get(hashed_address)
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
                        self.codes.insert(info.code_hash, code.clone());
                    }
                }
                // Store the added storage in the account's storage trie and compute its new root
                if !update.added_storage.is_empty() {
                    let storage_trie = storage_tries
                        .entry(update.address)
                        .or_insert_with(Trie::empty_in_memory);

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
                        storage_trie
                            .insert(hashed_key, storage_value.encode_to_vec())
                            .expect("failed to insert in trie");
                    }

                    for (hashed_key, _) in deletes {
                        storage_trie
                            .remove(&hashed_key)
                            .expect("failed to remove key");
                    }

                    account_state.storage_root = storage_trie.hash_no_commit();
                }

                state_trie
                    .insert(hashed_address.clone(), account_state.encode_to_vec())
                    .expect("failed to insert into storage");
            }
        }
        Ok(())
    }

    /// Returns the root hash of the state trie
    /// Returns an error if the state trie is not built yet
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

    /// Retrieves the parent block header for the specified block number
    /// Searches within `self.block_headers`
    pub fn get_block_parent_header(
        &self,
        block_number: u64,
    ) -> Result<&BlockHeader, ExecutionWitnessError> {
        self.block_headers
            .get(&block_number.saturating_sub(1))
            .ok_or(ExecutionWitnessError::MissingParentHeaderOf(block_number))
    }

    /// Retrieves the account info based on what is stored in the state trie.
    /// Returns an error if the state trie is not rebuilt or if decoding the account state fails.
    pub fn get_account_info(
        &mut self,
        address: Address,
    ) -> Result<Option<AccountInfo>, ExecutionWitnessError> {
        let state_trie = self
            .state_trie
            .as_ref()
            .ok_or(ExecutionWitnessError::Database(
                "ExecutionWitness: Tried to get state trie before rebuilding tries".to_string(),
            ))?;

        let hashed_address = self
            .account_hashes_by_address
            .entry(address)
            .or_insert_with(|| hash_address(&address));

        let Ok(Some(encoded_state)) = state_trie.get(hashed_address) else {
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

    /// Fetches the block hash for a specific block number.
    /// Looks up `self.block_headers` and computes the hash if it is not already computed.
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

    /// Retrieves a storage slot value for an account in its storage trie.
    ///
    /// Lazily builds the storage trie for the address if not already available.
    /// This lazy loading approach minimizes memory usage by only building tries when needed.
    pub fn get_storage_slot(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<Option<U256>, ExecutionWitnessError> {
        self.touched_account_storage_slots
            .entry(address)
            .or_default()
            .push(key);

        let storage_trie = if let Some(storage_trie) = self.storage_tries.get(&address) {
            storage_trie
        } else {
            if self.state_trie.is_none() {
                return Err(ExecutionWitnessError::Database(
                    "ExecutionWitness: Tried to get storage slot before rebuilding state trie."
                        .to_string(),
                ));
            };

            let Some(storage_trie) = self.rebuild_storage_trie(&address) else {
                return Ok(None);
            };

            self.storage_tries.entry(address).or_insert(storage_trie)
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

    /// Retrieves the chain configuration for the execution witness.
    pub fn get_chain_config(&self) -> Result<ChainConfig, ExecutionWitnessError> {
        Ok(self.chain_config)
    }

    /// Retrieves the account code for a specific account.
    /// Returns an Err if the code is not found.
    pub fn get_account_code(&self, code_hash: H256) -> Result<bytes::Bytes, ExecutionWitnessError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Bytes::new());
        }
        match self.codes.get(&code_hash) {
            Some(code) => Ok(code.clone()),
            None => Err(ExecutionWitnessError::Database(format!(
                "Could not find code for hash {code_hash}"
            ))),
        }
    }

    /// Hashes all block headers, initializing their inner `hash` field
    pub fn initialize_block_header_hashes(
        &self,
        blocks: &[Block],
    ) -> Result<(), ExecutionWitnessError> {
        for block in blocks {
            let hash = self
                .block_headers
                .get(&block.header.number)
                .map(|header| header.hash())
                .ok_or(ExecutionWitnessError::Custom(
                    format!(
                        "execution witness does not contain the block header of a block to execute ({}), but contains headers {:?} to {:?}",
                        block.header.number,
                        self.block_headers.keys().min(),
                        self.block_headers.keys().max()
                    )
                ))?;
            // this returns err if it's already set, so we drop the Result as we don't
            // care if it was already initialized.
            let _ = block.header.hash.set(hash);
        }
        Ok(())
    }
}

pub fn serialize_code<S>(map: &BTreeMap<H256, Bytes>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut seq_serializer = serializer.serialize_seq(Some(map.len()))?;
    for (code_hash, code) in map {
        let code_hash = format!("0x{}", hex::encode(code_hash));
        let code = format!("0x{}", hex::encode(code));

        let mut obj = serde_json::Map::new();
        obj.insert(code_hash, serde_json::Value::String(code));

        seq_serializer.serialize_element(&obj)?;
    }
    seq_serializer.end()
}

pub fn deserialize_code<'de, D>(deserializer: D) -> Result<BTreeMap<H256, Bytes>, D::Error>
where
    D: Deserializer<'de>,
{
    struct BytesVecVisitor;

    impl<'de> Visitor<'de> for BytesVecVisitor {
        type Value = BTreeMap<H256, Bytes>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a list of hex-encoded strings")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut map = BTreeMap::new();

            #[derive(Deserialize)]
            struct CodeEntry(BTreeMap<String, String>);

            while let Some(CodeEntry(entry)) = seq.next_element::<CodeEntry>()? {
                if entry.len() != 1 {
                    return Err(de::Error::custom(
                        "Each object must contain exactly one key",
                    ));
                }

                for (k, v) in entry {
                    let code_hash =
                        H256::from_str(k.trim_start_matches("0x")).map_err(de::Error::custom)?;

                    let bytecode =
                        decode_hex(v.trim_start_matches("0x")).map_err(de::Error::custom)?;

                    map.insert(code_hash, Bytes::from(bytecode));
                }
            }
            Ok(map)
        }
    }

    deserializer.deserialize_seq(BytesVecVisitor)
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
