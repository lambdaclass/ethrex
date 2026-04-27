use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;

use crate::rkyv_utils::H256Wrapper;
use crate::serde_utils;
use crate::types::{Block, Code, CodeMetadata};
use crate::{
    constants::EMPTY_KECCACK_HASH,
    types::{AccountState, AccountUpdate, BlockHeader, ChainConfig},
    utils::keccak,
};
use ethereum_types::{Address, H256, U256};
use ethrex_crypto::{Crypto, NativeCrypto};
use ethrex_rlp::error::RLPDecodeError;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_trie::{EMPTY_TRIE_HASH, Nibbles, Node, NodeRef, Trie, TrieError};
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
    /// Map of hashed addresses to their corresponding storage tries.
    pub storage_tries: BTreeMap<H256, Trie>,
    /// Map of account addresses to their corresponding hashed addresses.
    /// This is a convenience map to avoid recomputing the hashed address
    /// multiple times during guest program execution.
    /// It is built on-demand during guest program execution, inside the zkVM.
    pub account_hashes_by_address: BTreeMap<Address, H256>,
    /// Map of hashed addresses to booleans, indicating whose account's storage tries were
    /// verified.
    /// Verification is done by hashing the trie and comparing the root hash with the account's storage root.
    pub verified_storage_roots: BTreeMap<H256, bool>,
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
    /// Root nodes per account storage embedded with the rest of the trie's nodes,
    /// keyed by the keccak256 hash of the account address.
    #[rkyv(with = MapKV<H256Wrapper, Identity>)]
    pub storage_trie_roots: BTreeMap<H256, Node>,
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
            keys: Vec::new(),
            codes: value.codes.into_iter().map(Bytes::from).collect(),
            headers: value
                .block_headers_bytes
                .into_iter()
                .map(Bytes::from)
                .collect(),
        })
    }
}

impl RpcExecutionWitness {
    /// Convert an RPC execution witness into the internal [`ExecutionWitness`]
    /// format by rebuilding trie structures from the flat node list.
    pub fn into_execution_witness(
        self,
        chain_config: ChainConfig,
        first_block_number: u64,
    ) -> Result<ExecutionWitness, GuestProgramStateError> {
        if first_block_number == 0 {
            return Err(GuestProgramStateError::Custom(
                "first_block_number must be > 0 (need parent header)".to_string(),
            ));
        }

        // Skip headers that fail to decode (tolerance) and pick the parent by number.
        let initial_state_root = self
            .headers
            .iter()
            .filter_map(|h| BlockHeader::decode(h).ok())
            .find(|header| header.number == first_block_number - 1)
            .map(|header| header.state_root)
            .ok_or_else(|| {
                GuestProgramStateError::Custom(format!(
                    "header for block {} not found",
                    first_block_number - 1
                ))
            })?;

        // Tolerate witness entries that fail to decode (malformed RLP, or the `Null`
        // node `[0x80]` other clients emit): drop them silently. They cannot be looked
        // up by hash, so tolerating them is safe. If execution actually requires one,
        // the trie walk will fail with a missing-node error (completeness check).
        //
        // Matches EELS `witness_state.build_node_db` and geth `MakeHashDB`, both of
        // which store entries keyed by hash without pre-validation:
        //   https://github.com/ethereum/execution-specs/blob/projects/zkevm/src/ethereum/forks/amsterdam/witness_state.py#L37-L42
        //   https://github.com/ethereum/go-ethereum/blob/master/core/stateless/database.go#L26-L67
        let nodes: BTreeMap<H256, Node> = self
            .state
            .into_iter()
            .filter_map(|b| {
                let node = Node::decode(&b).ok()?;
                Some((keccak(&b), node))
            })
            .collect();

        // get state trie root and embed the rest of the trie into it
        let state_trie_root = if let NodeRef::Node(state_trie_root, _) =
            Trie::get_embedded_root(&nodes, initial_state_root)?
        {
            Some((*state_trie_root).clone())
        } else {
            None
        };

        // Walk the state trie to discover accounts and their storage roots,
        // instead of relying on the keys field which is being removed from the RPC spec.
        let mut storage_trie_roots = BTreeMap::new();
        if let Some(state_trie_root) = &state_trie_root {
            let mut accounts = Vec::new();
            collect_accounts_from_trie(
                state_trie_root,
                Nibbles::from_raw(&[], false),
                &mut accounts,
                &nodes,
            );

            for (hashed_address, storage_root_hash) in accounts {
                if storage_root_hash == *EMPTY_TRIE_HASH {
                    continue;
                }
                if !nodes.contains_key(&storage_root_hash) {
                    continue;
                }
                let node = Trie::get_embedded_root(&nodes, storage_root_hash)?;
                let NodeRef::Node(node, _) = node else {
                    return Err(GuestProgramStateError::Custom(
                        "execution witness does not contain non-empty storage trie".to_string(),
                    ));
                };
                storage_trie_roots.insert(hashed_address, (*node).clone());
            }
        }

        Ok(ExecutionWitness {
            codes: self.codes.into_iter().map(|b| b.to_vec()).collect(),
            chain_config,
            first_block_number,
            block_headers_bytes: self.headers.into_iter().map(|b| b.to_vec()).collect(),
            state_trie_root,
            storage_trie_roots,
        })
    }
}

