use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ethrex_common::{
    Address, H256,
    constants::EMPTY_KECCACK_HASH,
    types::{AccountState, AccountUpdate, Block, BlockHeader, ChainConfig, Code, CodeMetadata},
};

use crate::execution_witness::{ExecutionWitness, GuestProgramStateError};
use crate::mpt_backend::MptBackend;
use ethrex_crypto::{Crypto, NativeCrypto};
use ethrex_rlp::decode::RLPDecode;

use crate::{EMPTY_TRIE_HASH, Nibbles, Node, NodeRef, Trie};

/// State produced by the guest program execution inside the zkVM. It is
/// essentially built from the `ExecutionWitness`.
/// This state is used during the stateless validation of the zkVM execution.
/// Some data is prepared before the stateless validation, and some data is
/// built on-demand during the stateless validation.
/// This struct must be instantiated, filled, and consumed inside the zkVM.
pub struct GuestProgramState {
    /// MPT backend holding the state trie, storage tries, and code cache.
    pub backend: MptBackend,
    /// Map of block numbers to their corresponding block headers.
    pub block_headers: BTreeMap<u64, BlockHeader>,
    /// The parent block header of the first block in the batch.
    pub parent_block_header: BlockHeader,
    /// The block number of the first block in the batch.
    pub first_block_number: u64,
    /// The chain configuration.
    pub chain_config: ChainConfig,
    /// Map of hashed addresses to booleans indicating whose storage tries were verified.
    pub verified_storage_roots: BTreeMap<H256, bool>,
}

impl GuestProgramState {
    pub fn from_witness(
        value: ExecutionWitness,
        crypto: Arc<dyn Crypto + Send + Sync>,
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

        let codes_hashed: BTreeMap<H256, Code> = value
            .codes
            .into_iter()
            .map(|code| {
                let code = Code::from_bytecode(code.into(), crypto.as_ref());
                (code.hash, code)
            })
            .collect();

        let backend = MptBackend::from_witness_bytes(
            value.state_proof,
            parent_header.state_root,
            codes_hashed,
            crypto,
        )
        .map_err(|e| GuestProgramStateError::RebuildTrie(e.to_string()))?;

        Ok(GuestProgramState {
            backend,
            block_headers,
            parent_block_header: parent_header,
            first_block_number: value.first_block_number,
            chain_config: value.chain_config,
            verified_storage_roots: BTreeMap::new(),
        })
    }
}

impl GuestProgramState {
    /// Apply account updates to the execution witness state.
    ///
    /// Updates the state trie and storage tries with the given account updates.
    pub fn apply_account_updates(
        &mut self,
        account_updates: &[AccountUpdate],
    ) -> Result<(), GuestProgramStateError> {
        use ethrex_state_backend::{AccountMut, CodeMut, StateCommitter};

        for update in account_updates {
            if update.removed {
                self.backend.update_accounts(
                    &[update.address],
                    &[AccountMut {
                        account: None,
                        code: None,
                        code_size: 0,
                    }],
                )?;
                continue;
            }

            if update.removed_storage {
                self.backend.clear_storage(update.address)?;
            }

            if let Some(info) = &update.info {
                let mut acct_mut = AccountMut {
                    account: Some(info.clone()),
                    code: None,
                    code_size: 0,
                };
                if let Some(code) = &update.code {
                    acct_mut.code = Some(CodeMut {
                        code: Some(code.bytecode.to_vec()),
                    });
                    acct_mut.code_size = code.bytecode.len();
                }
                self.backend
                    .update_accounts(&[update.address], &[acct_mut])?;
            }

            if !update.added_storage.is_empty() {
                let slots: Vec<(H256, H256)> = update
                    .added_storage
                    .iter()
                    .map(|(k, v)| (*k, H256::from(v.to_big_endian())))
                    .collect();
                self.backend.update_storage(update.address, &slots)?;
            }
        }

        // Compute storage roots and update state trie.
        // Uses hash_no_commit which is safe in the guest path.
        self.backend.flush_storage_roots()?;
        Ok(())
    }

