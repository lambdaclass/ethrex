//! Producing and consuming the EIP-8297 execution witness.
//!
//! Two halves that have to agree, and are written next to each other so they
//! keep agreeing:
//!
//! - [`Blockchain::generate_binary_witness_for_blocks`] re-executes the blocks
//!   against the store, replays what that execution touched through a
//!   *recording* binary trie, and hands back the node encodings the replay
//!   read;
//! - [`BinaryWitnessVmDatabase`] plus [`recompute_post_state_root`] take those
//!   encodings and nothing else, re-execute, and produce a post-state root.
//!
//! # Why generation replays rather than executing through the recorder
//!
//! Execution reads state through a [`VmDatabase`]; the store's is
//! `StoreVmDatabase`, which opens its own trie per read and cannot be handed a
//! recorder. Rather than a second execution path (which could drift from the
//! real one, and drift is how a witness comes to prove the wrong thing), the
//! block is executed exactly the way the node executes it, with a
//! [`DatabaseLogger`] noting which accounts, slots, codes and block hashes it
//! touched, and *then* those same reads are replayed against a recording trie.
//!
//! The replay goes through the same [`pbt_state`] accessors the verifier's
//! database calls, so it walks the same keys and touches the same nodes. What
//! makes that sound rather than hopeful is that the set of nodes a read needs
//! is a function of the key, not of the order or the caller — and the
//! end-to-end test re-executes from the witness alone and checks the root, so a
//! replay that missed something fails there rather than silently.
//!
//! [`pbt_state`]: ethrex_common::types::pbt_state
//! [`DatabaseLogger`]: ethrex_vm::backends::levm::db::DatabaseLogger

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use ethrex_common::constants::EMPTY_KECCAK_HASH;
use ethrex_common::types::binary_execution_witness::{
    BinaryExecutionWitness, BinaryWitnessError, RpcBinaryExecutionWitness,
};
use ethrex_common::types::{
    AccountState, Block, BlockHeader, ChainConfig, Code, CodeMetadata, pbt_state,
};
use ethrex_common::{Address, H256, U256, constants::EMPTY_TRIE_HASH};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::error::RLPDecodeError;
use ethrex_vm::backends::levm::db::DatabaseLogger;
use ethrex_vm::{DynVmDatabase, EvmError, VmDatabase};

use ethrex_binary_trie::trie::BinaryTrie;

use crate::error::ChainError;
use crate::vm::StoreVmDatabase;
use crate::{Blockchain, BlockchainType};

