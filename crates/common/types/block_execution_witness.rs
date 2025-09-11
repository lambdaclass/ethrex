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
use keccak_hash::keccak;
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::de::{SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha3::{Digest, Keccak256};

/// State produced by the guest program execution inside the zkVM. It is
/// essentially built from the `ExecutionWitness`.
/// This state is used during the stateless validation of the zkVM execution.
/// Some data is prepared before the stateless validation, and some data is
/// built on-demand during the stateless validation.
/// This struct must be instantiated, filled, and consumed inside the zkVM.
pub struct GuestProgramState {
    /// Map of node hash to RLP-encoded node.
    /// This is computed during guest program execution inside the zkVM,
    /// before the stateless validation.
    /// It is used to rebuild the state trie and storage tries.
    pub nodes_hashed: BTreeMap<H256, NodeRLP>,
    /// Map of code hashes to their corresponding bytecode.
    /// This is computed during guest program execution inside the zkVM,
    /// before the stateless validation.
    pub codes_hashed: BTreeMap<H256, Vec<u8>>,
    /// Map of block numbers to their corresponding block headers.
    /// The block headers are pushed to the zkVM RLP-encoded, and then
    /// decoded and stored in this map during guest program execution,
    /// inside the zkVM.
    pub block_headers: BTreeMap<u64, BlockHeader>,
    /// The accounts state trie containing the necessary state for the guest
    /// program execution.
    /// The trie is built during guest program execution inside the zkVM,
    /// before the stateless validation.
    pub state_trie: Option<Trie>,
    /// The parent block header of the first block in the batch.
    pub parent_block_header: BlockHeader,
    /// The block number of the first block in the batch.
    pub first_block_number: u64,
    /// The chain configuration.
    pub chain_config: ChainConfig,
    /// Map of account addresses to their corresponding storage tries.
    /// This struct is initialized empty inside the zkVM and storage tries are
    /// built on-demand and cached here during guest program execution.
    pub storage_tries: BTreeMap<Address, Trie>,
    /// Map of account addresses to their corresponding hashed addresses.
    /// This is a convenience map to avoid recomputing the hashed address
    /// multiple times during guest program execution.
    /// It is built on-demand during guest program execution, inside the zkVM.
    pub account_hashes_by_address: BTreeMap<Address, Vec<u8>>,
}

/// Witness data produced by the client and consumed by the guest program
/// inside the zkVM.
///
/// It is essentially an `RpcExecutionWitness` but it also contains `ChainConfig`,
/// and `first_block_number`.
#[derive(Serialize, Deserialize, Default, RSerialize, RDeserialize, Archive, Clone)]
#[serde(rename_all = "camelCase")]
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
    /// RLP-encoded trie nodes needed for stateless execution.
    #[rkyv(with = crate::rkyv_utils::VecVecWrapper)]
    pub nodes: Vec<Vec<u8>>,
    /// Flattened map of account addresses and storage keys whose values
    /// are needed for stateless execution.
    #[rkyv(with = crate::rkyv_utils::VecVecWrapper)]
    pub keys: Vec<Vec<u8>>,
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

        // hash nodes
        let nodes_hashed = value
            .nodes
            .into_iter()
            .map(|node| {
                let node = node.to_vec();
                (keccak(&node), node)
            })
            .collect();

        // hash codes
        let codes_hashed = value
            .codes
            .into_iter()
            .map(|code| (keccak(&code), code))
            .collect();

        let mut guest_program_state = GuestProgramState {
            codes_hashed,
            state_trie: None,
            storage_tries: BTreeMap::new(),
            block_headers,
            parent_block_header: parent_header,
            first_block_number: value.first_block_number,
            chain_config: value.chain_config,
            nodes_hashed,
            account_hashes_by_address: BTreeMap::new(),
        };

        guest_program_state.rebuild_state_trie().map_err(|_| {
            GuestProgramStateError::RebuildTrie(
                "Failed to rebuild state trie from execution witness".to_owned(),
            )
        })?;

        Ok(guest_program_state)
    }
}