    /// Returns the root hash of the state trie.
    pub fn state_trie_root(&self) -> Result<H256, GuestProgramStateError> {
        Ok(self.backend.hash_no_commit_state())
    }

    /// Returns Some(block_number) if the hash for block_number is not the parent
    /// hash of block_number + 1. None if there's no such hash.
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

    /// Retrieves the parent block header for the specified block number.
    pub fn get_block_parent_header(
        &self,
        block_number: u64,
    ) -> Result<&BlockHeader, GuestProgramStateError> {
        self.block_headers
            .get(&block_number.saturating_sub(1))
            .ok_or(GuestProgramStateError::MissingParentHeaderOf(block_number))
    }

    /// Retrieves the account state from the state trie.
    pub fn get_account_state(
        &self,
        address: Address,
    ) -> Result<Option<AccountState>, GuestProgramStateError> {
        self.backend
            .account_state(address)
            .map_err(|e| GuestProgramStateError::Database(e.to_string()))
    }

    /// Fetches the block hash for a specific block number.
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
    ) -> Result<Option<ethrex_common::U256>, GuestProgramStateError> {
        use ethrex_state_backend::StateReader;

        // Validate storage trie root (guest-specific security check).
        // Returns None if account has no storage.
        self.get_valid_storage_trie(address)?;

        // Read through StateReader; storage_tries is pre-loaded in the guest.
        let value_h256 = self
            .backend
            .storage(address, key)
            .map_err(|e| GuestProgramStateError::Database(e.to_string()))?;
        if value_h256.is_zero() {
            Ok(None)
        } else {
            Ok(Some(ethrex_common::U256::from_big_endian(
                value_h256.as_bytes(),
            )))
        }
    }

    /// Retrieves the chain configuration.
    pub fn get_chain_config(&self) -> Result<ChainConfig, GuestProgramStateError> {
        Ok(self.chain_config)
    }

    /// Retrieves the account code for a specific code hash.
    pub fn get_account_code(&self, code_hash: H256) -> Result<Code, GuestProgramStateError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(Code::default());
        }
        match self.backend.codes.get(&code_hash) {
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

    /// Retrieves code metadata (length) for a specific code hash.
    pub fn get_code_metadata(
        &self,
        code_hash: H256,
    ) -> Result<CodeMetadata, GuestProgramStateError> {
        if code_hash == *EMPTY_KECCACK_HASH {
            return Ok(CodeMetadata { length: 0 });
        }
        match self.backend.codes.get(&code_hash) {
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

    /// Initializes block header hashes, validating consistency between `blocks` and
    /// `block_headers`.
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

    /// Retrieves the valid storage trie for an account, verifying storage root integrity.
    pub fn get_valid_storage_trie(
        &mut self,
        address: Address,
    ) -> Result<Option<&Trie>, GuestProgramStateError> {
        let hashed_address = H256(self.backend.crypto.keccak256(&address.to_fixed_bytes()));

        let is_storage_verified = *self
            .verified_storage_roots
            .get(&hashed_address)
            .unwrap_or(&false);
        if is_storage_verified {
            Ok(self.backend.storage_tries.get(&hashed_address))
        } else {
            let Some(storage_root) = self.get_account_state(address)?.map(|a| a.storage_root)
            else {
                return Ok(None);
            };
            let crypto = Arc::clone(&self.backend.crypto);
            let storage_trie = match self.backend.storage_tries.get(&hashed_address) {
                None if storage_root == *EMPTY_TRIE_HASH => return Ok(None),
                Some(trie) if trie.hash_no_commit(crypto.as_ref()) == storage_root => trie,
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

fn set_hash_or_validate(header: &BlockHeader, hash: H256) -> Result<(), GuestProgramStateError> {
    if let Err(prev_hash) = header.hash.set(hash)
        && prev_hash != hash
    {
        return Err(GuestProgramStateError::Custom(format!(
            "Block header hash was previously set for {} with the wrong value.",
            header.number
        )));
    }
    Ok(())
}

/// Recursively walks an embedded state trie node and collects
/// `(hashed_address, storage_root)` pairs from leaf nodes.
pub(crate) fn collect_accounts_from_trie(
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
            use ethrex_rlp::decode::RLPDecode;
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