/// Recursively walks an embedded state trie node and collects
/// `(hashed_address, storage_root)` pairs from leaf nodes.
fn collect_accounts_from_trie(
    node: &Node,
    path: Nibbles,
    accounts: &mut Vec<(H256, H256)>,
    nodes: &BTreeMap<H256, Node>,
) {
    match node {
        Node::Branch(branch) => {
            for (i, child) in branch.choices.iter().enumerate() {
                let child_node: Option<&Node> = match child {
                    NodeRef::Node(n, _) => Some(n),
                    NodeRef::Hash(hash) if hash.is_valid() => {
                        nodes.get(&hash.finalize(&NativeCrypto))
                    }
                    _ => None,
                };
                if let Some(child_node) = child_node {
                    collect_accounts_from_trie(
                        child_node,
                        path.append_new(i as u8),
                        accounts,
                        nodes,
                    );
                }
            }
        }
        Node::Extension(ext) => {
            let child_node: Option<&Node> = match &ext.child {
                NodeRef::Node(n, _) => Some(n),
                NodeRef::Hash(hash) if hash.is_valid() => nodes.get(&hash.finalize(&NativeCrypto)),
                _ => None,
            };
            if let Some(child_node) = child_node {
                collect_accounts_from_trie(child_node, path.concat(&ext.prefix), accounts, nodes);
            }
        }
        Node::Leaf(leaf) => {
            let full_path = path.concat(&leaf.partial);
            let path_bytes = full_path.to_bytes();
            if path_bytes.len() == 32 {
                let hashed_address = H256::from_slice(&path_bytes);
                match AccountState::decode(&leaf.value) {
                    Ok(account_state) => {
                        accounts.push((hashed_address, account_state.storage_root));
                    }
                    Err(e) => {
                        tracing::debug!(
                            ?hashed_address,
                            error = %e,
                            "Skipping leaf with un-decodable account state"
                        );
                    }
                }
            } else {
                tracing::debug!(
                    path_len = path_bytes.len(),
                    "Skipping leaf with unexpected path length (expected 32)"
                );
            }
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
    Trie(#[from] TrieError),
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
        crypto: &dyn Crypto,
    ) -> Result<Self, GuestProgramStateError> {
        // Decode headers and verify they form a contiguous chain in list order:
        // every header's `parent_hash` must equal `keccak256(previous header bytes)`.
        // Mirrors EELS `stateless.validate_headers`:
        //   https://github.com/ethereum/execution-specs/blob/projects/zkevm/src/ethereum/forks/amsterdam/stateless.py#L171-L191
        // A non-contiguous chain (e.g. reordered headers) makes the witness invalid
        // even if a by-number lookup would otherwise resolve to the right header.
        let mut block_headers: BTreeMap<u64, BlockHeader> = BTreeMap::new();
        let mut prev_hash: Option<H256> = None;
        for bytes in &value.block_headers_bytes {
            let Ok(header) = BlockHeader::decode(bytes.as_ref()) else {
                // Tolerate malformed entries by treating them as a chain break:
                // subsequent headers will not satisfy the parent_hash check.
                prev_hash = None;
                continue;
            };
            if let Some(expected_parent) = prev_hash {
                if header.parent_hash != expected_parent {
                    return Err(GuestProgramStateError::Custom(
                        "witness headers are not contiguous".to_string(),
                    ));
                }
            }
            prev_hash = Some(H256(crypto.keccak256(bytes)));
            block_headers.insert(header.number, header);
        }

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
        state_trie.hash_no_commit(crypto);

        let mut storage_tries = BTreeMap::new();
        for (hashed_address, storage_trie_root) in value.storage_trie_roots {
            // hash storage trie nodes
            let storage_trie = Trie::new_temp_with_root(storage_trie_root.into());
            storage_trie.hash_no_commit(crypto);
            storage_tries.insert(hashed_address, storage_trie);
        }

        // hash codes
        // TODO: codes here probably needs to be Vec<Code>, rather than recomputing here. This requires rkyv implementation.
        let codes_hashed = value
            .codes
            .into_iter()
            .map(|code| {
                let code = Code::from_bytecode(code.into(), crypto);
                (code.hash, code)
            })
            .collect();

        Ok(GuestProgramState {
            codes_hashed,
            state_trie,
            storage_tries,
            block_headers,
            parent_block_header: parent_header,
            first_block_number: value.first_block_number,
            chain_config: value.chain_config,
            account_hashes_by_address: BTreeMap::new(),
            verified_storage_roots: BTreeMap::new(),
        })
    }
}

impl GuestProgramState {
    /// Helper function to apply account updates to the execution witness
    /// It updates the state trie and storage tries with the given account updates
    /// Returns an error if the updates cannot be applied
    pub fn apply_account_updates(
        &mut self,
        account_updates: &[AccountUpdate],
        crypto: &dyn Crypto,
    ) -> Result<(), GuestProgramStateError> {
        for update in account_updates.iter() {
            let hashed_address = *self
                .account_hashes_by_address
                .entry(update.address)
                .or_insert_with(|| hash_address(&update.address, crypto));

            if update.removed {
                // Remove account from trie
                self.state_trie.remove(hashed_address.as_bytes())?;
            } else {
                // Add or update AccountState in the trie
                // Fetch current state or create a new state to be inserted
                let mut account_state = match self.state_trie.get(hashed_address.as_bytes())? {
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
                    let storage_trie = self.storage_tries.entry(hashed_address).or_default();

                    // Inserts must come before deletes, otherwise deletes might require extra nodes
                    // Example:
                    // If I have a branch node [A, B] and want to delete A and insert C
                    // I will need to have B only if the deletion happens first
                    let (deletes, inserts): (Vec<_>, Vec<_>) = update
                        .added_storage
                        .iter()
                        .map(|(k, v)| (hash_key(k, crypto), v))
                        .partition(|(_k, v)| v.is_zero());

                    for (hashed_key, storage_value) in inserts {
                        storage_trie.insert(hashed_key, storage_value.encode_to_vec())?;
                    }

                    for (hashed_key, _) in deletes {
                        storage_trie.remove(&hashed_key)?;
                    }

                    let storage_root = storage_trie.hash_no_commit(crypto);
                    account_state.storage_root = storage_root;
                }

                self.state_trie.insert(
                    hashed_address.as_bytes().to_vec(),
                    account_state.encode_to_vec(),
                )?;
            }
        }
        Ok(())
    }

    /// Returns the root hash of the state trie
    /// Returns an error if the state trie is not built yet
    pub fn state_trie_root(&self, crypto: &dyn Crypto) -> Result<H256, GuestProgramStateError> {
        Ok(self.state_trie.hash_no_commit(crypto))
    }

    /// Returns Some(block_number) if the hash for block_number is not the parent
    /// hash of block_number + 1. None if there's no such hash.
    ///
    /// Keep in mind that the last block hash (which is a batch's parent hash)
    /// can't be validated against the next header, because it has no successor.
    pub fn get_first_invalid_block_hash(
        &self,
        crypto: &dyn Crypto,
    ) -> Result<Option<u64>, GuestProgramStateError> {
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

            if next_header.parent_hash != header.compute_block_hash(crypto) {
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
        crypto: &dyn Crypto,
    ) -> Result<Option<AccountState>, GuestProgramStateError> {
        let hashed_address = *self
            .account_hashes_by_address
            .entry(address)
            .or_insert_with(|| hash_address(&address, crypto));

        let Ok(Some(encoded_state)) = self.state_trie.get(hashed_address.as_bytes()) else {
            return Ok(None);
        };
        let state = AccountState::decode(&encoded_state).map_err(|_| {
            GuestProgramStateError::Database("Failed to get decode account from trie".to_string())
        })?;

        Ok(Some(state))
    }

    /// Fetches the block hash for a specific block number.
    /// Looks up `self.block_headers` and computes the hash if it is not already computed.
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

    /// Retrieves a storage slot value for an account in its storage trie.
    pub fn get_storage_slot(
        &mut self,
        address: Address,
        key: H256,
        crypto: &dyn Crypto,
    ) -> Result<Option<U256>, GuestProgramStateError> {
        let hashed_key = hash_key(&key, crypto);
        let Some(storage_trie) = self.get_valid_storage_trie(address, crypto)? else {
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

    /// Retrieves the account code for a specific code hash.
    ///
    /// Errors if the code is not present in the witness. Per the EELS reference
    /// implementation, a stateless executor that touches code missing from its
    /// witness MUST treat the witness as incomplete and reject:
    ///   https://github.com/ethereum/execution-specs/blob/projects/zkevm/src/ethereum/forks/amsterdam/witness_state.py#L204-L212
    /// Geth implements the same hash-keyed-store + error-on-miss model:
    ///   https://github.com/ethereum/go-ethereum/blob/master/core/stateless/database.go#L26-L46
    pub fn get_account_code(&self, code_hash: H256) -> Result<Code, GuestProgramStateError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Code::default());
        }
        self.codes_hashed.get(&code_hash).cloned().ok_or_else(|| {
            GuestProgramStateError::Database(format!(
                "missing bytecode for hash {} in witness",
                hex::encode(code_hash)
            ))
        })
    }

    /// Retrieves code metadata (length) for a specific code hash. Errors on miss for
    /// the same completeness reason as `get_account_code`.
    pub fn get_code_metadata(
        &self,
        code_hash: H256,
    ) -> Result<CodeMetadata, GuestProgramStateError> {
        use crate::constants::EMPTY_KECCACK_HASH;

        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(CodeMetadata { length: 0 });
        }
        self.codes_hashed
            .get(&code_hash)
            .map(|code| CodeMetadata {
                length: code.bytecode.len() as u64,
            })
            .ok_or_else(|| {
                GuestProgramStateError::Database(format!(
                    "missing bytecode for hash {} in witness",
                    hex::encode(code_hash)
                ))
            })
    }

    /// When executing multiple blocks in the L2 it happens that the headers in block_headers correspond to the same block headers that we have in the blocks array. The main goal is to hash these only once and set them in both places.
    /// We also initialize the remaining block headers hashes. If they are set, we check their validity.
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
                // We have already set this hash in the previous step
                continue;
            }
            let hash = header.compute_block_hash(crypto);
            set_hash_or_validate(header, hash)?;
        }

        Ok(())
    }

    pub fn get_valid_storage_trie(
        &mut self,
        address: Address,
        crypto: &dyn Crypto,
    ) -> Result<Option<&Trie>, GuestProgramStateError> {
        let hashed_address = *self
            .account_hashes_by_address
            .entry(address)
            .or_insert_with(|| hash_address(&address, crypto));

        let is_storage_verified = *self
            .verified_storage_roots
            .get(&hashed_address)
            .unwrap_or(&false);
        if is_storage_verified {
            Ok(self.storage_tries.get(&hashed_address))
        } else {
            let Some(storage_root) = self
                .get_account_state(address, crypto)?
                .map(|a| a.storage_root)
            else {
                // empty account
                return Ok(None);
            };
            let storage_trie = match self.storage_tries.get(&hashed_address) {
                None if storage_root == *EMPTY_TRIE_HASH => return Ok(None),
                Some(trie) if trie.hash_no_commit(crypto) == storage_root => trie,
                _ => {
                    return Err(GuestProgramStateError::Custom(format!(
                        "invalid storage trie for account {address}"
                    )));
                }
            };
            self.verified_storage_roots.insert(hashed_address, true);
            Ok(Some(storage_trie))
        }
    }
}

fn hash_address(address: &Address, crypto: &dyn Crypto) -> H256 {
    H256(crypto.keccak256(&address.to_fixed_bytes()))
}

pub fn hash_key(key: &H256, crypto: &dyn Crypto) -> Vec<u8> {
    crypto.keccak256(&key.to_fixed_bytes()).to_vec()
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