impl Blockchain {
    /// The EIP-8297 execution witness for `blocks`.
    ///
    /// Every block must be binary-committed; the caller decides that per header
    /// (the RPC guard does) rather than per chain, and this function does not
    /// re-derive it — it would have to ask the same question twice and could
    /// answer it differently.
    ///
    /// # Errors
    ///
    /// [`ChainError::WitnessGeneration`] if the node has no binary pre-state
    /// for the first block's parent, if the replay cannot reproduce the block's
    /// committed root, or if any piece the witness needs is missing.
    pub async fn generate_binary_witness_for_blocks(
        &self,
        blocks: &[Block],
    ) -> Result<BinaryExecutionWitness, ChainError> {
        let first = &blocks
            .first()
            .ok_or_else(|| ChainError::WitnessGeneration("Empty block batch".to_string()))?
            .header;

        let parent_header = self
            .storage
            .get_block_header_by_hash(first.parent_hash)
            .map_err(ChainError::StoreError)?
            .ok_or(ChainError::ParentNotFound)?;

        // Not `parent_header.state_root`: the first binary-committed block's
        // parent is pre-flip and commits an MPT root. See
        // `Store::binary_pre_state_root`.
        let pre_state_root = self
            .storage
            .binary_pre_state_root(&parent_header)
            .map_err(ChainError::StoreError)?
            .ok_or_else(|| {
                ChainError::WitnessGeneration(format!(
                    "no binary pre-state recorded for block {}'s parent",
                    first.number
                ))
            })?;

        let (mut trie, recorded) = self
            .storage
            .binary_trie_recording_view(first.parent_hash, pre_state_root)
            .map_err(ChainError::StoreError)?;

        let mut codes: Vec<Vec<u8>> = Vec::new();
        let mut blockhash_references: HashMap<u64, H256> = HashMap::new();

        for block in blocks {
            let block_parent = self
                .storage
                .get_block_header_by_hash(block.header.parent_hash)
                .map_err(ChainError::StoreError)?
                .ok_or(ChainError::ParentNotFound)?;

            let vm_db: DynVmDatabase =
                Box::new(StoreVmDatabase::new(self.storage.clone(), block_parent)?);
            let logger = Arc::new(DatabaseLogger::new(Arc::new(vm_db)));
            let mut vm = match self.options.r#type {
                BlockchainType::L1 => {
                    ethrex_vm::Evm::new_from_db_for_l1(logger.clone(), Arc::new(NativeCrypto))
                }
                BlockchainType::L2(_) => {
                    return Err(ChainError::WitnessGeneration(
                        "an L2 chain does not schedule binaryTreeTime".to_string(),
                    ));
                }
            };
            vm.execute_block(block)?;
            let account_updates = vm.get_state_transitions()?;

            let state_accessed = logger
                .state_accessed
                .lock()
                .map_err(|_| {
                    ChainError::WitnessGeneration("Failed to read accessed state".to_string())
                })?
                .clone();

            blockhash_references.extend(
                logger
                    .block_hashes_accessed
                    .lock()
                    .map_err(|_| {
                        ChainError::WitnessGeneration(
                            "Failed to read accessed block hashes".to_string(),
                        )
                    })?
                    .iter()
                    .map(|(number, hash)| (*number, *hash)),
            );

            for code_hash in logger
                .code_accessed
                .lock()
                .map_err(|_| {
                    ChainError::WitnessGeneration("Failed to read accessed codes".to_string())
                })?
                .iter()
            {
                if *code_hash == *EMPTY_KECCAK_HASH {
                    continue;
                }
                let code = self
                    .storage
                    .get_account_code(*code_hash)
                    .map_err(ChainError::StoreError)?
                    .ok_or_else(|| {
                        ChainError::WitnessGeneration(format!(
                            "no bytecode stored for {code_hash:#x}"
                        ))
                    })?;
                codes.push(code.code().to_vec());
            }

            // Replay the reads through the recorder. `get_account` covers the
            // storage-presence walks too, which is why it is called rather than
            // `get_account_info`.
            for (address, slots) in &state_accessed {
                pbt_state::get_account(&mut trie, *address).map_err(witness_read_error)?;
                let mut seen = HashSet::new();
                for slot in slots {
                    if !seen.insert(*slot) {
                        continue;
                    }
                    pbt_state::get_storage_slot(&mut trie, *address, slot)
                        .map_err(witness_read_error)?;
                }
            }
            // Withdrawal recipients are credited outside the EVM; the MPT
            // generator touches them by hand for the same reason.
            //
            // **Redundant, and kept deliberately.** This was recorded as
            // untested on the premise that the logger never sees these
            // addresses. It does: `LEVM::process_withdrawals` credits through
            // `get_account_mut` -> `load_account`, which faults the account
            // from the store — the `DatabaseLogger` during witness generation —
            // and that records it in `state_accessed`, which the replay above
            // already covers. `apply_account_updates` below then walks the same
            // paths again to write the credited balances.
            //
            // Verified by mutation against chains that really pay withdrawals
            // (`build_boundary_chains_paying_withdrawals`): deleting this
            // branch fails no test, because nothing depends on it rather than
            // because nothing exercises it. Kept as insurance against that
            // faulting behaviour being loosened later, at one trie read per
            // withdrawal. See docs/known_issues.md.
            if let Some(withdrawals) = block.body.withdrawals.as_ref() {
                for withdrawal in withdrawals {
                    pbt_state::get_account(&mut trie, withdrawal.address)
                        .map_err(witness_read_error)?;
                }
            }

            pbt_state::apply_account_updates(&mut trie, &account_updates)
                .map_err(witness_read_error)?;
        }

        // The self-check: the replay has to land on the root the last block
        // actually committed, or the nodes recorded are a witness for something
        // else. Producing an unverifiable witness silently is the failure mode
        // this whole method exists to avoid.
        let last = &blocks.last().expect("non-empty, checked above").header;
        let replayed = trie.root();
        if replayed != last.state_root {
            return Err(ChainError::WitnessGeneration(format!(
                "replaying block {} produced binary root {replayed:#x}, but the header commits \
                 {:#x}",
                last.number, last.state_root
            )));
        }

        let nodes: Vec<Vec<u8>> = recorded
            .lock()
            .map_err(|_| ChainError::WitnessGeneration("Failed to read witness nodes".to_string()))?
            .values()
            .cloned()
            .collect();

        let block_headers_bytes =
            self.witness_headers(last, &blockhash_references, &parent_header)?;

