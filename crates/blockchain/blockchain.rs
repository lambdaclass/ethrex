pub mod constants;
pub mod error;
pub mod fork_choice;
pub mod mempool;
pub mod payload;
mod smoke_test;
pub mod tracing;
pub mod vm;

use ::tracing::{debug, info, instrument, trace};
use constants::{MAX_INITCODE_SIZE, MAX_TRANSACTION_DATA_SIZE, POST_OSAKA_GAS_LIMIT_CAP};
use error::MempoolError;
use error::{ChainError, InvalidBlockError};
use ethrex_common::constants::{
    EMPTY_TRIE_HASH, GAS_PER_BLOB, MAX_RLP_BLOCK_SIZE, MIN_BASE_FEE_PER_BLOB_GAS,
};
use ethrex_common::types::block_execution_witness::ExecutionWitness;
use ethrex_common::types::fee_config::FeeConfig;
use ethrex_common::types::requests::{EncodedRequests, Requests, compute_requests_hash};
use ethrex_common::types::{
    AccountState, AccountUpdate, Block, BlockHash, BlockHeader, BlockNumber, ChainConfig, Code,
    EIP4844Transaction, Receipt, Transaction, WrappedEIP4844Transaction, compute_receipts_root,
    validate_block_header, validate_cancun_header_fields, validate_prague_header_fields,
    validate_pre_cancun_header_fields,
};
use ethrex_common::types::{ELASTICITY_MULTIPLIER, P2PTransaction};
use ethrex_common::types::{Fork, MempoolTransaction};
use ethrex_common::utils::keccak;
use ethrex_common::{Address, H160, H256, TrieLogger};
use ethrex_metrics::metrics;
use ethrex_rlp::constants::RLP_NULL;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{
    AccountUpdatesList, Store, UpdateBatch, error::StoreError, hash_address, hash_key,
};
use ethrex_trie::node::{BranchNode, ExtensionNode};
use ethrex_trie::{Nibbles, Node, NodeRef, Trie};
use ethrex_vm::backends::levm::db::DatabaseLogger;
use ethrex_vm::{BlockExecutionResult, DynVmDatabase, Evm, EvmError};
use mempool::Mempool;
use payload::PayloadOrTask;
use rustc_hash::FxHashMap;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::sync::{
    Arc, Mutex, RwLock,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{Receiver, channel},
};
use std::time::Instant;
use tokio::sync::Mutex as TokioMutex;
use tokio_util::sync::CancellationToken;

use vm::StoreVmDatabase;

#[cfg(feature = "metrics")]
use ethrex_metrics::blocks::METRICS_BLOCKS;

#[cfg(feature = "c-kzg")]
use ethrex_common::types::BlobsBundle;

const MAX_PAYLOADS: usize = 10;
const MAX_MEMPOOL_SIZE_DEFAULT: usize = 10_000;

type StoreUpdatesMap = FxHashMap<H256, (Result<Trie, StoreError>, FxHashMap<Nibbles, Vec<u8>>)>;
//TODO: Implement a struct Chain or BlockChain to encapsulate
//functionality and canonical chain state and config

#[derive(Debug, Clone, Default)]
pub enum BlockchainType {
    #[default]
    L1,
    L2(L2Config),
}

#[derive(Debug, Clone, Default)]
pub struct L2Config {
    /// We use a RwLock because the Watcher updates the L1 fee config periodically
    pub fee_config: Arc<RwLock<FeeConfig>>,
}

#[derive(Debug)]
pub struct Blockchain {
    storage: Store,
    pub mempool: Mempool,
    /// Whether the node's chain is in or out of sync with the current chain
    /// This will be set to true once the initial sync has taken place and wont be set to false after
    /// This does not reflect whether there is an ongoing sync process
    is_synced: AtomicBool,
    pub options: BlockchainOptions,
    /// Mapping from a payload id to either a complete payload or a payload build task
    /// We need to keep completed payloads around in case consensus requests them twice
    pub payloads: Arc<TokioMutex<Vec<(u64, PayloadOrTask)>>>,
}

#[derive(Debug, Clone)]
pub struct BlockchainOptions {
    pub max_mempool_size: usize,
    /// Whether performance logs should be emitted
    pub perf_logs_enabled: bool,
    pub r#type: BlockchainType,
}

impl Default for BlockchainOptions {
    fn default() -> Self {
        Self {
            max_mempool_size: MAX_MEMPOOL_SIZE_DEFAULT,
            perf_logs_enabled: false,
            r#type: BlockchainType::default(),
        }
    }
}

struct PartialMerkleizationResults {
    state_updates: FxHashMap<Nibbles, Vec<u8>>,
    storage_updates: StoreUpdatesMap,
    code_updates: FxHashMap<H256, Code>,
}

#[derive(Debug, Clone)]
pub struct BatchBlockProcessingFailure {
    pub last_valid_hash: H256,
    pub failed_block_hash: H256,
}

fn log_batch_progress(batch_size: u32, current_block: u32) {
    let progress_needed = batch_size > 10;
    const PERCENT_MARKS: [u32; 4] = [20, 40, 60, 80];
    if progress_needed {
        PERCENT_MARKS.iter().for_each(|mark| {
            if (batch_size * mark) / 100 == current_block {
                info!("[SYNCING] {mark}% of batch processed");
            }
        });
    }
}

impl Blockchain {
    pub fn new(store: Store, blockchain_opts: BlockchainOptions) -> Self {
        Self {
            storage: store,
            mempool: Mempool::new(blockchain_opts.max_mempool_size),
            is_synced: AtomicBool::new(false),
            payloads: Arc::new(TokioMutex::new(Vec::new())),
            options: blockchain_opts,
        }
    }

    pub fn default_with_store(store: Store) -> Self {
        Self {
            storage: store,
            mempool: Mempool::new(MAX_MEMPOOL_SIZE_DEFAULT),
            is_synced: AtomicBool::new(false),
            payloads: Arc::new(TokioMutex::new(Vec::new())),
            options: BlockchainOptions::default(),
        }
    }

    /// Executes a block withing a new vm instance and state
    fn execute_block(
        &self,
        block: &Block,
    ) -> Result<(BlockExecutionResult, Vec<AccountUpdate>), ChainError> {
        // Validate if it can be the new head and find the parent
        let Ok(parent_header) = find_parent_header(&block.header, &self.storage) else {
            // If the parent is not present, we store it as pending.
            self.storage.add_pending_block(block.clone())?;
            return Err(ChainError::ParentNotFound);
        };

        let chain_config = self.storage.get_chain_config();

        // Validate the block pre-execution
        validate_block(block, &parent_header, &chain_config, ELASTICITY_MULTIPLIER)?;

        let vm_db = StoreVmDatabase::new(self.storage.clone(), parent_header)?;
        let mut vm = self.new_evm(vm_db)?;

        let execution_result = vm.execute_block(block)?;
        let account_updates = vm.get_state_transitions()?;

        // Validate execution went alright
        validate_gas_used(&execution_result.receipts, &block.header)?;
        validate_receipts_root(&block.header, &execution_result.receipts)?;
        validate_requests_hash(&block.header, &chain_config, &execution_result.requests)?;

        Ok((execution_result, account_updates))
    }