impl GuestProgramState {
    /// Use the state nodes to build the state trie and store them in `self.state_trie`
    /// This function will fail if the state trie cannot be rebuilt.
    pub fn rebuild_state_trie(&mut self) -> Result<(), GuestProgramStateError> {
        if self.state_trie.is_some() {
            return Ok(());
        }

        let state_trie = Trie::from_nodes(
            NodeHash::Hashed(self.parent_block_header.state_root),
            &self.nodes_hashed,
        )
        .map_err(|e| {
            GuestProgramStateError::RebuildTrie(format!("Failed to build state trie {e}"))
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
            &self.nodes_hashed,
        )
        .ok()
    }

    /// Helper function to apply account updates to the execution witness
    /// It updates the state trie and storage tries with the given account updates
    /// Returns an error if the updates cannot be applied
    pub fn apply_account_updates(
        &mut self,
        account_updates: &[AccountUpdate],
    ) -> Result<(), GuestProgramStateError> {
        let (Some(state_trie), storage_tries) = (self.state_trie.as_mut(), &mut self.storage_tries)
        else {
            return Err(GuestProgramStateError::ApplyAccountUpdates(
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
                        self.codes_hashed.insert(info.code_hash, code.to_vec());
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
    pub fn state_trie_root(&self) -> Result<H256, GuestProgramStateError> {
        let state_trie = self
            .state_trie
            .as_ref()
            .ok_or(GuestProgramStateError::RebuildTrie(
                "Tried to get state trie root before rebuilding tries".to_string(),
            ))?;

        Ok(state_trie.hash_no_commit())
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

    /// Retrieves the account info based on what is stored in the state trie.
    /// Returns an error if the state trie is not rebuilt or if decoding the account state fails.
    pub fn get_account_info(
        &mut self,
        address: Address,
    ) -> Result<Option<AccountInfo>, GuestProgramStateError> {
        let state_trie = self
            .state_trie
            .as_ref()
            .ok_or(GuestProgramStateError::Database(
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
            GuestProgramStateError::Database("Failed to get decode account from trie".to_string())
        })?;

        Ok(Some(AccountInfo {
            balance: state.balance,
            code_hash: state.code_hash,
            nonce: state.nonce,
        }))
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
    ///
    /// Lazily builds the storage trie for the address if not already available.
    /// This lazy loading approach minimizes memory usage by only building tries when needed.
    pub fn get_storage_slot(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<Option<U256>, GuestProgramStateError> {
        let storage_trie = if let Some(storage_trie) = self.storage_tries.get(&address) {
            storage_trie
        } else {
            if self.state_trie.is_none() {
                return Err(GuestProgramStateError::Database(
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
    pub fn get_account_code(
        &self,
        code_hash: H256,
    ) -> Result<bytes::Bytes, GuestProgramStateError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Bytes::new());
        }
        match self.codes_hashed.get(&code_hash) {
            Some(code) => Ok(Bytes::copy_from_slice(code)),
            None => {
                // We do this because what usually happens is that the Witness doesn't have the code we asked for but it is because it isn't relevant for that particular case.
                // In client implementations there are differences and it's natural for some clients to access more/less information in some edge cases.
                // Sidenote: logger doesn't work inside SP1, that's why we use println!
                // println!(
                //     "Missing bytecode for hash {} in witness. Defaulting to empty code.", // If there's a state root mismatch and this prints we have to see if it's the cause or not.
                //     hex::encode(code_hash)
                // );
                Ok(Bytes::new())
            }
        }
    }

    /// Hashes headers in witness and in blocks only once if they are repeated to avoid double hashing.
    pub fn initialize_block_header_hashes(
        &self,
        blocks: &[Block],
    ) -> Result<(), GuestProgramStateError> {
        // First we need to ensure that the block headers are initialized not before the guest program is executed
        for header in self.block_headers.values() {
            if header.hash.get().is_some() {
                return Err(GuestProgramStateError::Custom(format!(
                    "Block header hash is already set for {}",
                    header.number
                )));
            }
        }

        // Now we initialize the block_headers hashes and check the remaining blocks hashes
        for block in blocks {
            // Verify each block's header hash is uninitialized
            if block.header.hash.get().is_some() {
                return Err(GuestProgramStateError::Custom(format!(
                    "Block header hash is already set for {}",
                    block.header.number
                )));
            }
            let header = self
                .block_headers
                .get(&block.header.number)
                .unwrap_or(&block.header);

            let hash = header.hash();
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