        Ok(BinaryExecutionWitness {
            nodes,
            codes,
            block_headers_bytes,
            pre_state_root,
            first_block_number: first.number,
            chain_config: self.storage.get_chain_config(),
        })
    }

    /// Every header the witness has to carry: the first block's parent, each
    /// block's parent in the batch, and any older ancestor a `BLOCKHASH`
    /// reached.
    ///
    /// Walked by hash from the last block backwards, never by number, so a
    /// batch on a non-canonical branch collects that branch's ancestors.
    fn witness_headers(
        &self,
        last: &BlockHeader,
        blockhash_references: &HashMap<u64, H256>,
        parent: &BlockHeader,
    ) -> Result<Vec<Vec<u8>>, ChainError> {
        let oldest_needed = blockhash_references
            .keys()
            .min()
            .copied()
            .map(|number| number.min(parent.number))
            .unwrap_or(parent.number);

        let mut headers = Vec::new();
        let mut current = last.clone();
        while current.number > oldest_needed {
            let parent_hash = current.parent_hash;
            current = self
                .storage
                .get_block_header_by_hash(parent_hash)
                .map_err(ChainError::StoreError)?
                .ok_or_else(|| {
                    ChainError::WitnessGeneration(format!(
                        "missing header for block {}",
                        current.number - 1
                    ))
                })?;
            headers.push(current.encode_to_vec());
        }
        Ok(headers)
    }
}

fn witness_read_error(error: pbt_state::PbtStateError) -> ChainError {
    ChainError::WitnessGeneration(format!("binary witness replay failed: {error}"))
}

// ---------------------------------------------------------------------------
// Consuming
// ---------------------------------------------------------------------------

/// A [`VmDatabase`] over a witness's pre-state and nothing else.
///
/// No store, no disk, no network. Every account and slot is resolved out of the
/// binary trie the witness indexes, every bytecode out of the witness's `codes`,
/// and every block hash out of its `headers` — so a read the witness did not
/// cover fails rather than being answered from somewhere else.
///
/// Mirrors `StoreVmDatabase`'s binary path exactly, including the
/// `storage_root`/`has_storage` split: the binary trie has no per-account
/// storage root, so [`AccountState::storage_root`] is [`EMPTY_TRIE_HASH`] for
/// every account here and the real answer travels on
/// [`VmDatabase::has_storage`].
#[derive(Clone)]
pub struct BinaryWitnessVmDatabase {
    /// Behind a mutex because [`BinaryTrie`] reads take `&mut self` (a read
    /// installs the nodes it resolves) while [`VmDatabase`] is `&self` and
    /// `Sync`.
    trie: Arc<Mutex<BinaryTrie>>,
    codes: Arc<BTreeMap<H256, Code>>,
    block_hashes: Arc<BTreeMap<u64, H256>>,
    chain_config: ChainConfig,
}

impl BinaryWitnessVmDatabase {
    pub fn new(
        trie: BinaryTrie,
        codes: BTreeMap<H256, Code>,
        block_hashes: BTreeMap<u64, H256>,
        chain_config: ChainConfig,
    ) -> Self {
        Self {
            trie: Arc::new(Mutex::new(trie)),
            codes: Arc::new(codes),
            block_hashes: Arc::new(block_hashes),
            chain_config,
        }
    }

    /// A handle on the trie, shared with this database rather than copied.
    ///
    /// Shared rather than consumed because the VM holds a clone of the database
    /// for as long as it lives: a caller that has to apply the block's updates
    /// and read the resulting root would otherwise have to prove the VM is gone
    /// first, which is an ownership dance and not a safety property — the trie
    /// is behind a mutex either way.
    pub fn trie_handle(&self) -> Arc<Mutex<BinaryTrie>> {
        Arc::clone(&self.trie)
    }

    fn locked(&self) -> Result<std::sync::MutexGuard<'_, BinaryTrie>, EvmError> {
        self.trie
            .lock()
            .map_err(|_| EvmError::DB("witness trie mutex poisoned".to_string()))
    }
}

impl VmDatabase for BinaryWitnessVmDatabase {
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError> {
        let mut trie = self.locked()?;
        let account = pbt_state::get_account_info(&mut trie, address)
            .map_err(|error| EvmError::DB(error.to_string()))?;
        Ok(account.map(|info| AccountState {
            nonce: info.nonce,
            balance: info.balance,
            // No such value in this trie; see the type docs.
            storage_root: *EMPTY_TRIE_HASH,
            code_hash: info.code_hash,
        }))
    }