    /// Executes a block withing a new vm instance and state
    #[instrument(
        level = "trace",
        name = "Execute Block",
        skip_all,
        fields(namespace = "block_execution")
    )]
    fn execute_block_pipeline(
        &self,
        block: &Block,
        parent_header: &BlockHeader,
        mut vm: Evm,
    ) -> Result<
        (
            BlockExecutionResult,
            AccountUpdatesList,
            // FIXME: extract to stats struct
            usize,
            [Instant; 6],
        ),
        ChainError,
    > {
        let start_instant = Instant::now();

        let chain_config = self.storage.get_chain_config();

        // Validate the block pre-execution
        validate_block(block, parent_header, &chain_config, ELASTICITY_MULTIPLIER)?;
        let block_validated_instant = Instant::now();

        let exec_merkle_start = Instant::now();
        let queue_length = AtomicUsize::new(0);
        let queue_length_ref = &queue_length;
        let mut max_queue_length = 0;
        let (execution_result, account_updates_list) = std::thread::scope(|s| {
            let max_queue_length_ref = &mut max_queue_length;
            let (tx, rx) = channel();
            let execution_handle = std::thread::Builder::new()
                .name("block_executor_execution".to_string())
                .spawn_scoped(s, move || -> Result<_, ChainError> {
                    let execution_result =
                        vm.execute_block_pipeline(block, tx, queue_length_ref)?;

                    // Validate execution went alright
                    validate_gas_used(&execution_result.receipts, &block.header)?;
                    validate_receipts_root(&block.header, &execution_result.receipts)?;
                    validate_requests_hash(
                        &block.header,
                        &chain_config,
                        &execution_result.requests,
                    )?;

                    let exec_end_instant = Instant::now();
                    Ok((execution_result, exec_end_instant))
                })
                .expect("Failed to spawn block_executor exec thread");
            let parent_header_ref = &parent_header; // Avoid moving to thread
            let merkleize_handle = std::thread::Builder::new()
                .name("block_executor_merkleizer".to_string())
                .spawn_scoped(s, move || -> Result<_, StoreError> {
                    let account_updates_list = self.handle_merkleization(
                        s,
                        rx,
                        parent_header_ref,
                        queue_length_ref,
                        max_queue_length_ref,
                    )?;
                    let merkle_end_instant = Instant::now();
                    Ok((account_updates_list, merkle_end_instant))
                })
                .expect("Failed to spawn block_executor merkleizer thread");
            (
                execution_handle.join().unwrap_or_else(|_| {
                    Err(ChainError::Custom("execution thread panicked".to_string()))
                }),
                merkleize_handle.join().unwrap_or_else(|_| {
                    Err(StoreError::Custom(
                        "merklization thread panicked".to_string(),
                    ))
                }),
            )
        });
        let (account_updates_list, merkle_end_instant) = account_updates_list?;
        let (execution_result, exec_end_instant) = execution_result?;
        let exec_merkle_end_instant = Instant::now();

        Ok((
            execution_result,
            account_updates_list,
            max_queue_length,
            [
                start_instant,
                block_validated_instant,
                exec_merkle_start,
                exec_end_instant,
                merkle_end_instant,
                exec_merkle_end_instant,
            ],
        ))
    }

    fn handle_merkleization_subtrie(
        &self,
        rx: Receiver<Vec<(H256, AccountUpdate)>>,
        parent_header: &BlockHeader,
    ) -> Result<PartialMerkleizationResults, StoreError> {
        let mut state_trie = self
            .storage
            .state_trie(parent_header.hash())?
            .ok_or(StoreError::MissingStore)?;
        let mut state_updates_map: FxHashMap<Nibbles, Vec<u8>> = Default::default();
        let mut storage_updates_map: StoreUpdatesMap = Default::default();
        let mut code_updates: FxHashMap<H256, Code> = Default::default();
        let mut account_states: FxHashMap<H256, AccountState> = Default::default();
        for updates in rx {
            Self::process_incoming_update_message(
                &self.storage,
                &mut state_trie,
                updates,
                &mut storage_updates_map,
                parent_header,
                &mut state_updates_map,
                &mut code_updates,
                &mut account_states,
            )?;
        }

        Ok(PartialMerkleizationResults {
            state_updates: state_updates_map,
            storage_updates: storage_updates_map,
            code_updates,
        })
    }

    #[instrument(
        level = "trace",
        name = "Trie update",
        skip_all,
        fields(namespace = "block_execution")
    )]
    fn handle_merkleization<'a, 's, 'b>(
        &'a self,
        scope: &'s std::thread::Scope<'s, '_>,
        rx: Receiver<Vec<AccountUpdate>>,
        parent_header: &'b BlockHeader,
        queue_length: &AtomicUsize,
        max_queue_length: &mut usize,
    ) -> Result<AccountUpdatesList, StoreError>
    where
        'a: 's,
        'b: 's,
    {
        // Fetch the old root from the DB and decode it
        let old_root_opt = self
            .storage
            .state_trie(parent_header.hash())?
            .ok_or(StoreError::MissingStore)?
            .db()
            .get(Nibbles::default())?
            .map(|v| Node::decode(&v))
            .transpose()?;

        // If there's no root, or it's not a branch node, we fallback to sequential processing.
        let Some(Node::Branch(old_root)) = old_root_opt else {
            return self.handle_merkleization_sequential(
                rx,
                parent_header,
                queue_length,
                max_queue_length,
            );
        };
        // If there are less than 3 subtries, we fallback to sequential processing.
        // This simplifies the handling of shard results.
        if old_root.choices.iter().filter(|c| c.is_valid()).count() < 3 {
            return self.handle_merkleization_sequential(
                rx,
                parent_header,
                queue_length,
                max_queue_length,
            );
        }
        let mut workers_tx = Vec::with_capacity(16);
        let mut workers_handles = Vec::with_capacity(16);
        for i in 0..16 {
            let (tx, rx) = channel();
            let handle = std::thread::Builder::new()
                .name(format!("block_executor_merkleization_shard_worker_{i}"))
                .spawn_scoped(scope, move || {
                    self.handle_merkleization_subtrie(rx, parent_header)
                })
                .map_err(|e| StoreError::Custom(format!("spawn failed: {e:?}",)))?;
            workers_handles.push(handle);
            workers_tx.push(tx);
        }
        let mut state_updates_map: FxHashMap<Nibbles, Vec<u8>> = Default::default();
        let mut storage_updates_map: StoreUpdatesMap = Default::default();
        let mut code_updates: FxHashMap<H256, Code> = Default::default();
        let mut hashed_address_cache: FxHashMap<H160, H256> = Default::default();
        for updates in rx {
            let current_length = queue_length.fetch_sub(1, Ordering::Acquire);
            *max_queue_length = current_length.max(*max_queue_length);
            let mut hashed_updates: Vec<_> = updates
                .into_iter()
                .map(|u| {
                    let hashed_address = hashed_address_cache
                        .entry(u.address)
                        .or_insert_with(|| keccak(u.address));
                    (*hashed_address, u)
                })
                .collect();
            hashed_updates.sort_by_key(|(h, _)| h.0[0]);
            for sharded_update in hashed_updates.chunk_by(|l, r| l.0.0[0] & 0xf0 == r.0.0[0] & 0xf0)
            {
                let shard_message = sharded_update.to_vec();
                workers_tx[(shard_message[0].0.0[0] >> 4) as usize]
                    .send(shard_message)
                    .map_err(|e| StoreError::Custom(format!("send failed: {e}")))?;
            }
        }
        drop(workers_tx);
        let mut real_root = old_root;
        for (choice, worker) in workers_handles.into_iter().enumerate() {
            let worker_result = worker
                .join()
                .map_err(|e| StoreError::Custom(format!("join failed: {e:?}",)))??;
            let Some(root_node) = worker_result.state_updates.get(&Nibbles::default()) else {
                continue;
            };
            let root_node = Node::decode(root_node)?;
            let Node::Branch(mut subtrie_branch) = root_node else {
                unreachable!("the result can only remove one of the >2 subtries we had")
            };
            real_root.choices[choice] = std::mem::take(&mut subtrie_branch.choices[choice]);

            code_updates.extend(worker_result.code_updates);
            storage_updates_map.extend(worker_result.storage_updates);
            state_updates_map.extend(worker_result.state_updates);
        }

        // Turn the root back into an extension or leaf if applicable
        let root_node_opt = {
            let children = real_root
                .choices
                .iter()
                .filter(|child| child.is_valid())
                // No need to check all of them
                .take(2)
                .count();

            match children {
                0 | 1 => collapse_root_node(
                    real_root,
                    &mut state_updates_map,
                    &self.storage,
                    parent_header,
                )?,
                // More than one child. Keep as branch
                _ => Some(Node::Branch(real_root)),
            }
        };

        let (state_trie_hash, root_node) = if let Some(root_node) = &root_node_opt {
            let encoded_root_node = root_node.encode_to_vec();
            (keccak(&encoded_root_node), encoded_root_node)
        } else {
            (*EMPTY_TRIE_HASH, vec![RLP_NULL])
        };
        state_updates_map.insert(Nibbles::default(), root_node);
        let state_updates = state_updates_map.into_iter().collect();
        let storage_updates = storage_updates_map
            .into_iter()
            .map(|(a, (_, s))| (a, s.into_iter().collect()))
            .collect();
        let code_updates = code_updates.into_iter().collect();

        Ok(AccountUpdatesList {
            state_trie_hash,
            state_updates,
            storage_updates,
            code_updates,
        })
    }

    fn handle_merkleization_sequential(
        &self,
        rx: Receiver<Vec<AccountUpdate>>,
        parent_header: &BlockHeader,
        queue_length: &AtomicUsize,
        max_queue_length: &mut usize,
    ) -> Result<AccountUpdatesList, StoreError> {
        let mut state_trie = self
            .storage
            .state_trie(parent_header.hash())?
            .ok_or(StoreError::MissingStore)?;
        let mut state_trie_hash = H256::default();
        let mut state_updates_map: FxHashMap<Nibbles, Vec<u8>> = Default::default();
        let mut storage_updates_map: StoreUpdatesMap = Default::default();
        let mut code_updates: FxHashMap<H256, Code> = Default::default();
        let mut account_states: FxHashMap<H256, AccountState> = Default::default();

        let mut hashed_address_cache: FxHashMap<H160, H256> = Default::default();

        for updates in rx {
            let current_length = queue_length.fetch_sub(1, Ordering::Acquire);
            *max_queue_length = current_length.max(*max_queue_length);
            let hashed_updates: Vec<_> = updates
                .into_iter()
                .map(|u| {
                    let hashed_address = hashed_address_cache
                        .entry(u.address)
                        .or_insert_with(|| keccak(u.address));
                    (*hashed_address, u)
                })
                .collect();
            state_trie_hash = Self::process_incoming_update_message(
                &self.storage,
                &mut state_trie,
                hashed_updates,
                &mut storage_updates_map,
                parent_header,
                &mut state_updates_map,
                &mut code_updates,
                &mut account_states,
            )?;
        }
        let state_updates = state_updates_map.into_iter().collect();
        let storage_updates = storage_updates_map
            .into_iter()
            .map(|(a, (_, s))| (a, s.into_iter().collect()))
            .collect();
        let code_updates = code_updates.into_iter().collect();

        Ok(AccountUpdatesList {
            state_trie_hash,
            state_updates,
            storage_updates,
            code_updates,
        })
    }

    /// Processes a batch of account updates, applying them to the state trie and storage tries,
    /// and returns the new state root.
    #[allow(clippy::too_many_arguments)]
    fn process_incoming_update_message(
        storage: &Store,
        state_trie: &mut Trie,
        updates: Vec<(H256, AccountUpdate)>,
        storage_updates_map: &mut StoreUpdatesMap,
        parent_header: &BlockHeader,
        state_updates_map: &mut FxHashMap<Nibbles, Vec<u8>>,
        code_updates: &mut FxHashMap<H256, Code>,
        account_states: &mut FxHashMap<H256, AccountState>,
    ) -> Result<H256, StoreError> {
        trace!("Execute block pipeline: Received {} updates", updates.len());
        // Apply the account updates over the last block's state and compute the new state root
        for (hashed_address_h256, update) in updates {
            let hashed_address = hashed_address_h256.0.to_vec();
            // Use H256 directly (implements Debug) to avoid hex::encode allocation
            trace!(account = ?hashed_address_h256, "Execute block pipeline: Update cycle");
            if update.removed {
                // Remove account from trie
                state_trie.remove(&hashed_address)?;
                account_states.remove(&hashed_address_h256);
                continue;
            }
            // Add or update AccountState in the trie
            // Fetch current state or create a new state to be inserted
            let account_state = match account_states.entry(hashed_address_h256) {
                Entry::Occupied(occupied_entry) => {
                    trace!(account = ?hashed_address_h256, "Found account state in cache");
                    occupied_entry.into_mut()
                }
                Entry::Vacant(vacant_entry) => {
                    let account_state = match state_trie.get(&hashed_address)? {
                        Some(encoded_state) => {
                            trace!(account = ?hashed_address_h256, "Found account state in trie");
                            AccountState::decode(&encoded_state)?
                        }
                        None => {
                            trace!(account = ?hashed_address_h256, "Created account state in trie");
                            AccountState::default()
                        }
                    };
                    vacant_entry.insert(account_state)
                }
            };
            if update.removed_storage {
                account_state.storage_root = *EMPTY_TRIE_HASH;
                storage_updates_map.remove(&hashed_address_h256);
            }
            if let Some(info) = &update.info {
                // Use Debug formatting (?info) to avoid allocating hex strings
                trace!(
                    nonce = info.nonce,
                    balance = ?info.balance,
                    code_hash = ?info.code_hash,
                    "With info"
                );
                account_state.nonce = info.nonce;
                account_state.balance = info.balance;
                account_state.code_hash = info.code_hash;
                // Store updated code in DB
                if let Some(code) = &update.code {
                    trace!("Updated code");
                    code_updates.insert(info.code_hash, code.clone());
                }
            }
            // Store the added storage in the account's storage trie and compute its new root
            if !update.added_storage.is_empty() {
                trace!(count = update.added_storage.len(), "Update storages");
                let (storage_trie, storage_updates_map) = storage_updates_map
                    .entry(hashed_address_h256)
                    .or_insert_with(|| {
                        (
                            storage.open_storage_trie(
                                hashed_address_h256,
                                parent_header.state_root,
                                account_state.storage_root,
                            ),
                            Default::default(),
                        )
                    });
                let Ok(storage_trie) = storage_trie else {
                    debug!(account = ?hashed_address_h256, "Failed to open storage trie");
                    return Err(StoreError::Custom("Error opening storage trie".to_string()));
                };
                for (storage_key, storage_value) in &update.added_storage {
                    let hashed_key = hash_key(storage_key);
                    if storage_value.is_zero() {
                        trace!(slot = ?H256::from_slice(&hashed_key), "Removing");
                        storage_trie.remove(&hashed_key)?;
                    } else {
                        trace!(slot = ?H256::from_slice(&hashed_key), "Inserting");
                        storage_trie.insert(hashed_key, storage_value.encode_to_vec())?;
                    }
                }
                trace!(account = ?hashed_address_h256, "Collecting storage changes");
                let (storage_hash, storage_updates) =
                    storage_trie.collect_changes_since_last_hash();
                trace!(account = ?hashed_address_h256, "Storage changes collected");
                storage_updates_map.extend(storage_updates);
                account_state.storage_root = storage_hash;
            }
            state_trie.insert(hashed_address, account_state.encode_to_vec())?;
        }
        let (state_trie_hash, state_updates) = state_trie.collect_changes_since_last_hash();
        state_updates_map.extend(state_updates);
        Ok(state_trie_hash)
    }

    /// Executes a block from a given vm instance an does not clear its state
    fn execute_block_from_state(
        &self,
        parent_header: &BlockHeader,
        block: &Block,
        chain_config: &ChainConfig,
        vm: &mut Evm,
    ) -> Result<BlockExecutionResult, ChainError> {
        // Validate the block pre-execution
        validate_block(block, parent_header, chain_config, ELASTICITY_MULTIPLIER)?;
        let execution_result = vm.execute_block(block)?;
        // Validate execution went alright
        validate_gas_used(&execution_result.receipts, &block.header)?;
        validate_receipts_root(&block.header, &execution_result.receipts)?;
        validate_requests_hash(&block.header, chain_config, &execution_result.requests)?;

        Ok(execution_result)
    }

    pub async fn generate_witness_for_blocks(
        &self,
        blocks: &[Block],
    ) -> Result<ExecutionWitness, ChainError> {
        self.generate_witness_for_blocks_with_fee_configs(blocks, None)
            .await
    }

    pub async fn generate_witness_for_blocks_with_fee_configs(
        &self,
        blocks: &[Block],
        fee_configs: Option<&[FeeConfig]>,
    ) -> Result<ExecutionWitness, ChainError> {
        let first_block_header = &blocks
            .first()
            .ok_or(ChainError::WitnessGeneration(
                "Empty block batch".to_string(),
            ))?
            .header;

        // Get state at previous block
        let trie = self
            .storage
            .state_trie(first_block_header.parent_hash)
            .map_err(|_| ChainError::ParentStateNotFound)?
            .ok_or(ChainError::ParentStateNotFound)?;
        let initial_state_root = trie.hash_no_commit();

        let (mut current_trie_witness, mut trie) = TrieLogger::open_trie(trie);

        // For each block, a new TrieLogger will be opened, each containing the
        // witness accessed during the block execution. We need to accumulate
        // all the nodes accessed during the entire batch execution.
        let mut accumulated_state_trie_witness = current_trie_witness
            .lock()
            .map_err(|_| {
                ChainError::WitnessGeneration("Failed to lock state trie witness".to_string())
            })?
            .clone();

        let mut touched_account_storage_slots = BTreeMap::new();
        // This will become the state trie + storage trie
        let mut used_trie_nodes = Vec::new();

        // Store the root node in case the block is empty and the witness does not record any nodes
        let root_node = trie.root_node().map_err(|_| {
            ChainError::WitnessGeneration("Failed to get root state node".to_string())
        })?;

        let mut blockhash_opcode_references = HashMap::new();
        let mut codes = Vec::new();

        for (i, block) in blocks.iter().enumerate() {
            let parent_hash = block.header.parent_hash;
            let parent_header = self
                .storage
                .get_block_header_by_hash(parent_hash)
                .map_err(ChainError::StoreError)?
                .ok_or(ChainError::ParentNotFound)?;

            // This assumes that the user has the necessary state stored already,
            // so if the user only has the state previous to the first block, it
            // will fail in the second iteration of this for loop. To ensure this,
            // doesn't fail, later in this function we store the new state after
            // re-execution.
            let vm_db: DynVmDatabase =
                Box::new(StoreVmDatabase::new(self.storage.clone(), parent_header)?);

            let logger = Arc::new(DatabaseLogger::new(Arc::new(Mutex::new(Box::new(vm_db)))));

            let mut vm = match self.options.r#type {
                BlockchainType::L1 => Evm::new_from_db_for_l1(logger.clone()),
                BlockchainType::L2(_) => {
                    let l2_config = match fee_configs {
                        Some(fee_configs) => {
                            fee_configs.get(i).ok_or(ChainError::WitnessGeneration(
                                "FeeConfig not found for witness generation".to_string(),
                            ))?
                        }
                        None => Err(ChainError::WitnessGeneration(
                            "L2Config not found for witness generation".to_string(),
                        ))?,
                    };
                    Evm::new_from_db_for_l2(logger.clone(), *l2_config)
                }
            };

            // Re-execute block with logger
            let execution_result = vm.execute_block(block)?;

            // Gather account updates
            let account_updates = vm.get_state_transitions()?;

            for account_update in &account_updates {
                touched_account_storage_slots.insert(
                    account_update.address,
                    account_update
                        .added_storage
                        .keys()
                        .cloned()
                        .collect::<Vec<H256>>(),
                );
            }

            // Get the used block hashes from the logger
            let logger_block_hashes = logger
                .block_hashes_accessed
                .lock()
                .map_err(|_e| {
                    ChainError::WitnessGeneration("Failed to get block hashes".to_string())
                })?
                .clone();

            blockhash_opcode_references.extend(logger_block_hashes);

            // Access all the accounts needed for withdrawals
            if let Some(withdrawals) = block.body.withdrawals.as_ref() {
                for withdrawal in withdrawals {
                    trie.get(&hash_address(&withdrawal.address)).map_err(|_e| {
                        ChainError::Custom("Failed to access account from trie".to_string())
                    })?;
                }
            }

            let mut used_storage_tries = HashMap::new();

            // Access all the accounts from the initial trie
            // Record all the storage nodes for the initial state
            for (account, acc_keys) in logger
                .state_accessed
                .lock()
                .map_err(|_e| {
                    ChainError::WitnessGeneration("Failed to execute with witness".to_string())
                })?
                .iter()
            {
                // Access the account from the state trie to record the nodes used to access it
                trie.get(&hash_address(account)).map_err(|_e| {
                    ChainError::WitnessGeneration("Failed to access account from trie".to_string())
                })?;
                // Get storage trie at before updates
                if !acc_keys.is_empty()
                    && let Ok(Some(storage_trie)) = self.storage.storage_trie(parent_hash, *account)
                {
                    let (storage_trie_witness, storage_trie) = TrieLogger::open_trie(storage_trie);
                    // Access all the keys
                    for storage_key in acc_keys {
                        let hashed_key = hash_key(storage_key);
                        storage_trie.get(&hashed_key).map_err(|_e| {
                            ChainError::WitnessGeneration(
                                "Failed to access storage key".to_string(),
                            )
                        })?;
                    }
                    // Store the tries to reuse when applying account updates
                    used_storage_tries.insert(*account, (storage_trie_witness, storage_trie));
                }
            }

            // Store all the accessed evm bytecodes
            for code_hash in logger
                .code_accessed
                .lock()
                .map_err(|_e| {
                    ChainError::WitnessGeneration("Failed to gather used bytecodes".to_string())
                })?
                .iter()
            {
                let code = self
                    .storage
                    .get_account_code(*code_hash)
                    .map_err(|_e| {
                        ChainError::WitnessGeneration("Failed to get account code".to_string())
                    })?
                    .ok_or(ChainError::WitnessGeneration(
                        "Failed to get account code".to_string(),
                    ))?;
                codes.push(code.bytecode.to_vec());
            }

            // Apply account updates to the trie recording all the necessary nodes to do so
            let (storage_tries_after_update, account_updates_list) = self
                .storage
                .apply_account_updates_from_trie_with_witness(
                    trie,
                    &account_updates,
                    used_storage_tries,
                )
                .await?;

            // We cannot ensure that the users of this function have the necessary
            // state stored, so in order for it to not assume anything, we update
            // the storage with the new state after re-execution
            self.store_block(block.clone(), account_updates_list, execution_result)?;

            for (address, (witness, _storage_trie)) in storage_tries_after_update {
                let mut witness = witness.lock().map_err(|_| {
                    ChainError::WitnessGeneration("Failed to lock storage trie witness".to_string())
                })?;
                let witness = std::mem::take(&mut *witness);
                let witness = witness.into_values().collect::<Vec<_>>();
                used_trie_nodes.extend_from_slice(&witness);
                touched_account_storage_slots.entry(address).or_default();
            }

            let (new_state_trie_witness, updated_trie) = TrieLogger::open_trie(
                self.storage
                    .state_trie(block.header.hash())
                    .map_err(|_| ChainError::ParentStateNotFound)?
                    .ok_or(ChainError::ParentStateNotFound)?,
            );

            // Use the updated state trie for the next block
            trie = updated_trie;

            for state_trie_witness in current_trie_witness
                .lock()
                .map_err(|_| {
                    ChainError::WitnessGeneration("Failed to lock state trie witness".to_string())
                })?
                .iter()
            {
                accumulated_state_trie_witness
                    .insert(*state_trie_witness.0, state_trie_witness.1.clone());
            }

            current_trie_witness = new_state_trie_witness;
        }

        used_trie_nodes.extend_from_slice(&Vec::from_iter(
            accumulated_state_trie_witness.into_values(),
        ));

        // If the witness is empty at least try to store the root
        if used_trie_nodes.is_empty()
            && let Some(root) = root_node
        {
            used_trie_nodes.push((*root).clone());
        }

        // - We now need necessary block headers, these go from the first block referenced (via BLOCKHASH or just the first block to execute) up to the parent of the last block to execute.
        let mut block_headers_bytes = Vec::new();

        let first_blockhash_opcode_number = blockhash_opcode_references.keys().min();
        let first_needed_block_hash = first_blockhash_opcode_number
            .and_then(|n| {
                (*n < first_block_header.number.saturating_sub(1))
                    .then(|| blockhash_opcode_references.get(n))?
                    .copied()
            })
            .unwrap_or(first_block_header.parent_hash);

        // At the beginning this is the header of the last block to execute.
        let mut current_header = blocks
            .last()
            .ok_or_else(|| ChainError::WitnessGeneration("Empty batch".to_string()))?
            .header
            .clone();

        // Headers from latest - 1 until we reach first block header we need.
        // We do it this way because we want to fetch headers by hash, not by number
        while current_header.hash() != first_needed_block_hash {
            let parent_hash = current_header.parent_hash;
            let current_number = current_header.number - 1;

            current_header = self
                .storage
                .get_block_header_by_hash(parent_hash)?
                .ok_or_else(|| {
                    ChainError::WitnessGeneration(format!(
                        "Failed to get block {current_number} header"
                    ))
                })?;

            block_headers_bytes.push(current_header.encode_to_vec());
        }

        // Create a list of all read/write addresses and storage slots
        let mut keys = Vec::new();
        for (address, touched_storage_slots) in touched_account_storage_slots {
            keys.push(address.as_bytes().to_vec());
            for slot in touched_storage_slots.iter() {
                keys.push(slot.as_bytes().to_vec());
            }
        }

        // Get initial state trie root and embed the rest of the trie into it
        let nodes: BTreeMap<H256, Node> = used_trie_nodes
            .into_iter()
            .map(|node| (node.compute_hash().finalize(), node))
            .collect();
        let state_trie_root = if let NodeRef::Node(state_trie_root, _) =
            Trie::get_embedded_root(&nodes, initial_state_root)?
        {
            Some((*state_trie_root).clone())
        } else {
            None
        };

        // Get all initial storage trie roots and embed the rest of the trie into it
        let state_trie = if let Some(state_trie_root) = &state_trie_root {
            Trie::new_temp_with_root(state_trie_root.clone().into())
        } else {
            Trie::new_temp()
        };
        let mut storage_trie_roots = BTreeMap::new();
        for key in &keys {
            if key.len() != 20 {
                continue; // not an address
            }
            let address = Address::from_slice(key);
            let hashed_address = hash_address(&address);
            let Some(encoded_account) = state_trie.get(&hashed_address)? else {
                continue; // empty account, doesn't have a storage trie
            };
            let storage_root_hash = AccountState::decode(&encoded_account)?.storage_root;
            if storage_root_hash == *EMPTY_TRIE_HASH {
                continue; // empty storage trie
            }
            if !nodes.contains_key(&storage_root_hash) {
                continue; // storage trie isn't relevant to this execution
            }
            let node = Trie::get_embedded_root(&nodes, storage_root_hash)?;
            let NodeRef::Node(node, _) = node else {
                return Err(ChainError::Custom(
                    "execution witness does not contain non-empty storage trie".to_string(),
                ));
            };
            storage_trie_roots.insert(address, (*node).clone());
        }

        Ok(ExecutionWitness {
            codes,
            block_headers_bytes,
            first_block_number: first_block_header.number,
            chain_config: self.storage.get_chain_config(),
            state_trie_root,
            storage_trie_roots,
            keys,
        })
    }

    #[instrument(
        level = "trace",
        name = "Block DB update",
        skip_all,
        fields(namespace = "block_execution")
    )]
    pub fn store_block(
        &self,
        block: Block,
        account_updates_list: AccountUpdatesList,
        execution_result: BlockExecutionResult,
    ) -> Result<(), ChainError> {
        // Check state root matches the one in block header
        validate_state_root(&block.header, account_updates_list.state_trie_hash)?;

        let update_batch = UpdateBatch {
            account_updates: account_updates_list.state_updates,
            storage_updates: account_updates_list.storage_updates,
            receipts: vec![(block.hash(), execution_result.receipts)],
            blocks: vec![block],
            code_updates: account_updates_list.code_updates,
        };

        self.storage
            .store_block_updates(update_batch)
            .map_err(|e| e.into())
    }

    pub fn add_block(&self, block: Block) -> Result<(), ChainError> {
        let since = Instant::now();
        let (res, updates) = self.execute_block(&block)?;
        let executed = Instant::now();

        // Apply the account updates over the last block's state and compute the new state root
        let account_updates_list = self
            .storage
            .apply_account_updates_batch(block.header.parent_hash, &updates)?
            .ok_or(ChainError::ParentStateNotFound)?;

        let (gas_used, gas_limit, block_number, transactions_count) = (
            block.header.gas_used,
            block.header.gas_limit,
            block.header.number,
            block.body.transactions.len(),
        );

        let merkleized = Instant::now();
        let result = self.store_block(block, account_updates_list, res);
        let stored = Instant::now();

        if self.options.perf_logs_enabled {
            Self::print_add_block_logs(
                gas_used,
                gas_limit,
                block_number,
                transactions_count,
                since,
                executed,
                merkleized,
                stored,
            );
        }
        result
    }

    pub fn add_block_pipeline(&self, block: Block) -> Result<(), ChainError> {
        // Validate if it can be the new head and find the parent
        let Ok(parent_header) = find_parent_header(&block.header, &self.storage) else {
            // If the parent is not present, we store it as pending.
            self.storage.add_pending_block(block)?;
            return Err(ChainError::ParentNotFound);
        };

        let vm_db = StoreVmDatabase::new(self.storage.clone(), parent_header.clone())?;
        let vm = self.new_evm(vm_db)?;

        let (res, account_updates_list, merkle_queue_length, instants) =
            self.execute_block_pipeline(&block, &parent_header, vm)?;

        let (gas_used, gas_limit, block_number, transactions_count) = (
            block.header.gas_used,
            block.header.gas_limit,
            block.header.number,
            block.body.transactions.len(),
        );

        let result = self.store_block(block, account_updates_list, res);
        let stored = Instant::now();

        let instants = std::array::from_fn(move |i| {
            if i < instants.len() {
                instants[i]
            } else {
                stored
            }
        });
        if self.options.perf_logs_enabled {
            Self::print_add_block_pipeline_logs(
                gas_used,
                gas_limit,
                block_number,
                transactions_count,
                merkle_queue_length,
                instants,
            );
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn print_add_block_logs(
        gas_used: u64,
        gas_limit: u64,
        block_number: u64,
        transactions_count: usize,
        since: Instant,
        executed: Instant,
        merkleized: Instant,
        stored: Instant,
    ) {
        let interval = stored.duration_since(since).as_millis() as f64;
        if interval != 0f64 {
            let as_gigas = gas_used as f64 / 10_f64.powf(9_f64);
            let throughput = as_gigas / interval * 1000_f64;

            metrics!(
                METRICS_BLOCKS.set_block_number(block_number);
                METRICS_BLOCKS.set_latest_gas_used(gas_used as f64);
                METRICS_BLOCKS.set_latest_block_gas_limit(gas_limit as f64);
                METRICS_BLOCKS.set_latest_gigagas(throughput);
                METRICS_BLOCKS.set_execution_ms(executed.duration_since(since).as_millis() as i64);
                METRICS_BLOCKS.set_merkle_ms(merkleized.duration_since(executed).as_millis() as i64);
                METRICS_BLOCKS.set_store_ms(stored.duration_since(merkleized).as_millis() as i64);
                METRICS_BLOCKS.set_transaction_count(transactions_count as i64);
            );

            let base_log = format!(
                "[METRIC] BLOCK EXECUTION THROUGHPUT ({}): {:.3} Ggas/s TIME SPENT: {:.0} ms. Gas Used: {:.3} ({:.0}%), #Txs: {}.",
                block_number,
                throughput,
                interval,
                as_gigas,
                (gas_used as f64 / gas_limit as f64) * 100.0,
                transactions_count
            );

            fn percentage(init: Instant, end: Instant, total: f64) -> f64 {
                (end.duration_since(init).as_millis() as f64 / total * 100.0).round()
            }
            let extra_log = if as_gigas > 0.0 {
                format!(
                    " exec: {}% merkle: {}% store: {}%",
                    percentage(since, executed, interval),
                    percentage(executed, merkleized, interval),
                    percentage(merkleized, stored, interval)
                )
            } else {
                "".to_string()
            };
            info!("{}{}", base_log, extra_log);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn print_add_block_pipeline_logs(
        gas_used: u64,
        gas_limit: u64,
        block_number: u64,
        transactions_count: usize,
        merkle_queue_length: usize,
        [
            start_instant,
            block_validated_instant,
            exec_merkle_start,
            exec_end_instant,
            _merkle_end_instant,
            exec_merkle_end_instant,
            stored_instant,
        ]: [Instant; 7],
    ) {
        let interval = stored_instant.duration_since(start_instant).as_secs_f64();
        if interval != 0f64 {
            let as_gigas = gas_used as f64 * 1e-9;
            let throughput = as_gigas / interval;

            metrics!(
                METRICS_BLOCKS.set_block_number(block_number);
                METRICS_BLOCKS.set_latest_gas_used(gas_used as f64);
                METRICS_BLOCKS.set_latest_block_gas_limit(gas_limit as f64);
                METRICS_BLOCKS.set_latest_gigagas(throughput);
                METRICS_BLOCKS.set_transaction_count(transactions_count as i64);
            );

            let base_log = format!(
                "[METRIC] BLOCK EXECUTION THROUGHPUT ({}): {:.3} Ggas/s TIME SPENT: {:.0} ms. Gas Used: {:.3} ({:.0}%), #Txs: {}.",
                block_number,
                throughput,
                interval * 1000.0,
                as_gigas,
                (gas_used as f64 / gas_limit as f64) * 100.0,
                transactions_count
            );

            let percentage = move |init: Instant, end: Instant| {
                (end.duration_since(init).as_secs_f64() / interval * 100.0).round()
            };
            let extra_log = if as_gigas > 0.0 {
                format!(
                    " block validation: {}% | exec(w/merkle): {}% | merkle-only: {}% (max_queue_length: {merkle_queue_length}) | store: {}%",
                    percentage(start_instant, block_validated_instant),
                    percentage(exec_merkle_start, exec_end_instant),
                    percentage(exec_end_instant, exec_merkle_end_instant),
                    percentage(exec_merkle_end_instant, stored_instant),
                )
            } else {
                "".to_string()
            };
            info!("{}{}", base_log, extra_log);
        }
    }

    /// Adds multiple blocks in a batch.
    ///
    /// If an error occurs, returns a tuple containing:
    /// - The error type ([`ChainError`]).
    /// - [`BatchProcessingFailure`] (if the error was caused by block processing).
    ///
    /// Note: only the last block's state trie is stored in the db
    pub async fn add_blocks_in_batch(
        &self,
        blocks: Vec<Block>,
        cancellation_token: CancellationToken,
    ) -> Result<(), (ChainError, Option<BatchBlockProcessingFailure>)> {
        let mut last_valid_hash = H256::default();

        let Some(first_block_header) = blocks.first().map(|e| e.header.clone()) else {
            return Err((ChainError::Custom("First block not found".into()), None));
        };

        let chain_config: ChainConfig = self.storage.get_chain_config();

        // Cache block hashes for the full batch so we can access them during execution without having to store the blocks beforehand
        let block_hash_cache = blocks.iter().map(|b| (b.header.number, b.hash())).collect();

        let parent_header = self
            .storage
            .get_block_header_by_hash(first_block_header.parent_hash)
            .map_err(|e| (ChainError::StoreError(e), None))?
            .ok_or((ChainError::ParentNotFound, None))?;
        let vm_db = StoreVmDatabase::new_with_block_hash_cache(
            self.storage.clone(),
            parent_header,
            block_hash_cache,
        )
        .map_err(|e| (ChainError::EvmError(e), None))?;
        let mut vm = self.new_evm(vm_db).map_err(|e| (e.into(), None))?;

        let blocks_len = blocks.len();
        let mut all_receipts: Vec<(BlockHash, Vec<Receipt>)> = Vec::with_capacity(blocks_len);
        let mut total_gas_used = 0;
        let mut transactions_count = 0;

        let interval = Instant::now();
        for (i, block) in blocks.iter().enumerate() {
            if cancellation_token.is_cancelled() {
                info!("Received shutdown signal, aborting");
                return Err((ChainError::Custom(String::from("shutdown signal")), None));
            }
            // for the first block, we need to query the store
            let parent_header = if i == 0 {
                find_parent_header(&block.header, &self.storage).map_err(|err| {
                    (
                        err,
                        Some(BatchBlockProcessingFailure {
                            failed_block_hash: block.hash(),
                            last_valid_hash,
                        }),
                    )
                })?
            } else {
                // for the subsequent ones, the parent is the previous block
                blocks[i - 1].header.clone()
            };

            let BlockExecutionResult { receipts, .. } = self
                .execute_block_from_state(&parent_header, block, &chain_config, &mut vm)
                .map_err(|err| {
                    (
                        err,
                        Some(BatchBlockProcessingFailure {
                            failed_block_hash: block.hash(),
                            last_valid_hash,
                        }),
                    )
                })?;
            debug!("Executed block with hash {}", block.hash());
            last_valid_hash = block.hash();
            total_gas_used += block.header.gas_used;
            transactions_count += block.body.transactions.len();
            all_receipts.push((block.hash(), receipts));

            // Conversion is safe because EXECUTE_BATCH_SIZE=1024
            log_batch_progress(blocks_len as u32, i as u32);
            tokio::task::yield_now().await;
        }

        let account_updates = vm
            .get_state_transitions()
            .map_err(|err| (ChainError::EvmError(err), None))?;

        let last_block = blocks
            .last()
            .ok_or_else(|| (ChainError::Custom("Last block not found".into()), None))?;

        let last_block_number = last_block.header.number;
        let last_block_gas_limit = last_block.header.gas_limit;

        // Apply the account updates over all blocks and compute the new state root
        let account_updates_list = self
            .storage
            .apply_account_updates_batch(first_block_header.parent_hash, &account_updates)
            .map_err(|e| (e.into(), None))?
            .ok_or((ChainError::ParentStateNotFound, None))?;

        let new_state_root = account_updates_list.state_trie_hash;
        let state_updates = account_updates_list.state_updates;
        let accounts_updates = account_updates_list.storage_updates;
        let code_updates = account_updates_list.code_updates;

        // Check state root matches the one in block header
        validate_state_root(&last_block.header, new_state_root).map_err(|e| (e, None))?;

        let update_batch = UpdateBatch {
            account_updates: state_updates,
            storage_updates: accounts_updates,
            blocks,
            receipts: all_receipts,
            code_updates,
        };

        self.storage
            .store_block_updates(update_batch)
            .map_err(|e| (e.into(), None))?;

        let elapsed_seconds = interval.elapsed().as_secs_f64();
        let throughput = if elapsed_seconds > 0.0 && total_gas_used != 0 {
            let as_gigas = (total_gas_used as f64) / 1e9;
            as_gigas / elapsed_seconds
        } else {
            0.0
        };

        metrics!(
            METRICS_BLOCKS.set_block_number(last_block_number);
            METRICS_BLOCKS.set_latest_block_gas_limit(last_block_gas_limit as f64);
            // Set the latest gas used as the average gas used per block in the batch
            METRICS_BLOCKS.set_latest_gas_used(total_gas_used as f64 / blocks_len as f64);
            METRICS_BLOCKS.set_latest_gigagas(throughput);
        );

        if self.options.perf_logs_enabled {
            info!(
                "[METRICS] Executed and stored: Range: {}, Last block num: {}, Last block gas limit: {}, Total transactions: {}, Total Gas: {}, Throughput: {} Gigagas/s",
                blocks_len,
                last_block_number,
                last_block_gas_limit,
                transactions_count,
                total_gas_used,
                throughput
            );
        }

        Ok(())
    }

    /// Add a blob transaction and its blobs bundle to the mempool checking that the transaction is valid
    #[cfg(feature = "c-kzg")]
    pub async fn add_blob_transaction_to_pool(
        &self,
        transaction: EIP4844Transaction,
        blobs_bundle: BlobsBundle,
    ) -> Result<H256, MempoolError> {
        let fork = self.current_fork().await?;

        let transaction = Transaction::EIP4844Transaction(transaction);
        let hash = transaction.hash();
        if self.mempool.contains_tx(hash)? {
            return Ok(hash);
        }

        // Validate blobs bundle after checking if it's already added.
        if let Transaction::EIP4844Transaction(transaction) = &transaction {
            blobs_bundle.validate(transaction, fork)?;
        }

        let sender = transaction.sender()?;

        // Validate transaction
        if let Some(tx_to_replace) = self.validate_transaction(&transaction, sender).await? {
            self.remove_transaction_from_pool(&tx_to_replace)?;
        }

        // Add transaction and blobs bundle to storage
        self.mempool
            .add_transaction(hash, sender, MempoolTransaction::new(transaction, sender))?;
        self.mempool.add_blobs_bundle(hash, blobs_bundle)?;
        Ok(hash)
    }

    /// Add a transaction to the mempool checking that the transaction is valid
    pub async fn add_transaction_to_pool(
        &self,
        transaction: Transaction,
    ) -> Result<H256, MempoolError> {
        // Blob transactions should be submitted via add_blob_transaction along with the corresponding blobs bundle
        if matches!(transaction, Transaction::EIP4844Transaction(_)) {
            return Err(MempoolError::BlobTxNoBlobsBundle);
        }
        let hash = transaction.hash();
        if self.mempool.contains_tx(hash)? {
            return Ok(hash);
        }
        let sender = transaction.sender()?;
        // Validate transaction
        if let Some(tx_to_replace) = self.validate_transaction(&transaction, sender).await? {
            self.remove_transaction_from_pool(&tx_to_replace)?;
        }

        // Add transaction to storage
        self.mempool
            .add_transaction(hash, sender, MempoolTransaction::new(transaction, sender))?;

        Ok(hash)
    }

    /// Remove a transaction from the mempool
    pub fn remove_transaction_from_pool(&self, hash: &H256) -> Result<(), StoreError> {
        self.mempool.remove_transaction(hash)
    }

    /// Remove all transactions in the executed block from the pool (if we have them)
    pub fn remove_block_transactions_from_pool(&self, block: &Block) -> Result<(), StoreError> {
        for tx in &block.body.transactions {
            self.mempool.remove_transaction(&tx.hash())?;
        }
        Ok(())
    }

    /*

    SOME VALIDATIONS THAT WE COULD INCLUDE
    Stateless validations
    1. This transaction is valid on current mempool
        -> Depends on mempool transaction filtering logic
    2. Ensure the maxPriorityFeePerGas is high enough to cover the requirement of the calling pool (the minimum to be included in)
        -> Depends on mempool transaction filtering logic
    3. Transaction's encoded size is smaller than maximum allowed
        -> I think that this is not in the spec, but it may be a good idea
    4. Make sure the transaction is signed properly
    5. Ensure a Blob Transaction comes with its sidecar (Done! - All blob validations have been moved to `common/types/blobs_bundle.rs`):
      1. Validate number of BlobHashes is positive (Done!)
      2. Validate number of BlobHashes is less than the maximum allowed per block,
         which may be computed as `maxBlobGasPerBlock / blobTxBlobGasPerBlob`
      3. Ensure number of BlobHashes is equal to:
        - The number of blobs (Done!)
        - The number of commitments (Done!)
        - The number of proofs (Done!)
      4. Validate that the hashes matches with the commitments, performing a `kzg4844` hash. (Done!)
      5. Verify the blob proofs with the `kzg4844` (Done!)
    Stateful validations
    1. Ensure transaction nonce is higher than the `from` address stored nonce
    2. Certain pools do not allow for nonce gaps. Ensure a gap is not produced (that is, the transaction nonce is exactly the following of the stored one)
    3. Ensure the transactor has enough funds to cover transaction cost:
        - Transaction cost is calculated as `(gas * gasPrice) + (blobGas * blobGasPrice) + value`
    4. In case of transaction reorg, ensure the transactor has enough funds to cover for transaction replacements without overdrafts.
    - This is done by comparing the total spent gas of the transactor from all pooled transactions, and accounting for the necessary gas spenditure if any of those transactions is replaced.
    5. Ensure the transactor is able to add a new transaction. The number of transactions sent by an account may be limited by a certain configured value

    */
    /// Returns the hash of the transaction to replace in case the nonce already exists
    pub async fn validate_transaction(
        &self,
        tx: &Transaction,
        sender: Address,
    ) -> Result<Option<H256>, MempoolError> {
        let nonce = tx.nonce();

        if matches!(tx, &Transaction::PrivilegedL2Transaction(_)) {
            return Ok(None);
        }

        let header_no = self.storage.get_latest_block_number().await?;
        let header = self
            .storage
            .get_block_header(header_no)?
            .ok_or(MempoolError::NoBlockHeaderError)?;
        let config = self.storage.get_chain_config();

        // NOTE: We could add a tx size limit here, but it's not in the actual spec

        // Check init code size
        if config.is_shanghai_activated(header.timestamp)
            && tx.is_contract_creation()
            && tx.data().len() > MAX_INITCODE_SIZE as usize
        {
            return Err(MempoolError::TxMaxInitCodeSizeError);
        }

        if !tx.is_contract_creation() && tx.data().len() >= MAX_TRANSACTION_DATA_SIZE as usize {
            return Err(MempoolError::TxMaxDataSizeError);
        }

        if config.is_osaka_activated(header.timestamp) && tx.gas_limit() > POST_OSAKA_GAS_LIMIT_CAP
        {
            // https://eips.ethereum.org/EIPS/eip-7825
            return Err(MempoolError::TxMaxGasLimitExceededError(
                tx.hash(),
                tx.gas_limit(),
            ));
        }

        // Check gas limit is less than header's gas limit
        if header.gas_limit < tx.gas_limit() {
            return Err(MempoolError::TxGasLimitExceededError);
        }

        // Check priority fee is less or equal than gas fee gap
        if tx.max_priority_fee().unwrap_or(0) > tx.max_fee_per_gas().unwrap_or(0) {
            return Err(MempoolError::TxTipAboveFeeCapError);
        }

        // Check that the gas limit covers the gas needs for transaction metadata.
        if tx.gas_limit() < mempool::transaction_intrinsic_gas(tx, &header, &config)? {
            return Err(MempoolError::TxIntrinsicGasCostAboveLimitError);
        }

        // Check that the specified blob gas fee is above the minimum value
        if let Some(fee) = tx.max_fee_per_blob_gas() {
            // Blob tx fee checks
            if fee < MIN_BASE_FEE_PER_BLOB_GAS.into() {
                return Err(MempoolError::TxBlobBaseFeeTooLowError);
            }
        };

        let maybe_sender_acc_info = self.storage.get_account_info(header_no, sender).await?;

        if let Some(sender_acc_info) = maybe_sender_acc_info {
            if nonce < sender_acc_info.nonce || nonce == u64::MAX {
                return Err(MempoolError::NonceTooLow);
            }

            let tx_cost = tx
                .cost_without_base_fee()
                .ok_or(MempoolError::InvalidTxGasvalues)?;

            if tx_cost > sender_acc_info.balance {
                return Err(MempoolError::NotEnoughBalance);
            }
        } else {
            // An account that is not in the database cannot possibly have enough balance to cover the transaction cost
            return Err(MempoolError::NotEnoughBalance);
        }

        // Check the nonce of pendings TXs in the mempool from the same sender
        // If it exists check if the new tx has higher fees
        let tx_to_replace_hash = self.mempool.find_tx_to_replace(sender, nonce, tx)?;

        if tx
            .chain_id()
            .is_some_and(|chain_id| chain_id != config.chain_id)
        {
            return Err(MempoolError::InvalidChainId(config.chain_id));
        }

        Ok(tx_to_replace_hash)
    }

    /// Marks the node's chain as up to date with the current chain
    /// Once the initial sync has taken place, the node will be considered as sync
    pub fn set_synced(&self) {
        self.is_synced.store(true, Ordering::Relaxed);
    }

    /// Marks the node's chain as not up to date with the current chain.
    /// This will be used when the node is one batch or more behind the current chain.
    pub fn set_not_synced(&self) {
        self.is_synced.store(false, Ordering::Relaxed);
    }

    /// Returns whether the node's chain is up to date with the current chain
    /// This will be true if the initial sync has already taken place and does not reflect whether there is an ongoing sync process
    /// The node should accept incoming p2p transactions if this method returns true
    pub fn is_synced(&self) -> bool {
        self.is_synced.load(Ordering::Relaxed)
    }

    pub fn get_p2p_transaction_by_hash(&self, hash: &H256) -> Result<P2PTransaction, StoreError> {
        let Some(tx) = self.mempool.get_transaction_by_hash(*hash)? else {
            return Err(StoreError::Custom(format!(
                "Hash {hash} not found in the mempool",
            )));
        };
        let result = match tx {
            Transaction::LegacyTransaction(itx) => P2PTransaction::LegacyTransaction(itx),
            Transaction::EIP2930Transaction(itx) => P2PTransaction::EIP2930Transaction(itx),
            Transaction::EIP1559Transaction(itx) => P2PTransaction::EIP1559Transaction(itx),
            Transaction::EIP4844Transaction(itx) => {
                let Some(bundle) = self.mempool.get_blobs_bundle(*hash)? else {
                    return Err(StoreError::Custom(format!(
                        "Blob transaction present without its bundle: hash {hash}",
                    )));
                };

                P2PTransaction::EIP4844TransactionWithBlobs(WrappedEIP4844Transaction {
                    tx: itx,
                    wrapper_version: (bundle.version != 0).then_some(bundle.version),
                    blobs_bundle: bundle,
                })
            }
            Transaction::EIP7702Transaction(itx) => P2PTransaction::EIP7702Transaction(itx),
            // Exclude privileged transactions as they are only created
            // by the lead sequencer. In the future, they might get gossiped
            // like the rest.
            Transaction::PrivilegedL2Transaction(_) => {
                return Err(StoreError::Custom(
                    "Privileged Transactions are not supported in P2P".to_string(),
                ));
            }
            Transaction::FeeTokenTransaction(itx) => P2PTransaction::FeeTokenTransaction(itx),
        };

        Ok(result)
    }

    pub fn new_evm(&self, vm_db: StoreVmDatabase) -> Result<Evm, EvmError> {
        new_evm(&self.options.r#type, vm_db)
    }

    /// Get the current fork of the chain, based on the latest block's timestamp
    pub async fn current_fork(&self) -> Result<Fork, StoreError> {
        let chain_config = self.storage.get_chain_config();
        let latest_block_number = self.storage.get_latest_block_number().await?;
        let latest_block = self
            .storage
            .get_block_header(latest_block_number)?
            .ok_or(StoreError::Custom("Latest block not in DB".to_string()))?;
        Ok(chain_config.fork(latest_block.timestamp))
    }
}

pub fn new_evm(blockchain_type: &BlockchainType, vm_db: StoreVmDatabase) -> Result<Evm, EvmError> {
    let evm = match blockchain_type {
        BlockchainType::L1 => Evm::new_for_l1(vm_db),
        BlockchainType::L2(l2_config) => {
            let fee_config = *l2_config
                .fee_config
                .read()
                .map_err(|_| EvmError::Custom("Fee config lock was poisoned".to_string()))?;

            Evm::new_for_l2(vm_db, fee_config)?
        }
    };
    Ok(evm)
}

pub fn validate_requests_hash(
    header: &BlockHeader,
    chain_config: &ChainConfig,
    requests: &[Requests],
) -> Result<(), ChainError> {
    if !chain_config.is_prague_activated(header.timestamp) {
        return Ok(());
    }

    let encoded_requests: Vec<EncodedRequests> = requests.iter().map(|r| r.encode()).collect();
    let computed_requests_hash = compute_requests_hash(&encoded_requests);
    let valid = header
        .requests_hash
        .map(|requests_hash| requests_hash == computed_requests_hash)
        .unwrap_or(false);

    if !valid {
        return Err(ChainError::InvalidBlock(
            InvalidBlockError::RequestsHashMismatch,
        ));
    }

    Ok(())
}

/// Performs post-execution checks
pub fn validate_state_root(
    block_header: &BlockHeader,
    new_state_root: H256,
) -> Result<(), ChainError> {
    // Compare state root
    if new_state_root == block_header.state_root {
        Ok(())
    } else {
        Err(ChainError::InvalidBlock(
            InvalidBlockError::StateRootMismatch,
        ))
    }
}

pub fn validate_receipts_root(
    block_header: &BlockHeader,
    receipts: &[Receipt],
) -> Result<(), ChainError> {
    let receipts_root = compute_receipts_root(receipts);

    if receipts_root == block_header.receipts_root {
        Ok(())
    } else {
        Err(ChainError::InvalidBlock(
            InvalidBlockError::ReceiptsRootMismatch,
        ))
    }
}

// Returns the hash of the head of the canonical chain (the latest valid hash).
pub async fn latest_canonical_block_hash(storage: &Store) -> Result<H256, ChainError> {
    let latest_block_number = storage.get_latest_block_number().await?;
    if let Some(latest_valid_header) = storage.get_block_header(latest_block_number)? {
        let latest_valid_hash = latest_valid_header.hash();
        return Ok(latest_valid_hash);
    }
    Err(ChainError::StoreError(StoreError::Custom(
        "Could not find latest valid hash".to_string(),
    )))
}

/// Searchs the header of the parent block header. If the parent header is missing,
/// Returns a ChainError::ParentNotFound. If the storage has an error it propagates it
pub fn find_parent_header(
    block_header: &BlockHeader,
    storage: &Store,
) -> Result<BlockHeader, ChainError> {
    match storage.get_block_header_by_hash(block_header.parent_hash)? {
        Some(parent_header) => Ok(parent_header),
        None => Err(ChainError::ParentNotFound),
    }
}

/// Performs pre-execution validation of the block's header values in reference to the parent_header
/// Verifies that blob gas fields in the header are correct in reference to the block's body.
/// If a block passes this check, execution will still fail with execute_block when a transaction runs out of gas
///
/// Note that this doesn't validate that the transactions or withdrawals root of the header matches the body
/// contents, since we assume the caller already did it. And, in any case, that wouldn't invalidate the block header.
pub fn validate_block(
    block: &Block,
    parent_header: &BlockHeader,
    chain_config: &ChainConfig,
    elasticity_multiplier: u64,
) -> Result<(), ChainError> {
    // Verify initial header validity against parent
    validate_block_header(&block.header, parent_header, elasticity_multiplier)
        .map_err(InvalidBlockError::from)?;

    if chain_config.is_osaka_activated(block.header.timestamp) {
        let block_rlp_size = block.length();
        if block_rlp_size > MAX_RLP_BLOCK_SIZE as usize {
            return Err(error::ChainError::InvalidBlock(
                InvalidBlockError::MaximumRlpSizeExceeded(
                    MAX_RLP_BLOCK_SIZE,
                    block_rlp_size as u64,
                ),
            ));
        }
    }
    if chain_config.is_prague_activated(block.header.timestamp) {
        validate_prague_header_fields(&block.header, parent_header, chain_config)
            .map_err(InvalidBlockError::from)?;
        verify_blob_gas_usage(block, chain_config)?;
        if chain_config.is_osaka_activated(block.header.timestamp) {
            verify_transaction_max_gas_limit(block)?;
        }
    } else if chain_config.is_cancun_activated(block.header.timestamp) {
        validate_cancun_header_fields(&block.header, parent_header, chain_config)
            .map_err(InvalidBlockError::from)?;
        verify_blob_gas_usage(block, chain_config)?;
    } else {
        validate_pre_cancun_header_fields(&block.header).map_err(InvalidBlockError::from)?
    }

    Ok(())
}

pub async fn is_canonical(
    store: &Store,
    block_number: BlockNumber,
    block_hash: BlockHash,
) -> Result<bool, StoreError> {
    match store.get_canonical_block_hash(block_number).await? {
        Some(hash) if hash == block_hash => Ok(true),
        _ => Ok(false),
    }
}

pub fn validate_gas_used(
    receipts: &[Receipt],
    block_header: &BlockHeader,
) -> Result<(), ChainError> {
    if let Some(last) = receipts.last()
        && last.cumulative_gas_used != block_header.gas_used
    {
        return Err(ChainError::InvalidBlock(
            InvalidBlockError::GasUsedMismatch(last.cumulative_gas_used, block_header.gas_used),
        ));
    }
    Ok(())
}

// Perform validations over the block's blob gas usage.
// Must be called only if the block has cancun activated
fn verify_blob_gas_usage(block: &Block, config: &ChainConfig) -> Result<(), ChainError> {
    let mut blob_gas_used = 0_u32;
    let mut blobs_in_block = 0_u32;
    let max_blob_number_per_block = config
        .get_fork_blob_schedule(block.header.timestamp)
        .map(|schedule| schedule.max)
        .ok_or(ChainError::Custom("Provided block fork is invalid".into()))?;
    let max_blob_gas_per_block = max_blob_number_per_block * GAS_PER_BLOB;

    for transaction in block.body.transactions.iter() {
        if let Transaction::EIP4844Transaction(tx) = transaction {
            blob_gas_used += get_total_blob_gas(tx);
            blobs_in_block += tx.blob_versioned_hashes.len() as u32;
        }
    }
    if blob_gas_used > max_blob_gas_per_block {
        return Err(ChainError::InvalidBlock(
            InvalidBlockError::ExceededMaxBlobGasPerBlock,
        ));
    }
    if blobs_in_block > max_blob_number_per_block {
        return Err(ChainError::InvalidBlock(
            InvalidBlockError::ExceededMaxBlobNumberPerBlock,
        ));
    }
    if block
        .header
        .blob_gas_used
        .is_some_and(|header_blob_gas_used| header_blob_gas_used != blob_gas_used as u64)
    {
        return Err(ChainError::InvalidBlock(
            InvalidBlockError::BlobGasUsedMismatch,
        ));
    }
    Ok(())
}

// Perform validations over the block's gas usage.
// Must be called only if the block has osaka activated
// as specified in https://eips.ethereum.org/EIPS/eip-7825
fn verify_transaction_max_gas_limit(block: &Block) -> Result<(), ChainError> {
    for transaction in block.body.transactions.iter() {
        if transaction.gas_limit() > POST_OSAKA_GAS_LIMIT_CAP {
            return Err(ChainError::InvalidBlock(
                InvalidBlockError::InvalidTransaction(format!(
                    "Transaction gas limit exceeds maximum. Transaction hash: {}, transaction gas limit: {}",
                    transaction.hash(),
                    transaction.gas_limit()
                )),
            ));
        }
    }
    Ok(())
}

/// Calculates the blob gas required by a transaction
pub fn get_total_blob_gas(tx: &EIP4844Transaction) -> u32 {
    GAS_PER_BLOB * tx.blob_versioned_hashes.len() as u32
}

/// Collapses a root branch node into an extension or leaf node if it has only one valid child.
/// Returns None if there are no valid children.
///
/// NOTE: this assumes the branch has 0 or 1 children. If there are more than 1,
/// it will discard all but the first valid child found.
#[cold]
fn collapse_root_node(
    real_root: Box<BranchNode>,
    state_updates_map: &mut FxHashMap<Nibbles, Vec<u8>>,
    storage: &Store,
    parent_header: &BlockHeader,
) -> Result<Option<Node>, StoreError> {
    // Collapse the branch into an extension or leaf
    let Some((choice, only_child)) = real_root
        .choices
        .into_iter()
        .enumerate()
        .find(|(_, c)| c.is_valid())
    else {
        return Ok(None);
    };
    let path = Nibbles::from_hex(vec![choice as u8]);
    let child_bytes = match state_updates_map.get(&path) {
        Some(v) => v.clone(),
        None => storage
            .state_trie(parent_header.hash())?
            .ok_or(StoreError::MissingStore)?
            .db()
            .get(path)?
            .ok_or_else(|| StoreError::Custom("Missing child node during root collapse".into()))?,
    };
    // Same match as in [`BranchNode::remove`]
    let child = match Node::decode(&child_bytes)? {
        // Replace root with an extension node leading to the child
        Node::Branch(_) => {
            ExtensionNode::new(Nibbles::from_hex(vec![choice as u8]), only_child).into()
        }
        // Replace root with the child extension node, updating its path in the process
        Node::Extension(mut extension_node) => {
            let mut extension_node = extension_node.take();
            extension_node.prefix.prepend(choice as u8);
            extension_node.into()
        }
        Node::Leaf(mut leaf) => {
            let mut leaf = leaf.take();
            leaf.partial.prepend(choice as u8);
            leaf.into()
        }
    };
    Ok(Some(child))
}

#[cfg(test)]
mod tests {}
