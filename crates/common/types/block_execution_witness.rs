use core::fmt;
use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Arc, Mutex},
};

use crate::{
    H160,
    types::{AccountState, AccountUpdate, BlockHeader, ChainConfig},
    utils::decode_hex,
};
use bytes::Bytes;
use ethereum_types::Address;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_trie::{EMPTY_TRIE_HASH, Node, Trie};
use keccak_hash::H256;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, SeqAccess, Visitor},
    ser::{self, SerializeSeq},
};
use sha3::{Digest, Keccak256};

type StorageTrieNodes = HashMap<H160, Vec<Vec<u8>>>;

/// In-memory execution witness database for single batch execution data.
///
/// This is mainly used to store the relevant state data for executing a single batch and then
/// feeding the DB into a zkVM program to prove the execution.
#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionWitnessResult {
    // Rlp encoded state trie nodes
    #[serde(
        serialize_with = "serialize_proofs",
        deserialize_with = "deserialize_state"
    )]
    pub state_trie_nodes: Option<Vec<Vec<u8>>>,
    // Indexed by account
    // Rlp encoded state trie nodes
    #[serde(
        serialize_with = "serialize_storage_tries",
        deserialize_with = "deserialize_storage_tries"
    )]
    pub storage_trie_nodes: Option<HashMap<Address, Vec<Vec<u8>>>>,
    // Indexed by code hash
    // Used evm bytecodes
    #[serde(
        serialize_with = "serialize_code",
        deserialize_with = "deserialize_code"
    )]
    pub codes: HashMap<H256, Bytes>,
    // Pruned state MPT
    #[serde(skip)]
    pub state_trie: Option<Arc<Mutex<Trie>>>,
    // Indexed by account
    // Pruned storage MPT
    #[serde(skip)]
    pub storage_tries: Option<Arc<Mutex<HashMap<Address, Trie>>>>,
    // Block headers needed for BLOCKHASH opcode
    pub block_headers: HashMap<u64, BlockHeader>,
    // Parent block header to get the initial state root
    pub parent_block_header: BlockHeader,
    // Chain config
    pub chain_config: ChainConfig,
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
    pub fn rebuild_tries(&mut self) -> Result<(), ExecutionWitnessError> {
        let (Some(state_trie_nodes), Some(storage_trie_map)) = (
            self.state_trie_nodes.as_ref(),
            self.storage_trie_nodes.as_ref(),
        ) else {
            return Err(ExecutionWitnessError::RebuildTrie(
                "Tried to rebuild tries with empty nodes, rebuilding the trie can only be done once"
                    .to_string(),
            ));
        };

        let initial_state_root = self.parent_block_header.state_root;

        let mut initial_node = None;

        for node in state_trie_nodes.iter() {
            let x = Node::decode_raw(node).map_err(|_| {
                ExecutionWitnessError::RebuildTrie("Invalid state trie node in witness".to_string())
            })?;
            let hash = x.compute_hash().finalize();
            if hash == initial_state_root {
                initial_node = Some(node.clone());
                break;
            }
        }

        let state_trie =
            Trie::from_nodes(initial_node.as_ref(), state_trie_nodes).map_err(|e| {
                ExecutionWitnessError::RebuildTrie(format!("Failed to build state trie {e}"))
            })?;

        let mut storage_tries = HashMap::new();
        for (addr, nodes) in storage_trie_map {
            let hashed_address = hash_address(addr);
            let encoded_state = state_trie
                .get(&hashed_address)
                .expect("Failed to get from trie");

            let state = encoded_state
                .map(|encoded| AccountState::decode(&encoded))
                .unwrap_or_else(|| Ok(AccountState::default()))
                .expect("Failed to get account state");

            if state.storage_root == *EMPTY_TRIE_HASH {
                storage_tries.insert(
                    *addr,
                    Trie::from_nodes(None, nodes).map_err(|e| {
                        ExecutionWitnessError::RebuildTrie(format!(
                            "Failed to build storage trie {e}"
                        ))
                    })?,
                );
                continue;
            }

            let mut initial_node = None;

            for node in nodes.iter() {
                let x = Node::decode_raw(node).expect("invalid node");
                let hash = x.compute_hash().finalize();
                if hash == state.storage_root {
                    initial_node = Some(node);
                    break;
                }
            }

            let Ok(storage_trie) = Trie::from_nodes(initial_node, nodes) else {
                return Err(ExecutionWitnessError::RebuildTrie(
                    "Failed to rebuild storage trie".to_string(),
                ));
            };

            storage_tries.insert(*addr, storage_trie);
        }

        self.state_trie = Some(Arc::new(Mutex::new(state_trie)));
        self.storage_tries = Some(Arc::new(Mutex::new(storage_tries)));
        self.state_trie_nodes = None;
        self.storage_trie_nodes = None;

        Ok(())
    }

    pub fn apply_account_updates(
        &mut self,
        account_updates: &[AccountUpdate],
    ) -> Result<(), ExecutionWitnessError> {
        let (Some(state_trie), Some(storage_tries_map)) =
            (self.state_trie.as_ref(), self.storage_tries.as_ref())
        else {
            return Err(ExecutionWitnessError::ApplyAccountUpdates(
                "Tried to apply account updates before rebuilding the tries".to_string(),
            ));
        };

        let mut state_trie_lock = state_trie.lock().map_err(|_| {
            ExecutionWitnessError::ApplyAccountUpdates("Failed to lock state trie".to_string())
        })?;
        let mut storage_tries_lock = storage_tries_map.lock().map_err(|_| {
            ExecutionWitnessError::ApplyAccountUpdates("Failed to lock storage tries".to_string())
        })?;
        for update in account_updates.iter() {
            let hashed_address = hash_address(&update.address);
            if update.removed {
                // Remove account from trie
                state_trie_lock
                    .remove(hashed_address)
                    .expect("failed to remove from trie");
            } else {
                // Add or update AccountState in the trie
                // Fetch current state or create a new state to be inserted
                let mut account_state = match state_trie_lock
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
                        self.codes.insert(info.code_hash, code.clone());
                    }
                }
                // Store the added storage in the account's storage trie and compute its new root
                if !update.added_storage.is_empty() {
                    let storage_trie =
                        storage_tries_lock.entry(update.address).or_insert_with(|| {
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
                state_trie_lock
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
        let lock = state_trie.lock().map_err(|_| {
            ExecutionWitnessError::Database("Failed to lock state trie".to_string())
        })?;
        Ok(lock.hash_no_commit())
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
}

pub fn serialize_code<S>(map: &HashMap<H256, Bytes>, serializer: S) -> Result<S::Ok, S::Error>
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

pub fn serialize_storage_tries<S>(
    map: &Option<HashMap<H160, Vec<Vec<u8>>>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let Some(map) = map else {
        return Err(ser::Error::custom("Storage trie nodes is empty"));
    };
    let mut seq_serializer = serializer.serialize_seq(Some(map.len()))?;

    for (address, keys) in map {
        let address_hex = format!("0x{}", hex::encode(address));
        let values_hex: Vec<String> = keys
            .iter()
            .map(|v| format!("0x{}", hex::encode(v)))
            .collect();

        let mut obj = serde_json::Map::new();
        obj.insert(
            address_hex,
            serde_json::Value::Array(
                values_hex
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );

        seq_serializer.serialize_element(&obj)?;
    }

    seq_serializer.end()
}

pub fn deserialize_state<'de, D>(deserializer: D) -> Result<Option<Vec<Vec<u8>>>, D::Error>
where
    D: Deserializer<'de>,
{
    struct HexVecVisitor;

    impl<'de> Visitor<'de> for HexVecVisitor {
        type Value = Option<Vec<Vec<u8>>>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a list of hex-encoded strings")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut out = Vec::new();
            while let Some(s) = seq.next_element::<String>()? {
                let bytes = decode_hex(&s).map_err(de::Error::custom)?;
                out.push(bytes);
            }
            Ok(Some(out))
        }
    }

    deserializer.deserialize_seq(HexVecVisitor)
}

pub fn deserialize_code<'de, D>(deserializer: D) -> Result<HashMap<H256, Bytes>, D::Error>
where
    D: Deserializer<'de>,
{
    struct BytesVecVisitor;

    impl<'de> Visitor<'de> for BytesVecVisitor {
        type Value = HashMap<H256, Bytes>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a list of hex-encoded strings")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut map = HashMap::new();

            #[derive(Deserialize)]
            struct CodeEntry(HashMap<String, String>);

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

pub fn deserialize_storage_tries<'de, D>(
    deserializer: D,
) -> Result<Option<StorageTrieNodes>, D::Error>
where
    D: Deserializer<'de>,
{
    struct KeysVisitor;

    impl<'de> Visitor<'de> for KeysVisitor {
        type Value = Option<StorageTrieNodes>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str(
                "a list of maps with H160 keys and array of hex-encoded strings as values",
            )
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut map = HashMap::new();

            #[derive(Deserialize)]
            struct KeyEntry(HashMap<String, Vec<String>>);

            while let Some(entry) = seq.next_element::<KeyEntry>()? {
                let obj = entry.0;
                if obj.len() != 1 {
                    return Err(de::Error::custom(
                        "Each object must contain exactly one key",
                    ));
                }

                for (k, v) in obj {
                    let h160 =
                        H160::from_str(k.trim_start_matches("0x")).map_err(de::Error::custom)?;

                    let vecs = v
                        .into_iter()
                        .map(|s| {
                            let s = s.trim_start_matches("0x");
                            hex::decode(s).map_err(de::Error::custom)
                        })
                        .collect::<Result<Vec<_>, _>>()?;

                    map.insert(h160, vecs);
                }
            }

            Ok(Some(map))
        }
    }

    deserializer.deserialize_seq(KeysVisitor)
}

pub fn serialize_proofs<S>(
    state_trie_nodes: &Option<Vec<Vec<u8>>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let Some(state_trie_nodes) = state_trie_nodes else {
        return Err(ser::Error::custom("State trie nodes is empty"));
    };
    let mut seq_serializer = serializer.serialize_seq(Some(state_trie_nodes.len()))?;
    for encoded_node in state_trie_nodes {
        seq_serializer.serialize_element(&format!("0x{}", hex::encode(encoded_node)))?;
    }
    seq_serializer.end()
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