    fn has_storage(&self, address: Address) -> Result<bool, EvmError> {
        let mut trie = self.locked()?;
        pbt_state::has_storage(&mut trie, address).map_err(|error| EvmError::DB(error.to_string()))
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        let mut trie = self.locked()?;
        pbt_state::get_storage_slot(&mut trie, address, &key)
            .map_err(|error| EvmError::DB(error.to_string()))
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        self.block_hashes
            .get(&block_number)
            .copied()
            .ok_or_else(|| {
                EvmError::DB(format!(
                    "the witness carries no header for block {block_number}"
                ))
            })
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        Ok(self.chain_config)
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, EvmError> {
        if code_hash == *EMPTY_KECCAK_HASH {
            return Ok(Code::default());
        }
        self.codes
            .get(&code_hash)
            .cloned()
            .ok_or_else(|| EvmError::DB(format!("the witness carries no code for {code_hash:#x}")))
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, EvmError> {
        if code_hash == *EMPTY_KECCAK_HASH {
            return Ok(CodeMetadata { length: 0 });
        }
        Ok(CodeMetadata {
            length: self.get_account_code(code_hash)?.code().len() as u64,
        })
    }
}

/// Why a witness did not verify.
#[derive(Debug, thiserror::Error)]
pub enum BinaryWitnessVerifyError {
    /// The witness is not a witness for its own `preStateRoot`, or is not in
    /// this format at all.
    #[error(transparent)]
    Witness(#[from] BinaryWitnessError),
    /// A header in the witness does not decode.
    #[error("witness header does not decode: {0}")]
    Header(#[from] RLPDecodeError),
    /// Re-execution failed — typically because the witness does not cover a
    /// read the block makes.
    #[error("re-execution against the witness failed: {0}")]
    Execution(String),
    /// The updates could not be applied to the witness's trie, which is what a
    /// missing node on a *write* path looks like.
    #[error("applying the block's updates to the witness failed: {0}")]
    Apply(String),
}

/// Re-execute `blocks` against `witness` alone and return the resulting binary
/// state root.
///
/// **No store, no disk.** Everything comes out of the witness: the pre-state
/// trie, the bytecodes, the ancestor hashes. The only other input is the chain
/// configuration, which the MPT path takes the same way — `RpcExecutionWitness`
/// does not carry one either, and `into_execution_witness` is handed it — and
/// which is not state.
///
/// The caller compares the result with the last block's `header.state_root`.
/// This deliberately does *not* do that itself: a verifier that both computes
/// the answer and grades it can only ever report "ok", and the failures worth
/// catching are the ones where a wrong root comes back looking fine.
pub fn recompute_post_state_root(
    witness: &RpcBinaryExecutionWitness,
    blocks: &[Block],
    chain_config: ChainConfig,
) -> Result<H256, BinaryWitnessVerifyError> {
    let mut trie = witness.into_pre_state_trie()?;

    let mut block_hashes = BTreeMap::new();
    for encoded in &witness.headers {
        let header = BlockHeader::decode(encoded)?;
        block_hashes.insert(header.number, header.compute_block_hash(&NativeCrypto));
    }

    let mut codes = BTreeMap::new();
    for code in &witness.codes {
        let code = Code::from_bytecode(code.clone(), &NativeCrypto);
        codes.insert(code.hash, code);
    }

    for block in blocks {
        let db =
            BinaryWitnessVmDatabase::new(trie, codes.clone(), block_hashes.clone(), chain_config);
        let handle = db.trie_handle();
        // Scoped so the VM — which holds its own clone of the database, and so
        // of the trie handle — is dropped before the handle is unwrapped.
        let updates = {
            let mut vm = ethrex_vm::Evm::new_from_db_for_l1(
                Arc::new(Box::new(db) as DynVmDatabase),
                Arc::new(NativeCrypto),
            );
            vm.execute_block(block)
                .map_err(|error| BinaryWitnessVerifyError::Execution(error.to_string()))?;
            vm.get_state_transitions()
                .map_err(|error| BinaryWitnessVerifyError::Execution(error.to_string()))?
        };
        {
            let mut locked = handle
                .lock()
                .map_err(|_| BinaryWitnessVerifyError::Apply("mutex poisoned".to_string()))?;
            pbt_state::apply_account_updates(&mut locked, &updates)
                .map_err(|error| BinaryWitnessVerifyError::Apply(error.to_string()))?;
        }
        trie = Arc::try_unwrap(handle)
            .map_err(|_| {
                BinaryWitnessVerifyError::Apply("witness trie is still shared".to_string())
            })?
            .into_inner()
            .map_err(|_| BinaryWitnessVerifyError::Apply("mutex poisoned".to_string()))?;
    }
    Ok(trie.root())
}
