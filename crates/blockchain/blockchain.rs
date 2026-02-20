//! # ethrex Blockchain
//!
//! Core blockchain logic for the ethrex Ethereum client.
//!
//! ## Overview
//!
//! This module implements the blockchain layer, which is responsible for:
//! - Block validation and execution
//! - State management and transitions
//! - Fork choice rule implementation
//! - Transaction mempool management
//! - Payload building for block production
//!
//! ## Key Components
//!
//! - [`Blockchain`]: Main interface for blockchain operations
//! - [`Mempool`]: Transaction pool for pending transactions
//! - [`fork_choice`]: Fork choice rule implementation
//! - [`payload`]: Block payload building for consensus
//!
//! ## Block Execution Flow
//!
//! ```text
//! 1. Receive block from consensus/P2P
//! 2. Validate block header (parent, timestamp, gas limit, etc.)
//! 3. Execute transactions in EVM
//! 4. Verify state root matches header
//! 5. Store block and update canonical chain
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! use ethrex_blockchain::Blockchain;
//!
//! let blockchain = Blockchain::new(store, BlockchainOptions::default());
//!
//! // Add a block
//! blockchain.add_block(&block)?;
//!
//! // Add transaction to mempool
//! blockchain.add_transaction_to_mempool(tx).await?;
//! ```

pub mod constants;
pub mod error;
pub mod fork_choice;
pub mod mempool;
pub mod payload;
pub mod tracing;
pub mod vm;

use ::tracing::{debug, info, instrument, warn};
use constants::{MAX_INITCODE_SIZE, MAX_TRANSACTION_DATA_SIZE, POST_OSAKA_GAS_LIMIT_CAP};
use error::MempoolError;
use error::{ChainError, InvalidBlockError};
use ethrex_common::constants::{EMPTY_TRIE_HASH, MIN_BASE_FEE_PER_BLOB_GAS};

// Re-export stateless validation functions for backwards compatibility
#[cfg(feature = "c-kzg")]
use ethrex_common::types::EIP4844Transaction;
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::block_execution_witness::ExecutionWitness;
use ethrex_common::types::fee_config::FeeConfig;
use ethrex_common::types::{
    AccountInfo, AccountState, AccountUpdate, Block, BlockHash, BlockHeader, BlockNumber,
    ChainConfig, Code, Receipt, Transaction, WrappedEIP4844Transaction,
};
use ethrex_common::types::{ELASTICITY_MULTIPLIER, P2PTransaction};
use ethrex_common::types::{Fork, MempoolTransaction};
use ethrex_common::utils::keccak;
use ethrex_common::{Address, H256, TrieLogger, U256};
pub use ethrex_common::{
    get_total_blob_gas, validate_block, validate_block_access_list_hash, validate_gas_used,
    validate_receipts_root, validate_requests_hash,
};
use ethrex_metrics::metrics;
use ethrex_rlp::constants::RLP_NULL;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{
    AccountUpdatesList, Store, UpdateBatch, error::StoreError, hash_address, hash_key,
};
use ethrex_trie::node::{BranchNode, ExtensionNode, LeafNode};
use ethrex_trie::{Nibbles, Node, NodeRef, Trie, TrieError, TrieNode};
use ethrex_vm::backends::CachingDatabase;
use ethrex_vm::backends::levm::LEVM;
use ethrex_vm::backends::levm::db::DatabaseLogger;
use ethrex_vm::{BlockExecutionResult, DynVmDatabase, Evm, EvmError};
use mempool::Mempool;
use payload::PayloadOrTask;
use rustc_hash::FxHashMap;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{Receiver, channel},
};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as TokioMutex;
use tokio_util::sync::CancellationToken;

use vm::StoreVmDatabase;

#[cfg(feature = "metrics")]
use ethrex_metrics::blocks::METRICS_BLOCKS;

#[cfg(feature = "c-kzg")]
use ethrex_common::types::BlobsBundle;

const MAX_PAYLOADS: usize = 10;
const MAX_MEMPOOL_SIZE_DEFAULT: usize = 10_000;

// Result type for execute_block_pipeline
type BlockExecutionPipelineResult = (
    BlockExecutionResult,
    AccountUpdatesList,
    Option<Vec<AccountUpdate>>,
    usize,        // max queue length
    [Instant; 6], // timing instants
    Duration,     // warmer duration
);

/// Structured block-pipeline metrics for a single `add_block_pipeline` execution.
#[derive(Debug, Clone)]
pub struct AddBlockPipelineMetrics {
    pub gas_used: u64,
    pub gas_limit: u64,
    pub block_number: u64,
    pub transactions_count: usize,
    pub merkle_queue_length: usize,
    pub total_ms: u64,
    pub throughput_ggas_per_s: f64,
    pub validate_ms: u64,
    pub exec_ms: u64,
    pub merkle_concurrent_ms: u64,
    pub merkle_drain_ms: u64,
    pub merkle_total_ms: u64,
    pub merkle_overlap_pct: u64,
    pub store_ms: u64,
    pub warmer_ms: u64,
    pub warmer_early_ms: i64,
    pub bottleneck_phase: &'static str,
}

//TODO: Implement a struct Chain or BlockChain to encapsulate
//functionality and canonical chain state and config

/// Specifies whether the blockchain operates as L1 (mainnet/testnet) or L2 (rollup).
#[derive(Debug, Clone, Default)]
pub enum BlockchainType {
    /// Standard Ethereum L1 blockchain.
    #[default]
    L1,
    /// Layer 2 rollup with additional fee configuration.
    L2(L2Config),
}

/// Configuration for L2 rollup operation.
#[derive(Debug, Clone, Default)]
pub struct L2Config {
    /// Fee configuration for L2 transactions.
    ///
    /// Uses `RwLock` because the Watcher updates L1 fee config periodically.
    pub fee_config: Arc<RwLock<FeeConfig>>,
}

/// Core blockchain implementation for block validation and execution.
///
/// The `Blockchain` struct is the main entry point for all blockchain operations:
/// - Adding and validating blocks
/// - Managing the transaction mempool
/// - Building payloads for block production
/// - Handling fork choice updates
///
/// # Thread Safety
///
/// `Blockchain` uses interior mutability for thread-safe access to shared state.
/// The mempool and payload storage are protected by appropriate synchronization primitives.
///
/// # Example
///
/// ```ignore
/// let blockchain = Blockchain::new(store, BlockchainOptions::default());
///
/// // Validate and add a block
/// blockchain.add_block(&block)?;
///
/// // Check sync status
/// if blockchain.is_synced() {
///     // Process transactions from mempool
/// }
/// ```
#[derive(Debug)]
pub struct Blockchain {
    /// Underlying storage for blocks and state.
    storage: Store,
    /// Transaction mempool for pending transactions.
    pub mempool: Mempool,
    /// Whether the node has completed initial sync.
    ///
    /// Set to true after initial sync completes, never reset to false.
    /// Does not reflect whether an ongoing sync is in progress.
    is_synced: AtomicBool,
    /// Configuration options for blockchain behavior.
    pub options: BlockchainOptions,
    /// Cache of recently built payloads.
    ///
    /// Maps payload IDs to either completed payloads or in-progress build tasks.
    /// Kept around in case consensus requests the same payload twice.
    pub payloads: Arc<TokioMutex<Vec<(u64, PayloadOrTask)>>>,
}

/// Configuration options for the blockchain.
#[derive(Debug, Clone)]
pub struct BlockchainOptions {
    /// Maximum number of transactions in the mempool.
    pub max_mempool_size: usize,
    /// Whether to emit performance logging.
    pub perf_logs_enabled: bool,
    /// Blockchain type (L1 or L2).
    pub r#type: BlockchainType,
    /// EIP-7872: User-configured maximum blobs per block for local building.
    /// If None, uses the protocol maximum for the current fork.
    pub max_blobs_per_block: Option<u32>,
    /// If true, computes execution witnesses upon receiving newPayload messages and stores them in local storage
    pub precompute_witnesses: bool,
}

impl Default for BlockchainOptions {
    fn default() -> Self {
        Self {
            max_mempool_size: MAX_MEMPOOL_SIZE_DEFAULT,
            perf_logs_enabled: false,
            r#type: BlockchainType::default(),
            max_blobs_per_block: None,
            precompute_witnesses: false,
        }
    }
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

enum MerklizationRequest {
    LoadAccount(H256),
    Delete(H256),
    MerklizeStorage {
        prefix: H256,
        key: H256,
        value: U256,
    },
    MerklizeAccount {
        hashed_account: H256,
        state: PreMerkelizedAccountState,
    },
    CollectStorages {
        tx: Sender<CollectedStorageMsg>,
    },
    CollectState {
        tx: Sender<CollectedStateMsg>,
    },
}

struct CollectedStateMsg {
    index: u8,
    subroot: Box<BranchNode>,
    state_nodes: Vec<TrieNode>,
    storage_nodes: Vec<(H256, Vec<TrieNode>)>,
}

struct CollectedStorageMsg {
    index: u8,
    prefix: H256,
    subroot: Box<BranchNode>,
    nodes: Vec<TrieNode>,
}

#[derive(Default)]
struct PreMerkelizedAccountState {
    info: Option<AccountInfo>,
    storage_root: Option<Box<BranchNode>>,
    nodes: Vec<TrieNode>,
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

        let (execution_result, bal) = vm.execute_block(block)?;
        let account_updates = vm.get_state_transitions()?;

        // Validate execution went alright
        validate_gas_used(execution_result.block_gas_used, &block.header)?;
        validate_receipts_root(&block.header, &execution_result.receipts)?;
        validate_requests_hash(&block.header, &chain_config, &execution_result.requests)?;
        if let Some(bal) = &bal {
            validate_block_access_list_hash(
                &block.header,
                &chain_config,
                bal,
                block.body.transactions.len(),
            )?;
        }

        Ok((execution_result, account_updates))
    }

    /// Generates Block Access List by re-executing a block.
    /// Returns None for pre-Amsterdam blocks.
    /// This is used by engine_getPayloadBodiesByHashV2 and engine_getPayloadBodiesByRangeV2.
    pub fn generate_bal_for_block(
        &self,
        block: &Block,
    ) -> Result<Option<BlockAccessList>, ChainError> {
        let chain_config = self.storage.get_chain_config();

        // Pre-Amsterdam blocks don't have BAL
        if !chain_config.is_amsterdam_activated(block.header.timestamp) {
            return Ok(None);
        }

        // Find parent header
        let parent_header = find_parent_header(&block.header, &self.storage)?;

        // Create VM and execute block with BAL recording
        let vm_db = StoreVmDatabase::new(self.storage.clone(), parent_header)?;
        let mut vm = self.new_evm(vm_db)?;

        let (_execution_result, bal) = vm.execute_block(block)?;

        Ok(bal)
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
        vm: &mut Evm,
    ) -> Result<BlockExecutionPipelineResult, ChainError> {
        let start_instant = Instant::now();

        let chain_config = self.storage.get_chain_config();

        // Validate the block pre-execution
        validate_block(block, parent_header, &chain_config, ELASTICITY_MULTIPLIER)?;
        let block_validated_instant = Instant::now();

        let exec_merkle_start = Instant::now();
        let queue_length = AtomicUsize::new(0);
        let queue_length_ref = &queue_length;
        let mut max_queue_length = 0;

        // Wrap the store with CachingDatabase so both warming and execution
        // can benefit from shared caching of state lookups
        let original_store = vm.db.store.clone();
        let caching_store: Arc<dyn ethrex_vm::backends::LevmDatabase> =
            Arc::new(CachingDatabase::new(original_store));

        // Replace the VM's store with the caching version
        vm.db.store = caching_store.clone();

        let (execution_result, merkleization_result, warmer_duration) =
            std::thread::scope(|s| -> Result<_, ChainError> {
                let vm_type = vm.vm_type;
                let warm_handle = std::thread::Builder::new()
                    .name("block_executor_warmer".to_string())
                    .spawn_scoped(s, move || {
                        // Warming uses the same caching store, sharing cached state with execution
                        let start = Instant::now();
                        let _ = LEVM::warm_block(block, caching_store, vm_type);
                        start.elapsed()
                    })
                    .map_err(|e| {
                        ChainError::Custom(format!("Failed to spawn warmer thread: {e}"))
                    })?;
                let max_queue_length_ref = &mut max_queue_length;
                let (tx, rx) = channel();
                let execution_handle = std::thread::Builder::new()
                    .name("block_executor_execution".to_string())
                    .spawn_scoped(s, move || -> Result<_, ChainError> {
                        let (execution_result, bal) =
                            vm.execute_block_pipeline(block, tx, queue_length_ref)?;

                        // Validate execution went alright
                        validate_gas_used(execution_result.block_gas_used, &block.header)?;
                        validate_receipts_root(&block.header, &execution_result.receipts)?;
                        validate_requests_hash(
                            &block.header,
                            &chain_config,
                            &execution_result.requests,
                        )?;
                        if let Some(bal) = &bal {
                            validate_block_access_list_hash(
                                &block.header,
                                &chain_config,
                                bal,
                                block.body.transactions.len(),
                            )?;
                        }

                        let exec_end_instant = Instant::now();
                        Ok((execution_result, exec_end_instant))
                    })
                    .map_err(|e| {
                        ChainError::Custom(format!("Failed to spawn execution thread: {e}"))
                    })?;
                let parent_header_ref = &parent_header; // Avoid moving to thread
                let merkleize_handle = std::thread::Builder::new()
                    .name("block_executor_merkleizer".to_string())
                    .spawn_scoped(s, move || -> Result<_, StoreError> {
                        let (account_updates_list, accumulated_updates) = self
                            .handle_merkleization(
                                s,
                                rx,
                                parent_header_ref,
                                queue_length_ref,
                                max_queue_length_ref,
                            )?;
                        let merkle_end_instant = Instant::now();
                        Ok((
                            account_updates_list,
                            accumulated_updates,
                            merkle_end_instant,
                        ))
                    })
                    .map_err(|e| {
                        ChainError::Custom(format!("Failed to spawn merkleizer thread: {e}"))
                    })?;
                let warmer_duration = warm_handle
                    .join()
                    .inspect_err(|e| warn!("Warming thread error: {e:?}"))
                    .ok()
                    .unwrap_or(Duration::ZERO);
                Ok((
                    execution_handle.join().unwrap_or_else(|_| {
                        Err(ChainError::Custom("execution thread panicked".to_string()))
                    }),
                    merkleize_handle.join().unwrap_or_else(|_| {
                        Err(StoreError::Custom(
                            "merklization thread panicked".to_string(),
                        ))
                    }),
                    warmer_duration,
                ))
            })?;
        let (account_updates_list, accumulated_updates, merkle_end_instant) = merkleization_result?;
        let (execution_result, exec_end_instant) = execution_result?;

        let exec_merkle_end_instant = Instant::now();

        Ok((
            execution_result,
            account_updates_list,
            accumulated_updates,
            max_queue_length,
            [
                start_instant,
                block_validated_instant,
                exec_merkle_start,
                exec_end_instant,
                merkle_end_instant,
                exec_merkle_end_instant,
            ],
            warmer_duration,
        ))
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
    ) -> Result<(AccountUpdatesList, Option<Vec<AccountUpdate>>), StoreError>
    where
        'a: 's,
        'b: 's,
    {
        let mut workers_tx = Vec::with_capacity(16);
        let mut workers_handles = Vec::with_capacity(16);
        for i in 0..16 {
            let (tx, rx) = channel();
            let handle = std::thread::Builder::new()
                .name(format!("block_executor_merkleization_shard_worker_{i}"))
                .spawn_scoped(scope, move || {
                    self.handle_merkleization_subtrie(rx, parent_header, i)
                })
                .map_err(|e| StoreError::Custom(format!("spawn failed: {e:?}",)))?;
            workers_handles.push(handle);
            workers_tx.push(tx);
        }

        let mut account_state: FxHashMap<H256, PreMerkelizedAccountState> = Default::default();
        let mut code_updates: Vec<(H256, Code)> = vec![];
        let mut hashed_address_cache: FxHashMap<Address, H256> = Default::default();

        // Accumulator for witness generation (only used if precompute_witnesses is true)
        let mut accumulator: Option<FxHashMap<Address, AccountUpdate>> =
            if self.options.precompute_witnesses {
                Some(FxHashMap::default())
            } else {
                None
            };

        for updates in rx {
            let current_length = queue_length.fetch_sub(1, Ordering::Acquire);
            *max_queue_length = current_length.max(*max_queue_length);
            // Accumulate updates for witness generation if enabled
            if let Some(acc) = &mut accumulator {
                for update in updates.clone() {
                    match acc.entry(update.address) {
                        Entry::Vacant(e) => {
                            e.insert(update);
                        }
                        Entry::Occupied(mut e) => {
                            e.get_mut().merge(update);
                        }
                    }
                }
            }

            for update in updates {
                let hashed_address = *hashed_address_cache
                    .entry(update.address)
                    .or_insert_with(|| keccak(update.address));
                let account_bucket = hashed_address.as_fixed_bytes()[0] >> 4;
                workers_tx[account_bucket as usize]
                    .send(MerklizationRequest::LoadAccount(hashed_address))
                    .map_err(|e| StoreError::Custom(format!("send error: {e}")))?;
                if update.removed {
                    // Match old behavior: remove account, skip added_storage processing.
                    // Send Delete to clear any existing storage in workers so the
                    // storage root becomes EMPTY_TRIE_HASH during collection.
                    for tx in &workers_tx {
                        tx.send(MerklizationRequest::Delete(hashed_address))
                            .map_err(|e| StoreError::Custom(format!("send error: {e}")))?;
                    }
                    let state = account_state.entry(hashed_address).or_default();
                    *state = PreMerkelizedAccountState {
                        info: Some(Default::default()),
                        ..Default::default()
                    };
                    continue;
                }

                if update.removed_storage {
                    for tx in &workers_tx {
                        tx.send(MerklizationRequest::Delete(hashed_address))
                            .map_err(|e| StoreError::Custom(format!("send error: {e}")))?;
                    }
                }
                for (key, value) in update.added_storage {
                    let hashed_key = keccak(key);
                    let bucket = hashed_key.as_fixed_bytes()[0] >> 4;
                    workers_tx[bucket as usize]
                        .send(MerklizationRequest::MerklizeStorage {
                            prefix: hashed_address,
                            key: hashed_key,
                            value,
                        })
                        .map_err(|e| StoreError::Custom(format!("send error: {e}")))?;
                }
                let state = account_state.entry(hashed_address).or_default();
                if let Some(info) = update.info {
                    if let Some(code) = update.code {
                        code_updates.push((info.code_hash, code));
                    }
                    state.info = Some(info);
                }
            }
        }

        let (gatherer_tx, gatherer_rx) = channel();
        for tx in &workers_tx {
            tx.send(MerklizationRequest::CollectStorages {
                tx: gatherer_tx.clone(),
            })
            .map_err(|e| StoreError::Custom(format!("send error: {e}")))?;
        }
        drop(gatherer_tx);

        for CollectedStorageMsg {
            index,
            prefix,
            mut subroot,
            nodes,
        } in gatherer_rx
        {
            let state = account_state.entry(prefix).or_default();
            match &mut state.storage_root {
                Some(root) => {
                    root.choices[index as usize] =
                        std::mem::take(&mut subroot.choices[index as usize]);
                }
                rootptr => {
                    *rootptr = Some(subroot);
                }
            }
            state.nodes.extend(nodes);
        }

        let mut storage_updates: Vec<(H256, Vec<TrieNode>)> = Default::default();

        for (hashed_account, state) in account_state {
            let bucket = hashed_account.as_fixed_bytes()[0] >> 4;
            workers_tx[bucket as usize]
                .send(MerklizationRequest::MerklizeAccount {
                    hashed_account,
                    state,
                })
                .map_err(|e| StoreError::Custom(format!("send error: {e}")))?;
        }

        let (gatherer_tx, gatherer_rx) = channel();
        for tx in &workers_tx {
            tx.send(MerklizationRequest::CollectState {
                tx: gatherer_tx.clone(),
            })
            .map_err(|e| StoreError::Custom(format!("send error: {e}")))?;
        }
        drop(gatherer_tx);

        let mut root = BranchNode::default();
        let mut state_updates = Vec::new();
        for CollectedStateMsg {
            index,
            subroot,
            state_nodes,
            storage_nodes,
        } in gatherer_rx
        {
            storage_updates.extend(storage_nodes);
            state_updates.extend(state_nodes);
            root.choices[index as usize] = subroot.choices[index as usize].clone();
        }
        let state_trie_hash =
            if let Some(root) = self.collapse_root_node(parent_header, None, root)? {
                let mut root = NodeRef::from(root);
                let hash = root.commit(Nibbles::default(), &mut state_updates);
                hash.finalize()
            } else {
                state_updates.push((Nibbles::default(), vec![RLP_NULL]));
                *EMPTY_TRIE_HASH
            };

        let accumulated_updates = accumulator.map(|acc| acc.into_values().collect());

        Ok((
            AccountUpdatesList {
                state_trie_hash,
                state_updates,
                storage_updates,
                code_updates,
            },
            accumulated_updates,
        ))
    }

    fn load_trie(
        &self,
        parent_header: &BlockHeader,
        prefix: Option<H256>,
    ) -> Result<Trie, StoreError> {
        Ok(match prefix {
            Some(account_hash) => {
                let state_trie = self.storage.open_state_trie(parent_header.state_root)?;
                let storage_root = match state_trie.get(account_hash.as_bytes())? {
                    Some(rlp) => AccountState::decode(&rlp)?.storage_root,
                    None => *EMPTY_TRIE_HASH,
                };
                self.storage.open_storage_trie(
                    account_hash,
                    parent_header.state_root,
                    storage_root,
                )?
            }
            None => self.storage.open_state_trie(parent_header.state_root)?,
        })
    }

    /// Collapses a root branch node into an extension or leaf node if it has only one valid child.
    /// Returns None if there are no valid children.
    fn collapse_root_node(
        &self,
        parent_header: &BlockHeader,
        prefix: Option<H256>,
        mut root: BranchNode,
    ) -> Result<Option<Node>, StoreError> {
        // Ensures the children are included in the final commit
        root.choices.iter_mut().for_each(NodeRef::clear_hash);
        let children: Vec<(usize, &NodeRef)> = root
            .choices
            .iter()
            .enumerate()
            .filter(|(_, choice)| choice.is_valid())
            .take(2)
            .collect();
        if children.len() > 1 {
            return Ok(Some(Node::Branch(Box::from(root))));
        }
        let Some((choice, only_child)) = children.first() else {
            return Ok(None);
        };
        let only_child = Arc::unwrap_or_clone(match only_child {
            NodeRef::Node(node, _) => node.clone(),
            noderef @ NodeRef::Hash(_) => {
                let trie = self.load_trie(parent_header, prefix)?;
                let Some(node) =
                    noderef.get_node(trie.db(), Nibbles::from_hex(vec![*choice as u8]))?
                else {
                    return Ok(None);
                };
                node
            }
        });
        Ok(Some(match only_child {
            Node::Branch(_) => {
                ExtensionNode::new(Nibbles::from_hex(vec![*choice as u8]), only_child.into()).into()
            }
            Node::Extension(mut extension_node) => {
                extension_node.prefix.prepend(*choice as u8);
                extension_node.into()
            }
            Node::Leaf(mut leaf) => {
                leaf.partial.prepend(*choice as u8);
                leaf.into()
            }
        }))
    }

    fn handle_merkleization_subtrie(
        &self,
        rx: Receiver<MerklizationRequest>,
        parent_header: &BlockHeader,
        index: u8,
    ) -> Result<(), StoreError> {
        let mut tree: FxHashMap<H256, Trie> = Default::default();
        let mut state_trie = self.storage.open_state_trie(parent_header.state_root)?;
        let mut storage_nodes = vec![];
        let mut accounts: FxHashMap<H256, AccountState> = Default::default();
        for msg in rx {
            match msg {
                MerklizationRequest::LoadAccount(prefix) => match accounts.entry(prefix) {
                    Entry::Occupied(_) => {}
                    Entry::Vacant(vacant_entry) => {
                        let account_state = match state_trie.get(prefix.as_bytes())? {
                            Some(rlp) => {
                                let state = AccountState::decode(&rlp)?;
                                state_trie.insert(prefix.as_bytes().to_vec(), rlp)?;
                                state
                            }
                            None => AccountState::default(),
                        };
                        vacant_entry.insert(account_state);
                    }
                },
                MerklizationRequest::Delete(prefix) => {
                    tree.insert(prefix, Trie::new_temp());
                }
                MerklizationRequest::MerklizeStorage { prefix, key, value } => {
                    let trie = match tree.entry(prefix) {
                        Entry::Occupied(occupied_entry) => occupied_entry.into_mut(),
                        Entry::Vacant(vacant_entry) => {
                            let storage_root = match state_trie.get(prefix.as_bytes())? {
                                Some(rlp) => AccountState::decode(&rlp)?.storage_root,
                                None => *EMPTY_TRIE_HASH,
                            };
                            vacant_entry.insert(self.storage.open_storage_trie(
                                prefix,
                                parent_header.state_root,
                                storage_root,
                            )?)
                        }
                    };
                    if value.is_zero() {
                        trie.remove(key.as_bytes())?;
                    } else {
                        trie.insert(key.as_bytes().to_vec(), value.encode_to_vec())?;
                    }
                }
                MerklizationRequest::MerklizeAccount {
                    hashed_account,
                    mut state,
                } => {
                    let mut storage_root = None;
                    if let Some(root) = state.storage_root {
                        if let Some(root) =
                            self.collapse_root_node(parent_header, Some(hashed_account), *root)?
                        {
                            let mut root = NodeRef::from(root);
                            let hash = root.commit(Nibbles::default(), &mut state.nodes);
                            storage_root = Some(hash.finalize());
                        } else {
                            state.nodes.push((Nibbles::default(), vec![RLP_NULL]));
                            storage_root = Some(*EMPTY_TRIE_HASH);
                        }
                    }
                    storage_nodes.push((hashed_account, state.nodes));

                    let path = hashed_account.as_bytes();
                    let old_state = match accounts.entry(hashed_account) {
                        Entry::Occupied(occupied_entry) => occupied_entry.into_mut(),
                        Entry::Vacant(vacant_entry) => {
                            let account_state = match state_trie.get(path)? {
                                Some(rlp) => AccountState::decode(&rlp)?,
                                None => AccountState::default(),
                            };
                            vacant_entry.insert(account_state)
                        }
                    };

                    if let Some(storage_root) = storage_root {
                        old_state.storage_root = storage_root;
                    }
                    if let Some(info) = state.info {
                        old_state.nonce = info.nonce;
                        old_state.balance = info.balance;
                        old_state.code_hash = info.code_hash;
                    }
                    if *old_state != AccountState::default() {
                        state_trie.insert(path.to_vec(), old_state.encode_to_vec())?;
                    } else {
                        state_trie.remove(path)?;
                    }
                }
                MerklizationRequest::CollectStorages { tx } => {
                    for (prefix, trie) in tree.drain() {
                        let (root, nodes) = collect_trie(index, trie)?;
                        tx.send(CollectedStorageMsg {
                            index,
                            prefix,
                            subroot: root,
                            nodes,
                        })
                        .map_err(|e| StoreError::Custom(format!("send error: {e}")))?;
                    }
                }
                MerklizationRequest::CollectState { tx } => {
                    let (subroot, state_nodes) =
                        collect_trie(index, std::mem::take(&mut state_trie))?;
                    tx.send(CollectedStateMsg {
                        index,
                        subroot,
                        state_nodes,
                        storage_nodes: std::mem::take(&mut storage_nodes),
                    })
                    .map_err(|e| StoreError::Custom(format!("send error: {e}")))?;
                }
            }
        }
        Ok(())
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
        let (execution_result, bal) = vm.execute_block(block)?;
        // Validate execution went alright
        validate_gas_used(execution_result.block_gas_used, &block.header)?;
        validate_receipts_root(&block.header, &execution_result.receipts)?;
        validate_requests_hash(&block.header, chain_config, &execution_result.requests)?;
        if let Some(bal) = &bal {
            validate_block_access_list_hash(
                &block.header,
                chain_config,
                bal,
                block.body.transactions.len(),
            )?;
        }

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

            let logger = Arc::new(DatabaseLogger::new(Arc::new(vm_db)));

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
            let (execution_result, _bal) = vm.execute_block(block)?;

            // Gather account updates
            let account_updates = vm.get_state_transitions()?;

            let mut state_accessed = logger
                .state_accessed
                .lock()
                .map_err(|_e| {
                    ChainError::WitnessGeneration("Failed to execute with witness".to_string())
                })?
                .clone();

            // Deduplicate storage keys while preserving access order
            for keys in state_accessed.values_mut() {
                let mut seen = HashSet::new();
                keys.retain(|k| seen.insert(*k));
            }

            for (account, acc_keys) in state_accessed.iter() {
                let slots: &mut Vec<H256> =
                    touched_account_storage_slots.entry(*account).or_default();
                slots.extend(acc_keys.iter().copied());
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
            for (account, acc_keys) in state_accessed.iter() {
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
            let (storage_tries_after_update, account_updates_list) =
                self.storage.apply_account_updates_from_trie_with_witness(
                    trie,
                    &account_updates,
                    used_storage_tries,
                )?;

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

    pub fn generate_witness_from_account_updates(
        &self,
        account_updates: Vec<AccountUpdate>,
        block: &Block,
        parent_header: BlockHeader,
        logger: &DatabaseLogger,
    ) -> Result<ExecutionWitness, ChainError> {
        // Get state at previous block
        let trie = self
            .storage
            .state_trie(parent_header.hash())
            .map_err(|_| ChainError::ParentStateNotFound)?
            .ok_or(ChainError::ParentStateNotFound)?;
        let initial_state_root = trie.hash_no_commit();

        let (trie_witness, trie) = TrieLogger::open_trie(trie);

        let mut touched_account_storage_slots = BTreeMap::new();
        // This will become the state trie + storage trie
        let mut used_trie_nodes = Vec::new();

        // Store the root node in case the block is empty and the witness does not record any nodes
        let root_node = trie.root_node().map_err(|_| {
            ChainError::WitnessGeneration("Failed to get root state node".to_string())
        })?;

        let mut codes = Vec::new();

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
        let blockhash_opcode_references = logger
            .block_hashes_accessed
            .lock()
            .map_err(|_e| ChainError::WitnessGeneration("Failed to get block hashes".to_string()))?
            .clone();

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
                && let Ok(Some(storage_trie)) =
                    self.storage.storage_trie(parent_header.hash(), *account)
            {
                let (storage_trie_witness, storage_trie) = TrieLogger::open_trie(storage_trie);
                // Access all the keys
                for storage_key in acc_keys {
                    let hashed_key = hash_key(storage_key);
                    storage_trie.get(&hashed_key).map_err(|_e| {
                        ChainError::WitnessGeneration("Failed to access storage key".to_string())
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
        let (storage_tries_after_update, _account_updates_list) =
            self.storage.apply_account_updates_from_trie_with_witness(
                trie,
                &account_updates,
                used_storage_tries,
            )?;

        for (address, (witness, _storage_trie)) in storage_tries_after_update {
            let mut witness = witness.lock().map_err(|_| {
                ChainError::WitnessGeneration("Failed to lock storage trie witness".to_string())
            })?;
            let witness = std::mem::take(&mut *witness);
            let witness = witness.into_values().collect::<Vec<_>>();
            used_trie_nodes.extend_from_slice(&witness);
            touched_account_storage_slots.entry(address).or_default();
        }

        used_trie_nodes.extend_from_slice(&Vec::from_iter(
            trie_witness
                .lock()
                .map_err(|_| {
                    ChainError::WitnessGeneration("Failed to lock state trie witness".to_string())
                })?
                .clone()
                .into_values(),
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
                (*n < block.header.number.saturating_sub(1))
                    .then(|| blockhash_opcode_references.get(n))?
                    .copied()
            })
            .unwrap_or(block.header.parent_hash);

        let mut current_header = block.header.clone();

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
            first_block_number: parent_header.number,
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
        self.add_block_pipeline_with_metrics(block).map(|_| ())
    }

    pub fn add_block_pipeline_with_metrics(
        &self,
        block: Block,
    ) -> Result<AddBlockPipelineMetrics, ChainError> {
        // Validate if it can be the new head and find the parent
        let Ok(parent_header) = find_parent_header(&block.header, &self.storage) else {
            // If the parent is not present, we store it as pending.
            self.storage.add_pending_block(block)?;
            return Err(ChainError::ParentNotFound);
        };

        let (mut vm, logger) = if self.options.precompute_witnesses && self.is_synced() {
            // If witness pre-generation is enabled, we wrap the db with a logger
            // to track state access (block hashes, storage keys, codes) during execution
            // avoiding the need to re-execute the block later.
            let vm_db: DynVmDatabase = Box::new(StoreVmDatabase::new(
                self.storage.clone(),
                parent_header.clone(),
            )?);

            let logger = Arc::new(DatabaseLogger::new(Arc::new(vm_db)));

            let vm = match self.options.r#type.clone() {
                BlockchainType::L1 => Evm::new_from_db_for_l1(logger.clone()),
                BlockchainType::L2(l2_config) => Evm::new_from_db_for_l2(
                    logger.clone(),
                    *l2_config.fee_config.read().map_err(|_| {
                        EvmError::Custom("Fee config lock was poisoned".to_string())
                    })?,
                ),
            };
            (vm, Some(logger))
        } else {
            let vm_db = StoreVmDatabase::new(self.storage.clone(), parent_header.clone())?;
            let vm = self.new_evm(vm_db)?;
            (vm, None)
        };

        let (
            res,
            account_updates_list,
            accumulated_updates,
            merkle_queue_length,
            instants,
            warmer_duration,
        ) = self.execute_block_pipeline(&block, &parent_header, &mut vm)?;

        let (gas_used, gas_limit, block_number, transactions_count) = (
            block.header.gas_used,
            block.header.gas_limit,
            block.header.number,
            block.body.transactions.len(),
        );

        if let Some(logger) = logger
            && let Some(account_updates) = accumulated_updates
        {
            let block_hash = block.hash();
            let witness = self.generate_witness_from_account_updates(
                account_updates,
                &block,
                parent_header,
                &logger,
            )?;
            self.storage
                .store_witness(block_hash, block_number, witness)?;
        };

        let result = self.store_block(block, account_updates_list, res);

        let stored = Instant::now();

        let instants = std::array::from_fn(move |i| {
            if i < instants.len() {
                instants[i]
            } else {
                stored
            }
        });

        let metrics = Self::build_add_block_pipeline_metrics(
            gas_used,
            gas_limit,
            block_number,
            transactions_count,
            merkle_queue_length,
            warmer_duration,
            instants,
        );

        if self.options.perf_logs_enabled {
            Self::print_add_block_pipeline_logs(&metrics);
        }

        result?;
        Ok(metrics)
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
    fn build_add_block_pipeline_metrics(
        gas_used: u64,
        gas_limit: u64,
        block_number: u64,
        transactions_count: usize,
        merkle_queue_length: usize,
        warmer_duration: Duration,
        [
            start_instant,
            block_validated_instant,
            exec_merkle_start,
            exec_end_instant,
            merkle_end_instant,
            exec_merkle_end_instant,
            stored_instant,
        ]: [Instant; 7],
    ) -> AddBlockPipelineMetrics {
        let total_ms = stored_instant.duration_since(start_instant).as_millis() as u64;
        let throughput_ggas_per_s = if total_ms > 0 {
            (gas_used as f64 / 1e9) / (total_ms as f64 / 1000.0)
        } else {
            0.0
        };

        // Calculate phase durations in ms.
        let validate_ms = block_validated_instant
            .duration_since(start_instant)
            .as_millis() as u64;
        let exec_ms = exec_end_instant
            .duration_since(exec_merkle_start)
            .as_millis() as u64;
        let store_ms = stored_instant
            .duration_since(exec_merkle_end_instant)
            .as_millis() as u64;
        let warmer_ms = warmer_duration.as_millis() as u64;

        // merkle_end_instant marks when merkle thread finished (may be before or after exec)
        // exec_merkle_end_instant marks when both exec and merkle are done
        let merkle_total_ms = exec_merkle_end_instant
            .duration_since(exec_merkle_start)
            .as_millis() as u64;
        let merkle_concurrent_ms = (merkle_end_instant
            .duration_since(exec_merkle_start)
            .as_millis() as u64)
            .min(exec_ms);
        let merkle_drain_ms = exec_merkle_end_instant
            .saturating_duration_since(exec_end_instant)
            .as_millis() as u64;

        // Overlap percentage: how much of merkle work was done concurrently.
        let actual_merkle_ms = merkle_concurrent_ms + merkle_drain_ms;
        let merkle_overlap_pct = if actual_merkle_ms > 0 {
            (merkle_concurrent_ms * 100) / actual_merkle_ms
        } else {
            0
        };

        let warmer_early_ms = exec_ms as i64 - warmer_ms as i64;

        // Determine bottleneck by effective phase time.
        let phases = [
            ("validate", validate_ms),
            ("exec", exec_ms),
            ("merkle", merkle_drain_ms),
            ("store", store_ms),
        ];
        let bottleneck_phase = phases
            .iter()
            .max_by_key(|(_, ms)| ms)
            .map(|(name, _)| *name)
            .unwrap_or("exec");

        AddBlockPipelineMetrics {
            gas_used,
            gas_limit,
            block_number,
            transactions_count,
            merkle_queue_length,
            total_ms,
            throughput_ggas_per_s,
            validate_ms,
            exec_ms,
            merkle_concurrent_ms,
            merkle_drain_ms,
            merkle_total_ms,
            merkle_overlap_pct,
            store_ms,
            warmer_ms,
            warmer_early_ms,
            bottleneck_phase,
        }
    }

    fn print_add_block_pipeline_logs(metrics_data: &AddBlockPipelineMetrics) {
        if metrics_data.total_ms == 0 {
            return;
        }

        let as_mgas = metrics_data.gas_used as f64 / 1e6;
        let pct = |ms: u64| ((ms as f64 / metrics_data.total_ms as f64) * 100.0).round() as u64;

        let header = format!(
            "[METRIC] BLOCK {} | {:.3} Ggas/s | {} ms | {} txs | {:.0} Mgas ({}%)",
            metrics_data.block_number,
            metrics_data.throughput_ggas_per_s,
            metrics_data.total_ms,
            metrics_data.transactions_count,
            as_mgas,
            (metrics_data.gas_used as f64 / metrics_data.gas_limit as f64 * 100.0).round() as u64
        );

        let bottleneck_marker = |name: &str| {
            if name == metrics_data.bottleneck_phase {
                " << BOTTLENECK"
            } else {
                ""
            }
        };
        let warmer_relation = if metrics_data.warmer_early_ms >= 0 {
            "before exec"
        } else {
            "after exec"
        };

        info!("{}", header);
        info!(
            "  |- validate: {:>4} ms  ({:>2}%){}",
            metrics_data.validate_ms,
            pct(metrics_data.validate_ms),
            bottleneck_marker("validate")
        );
        info!(
            "  |- exec:     {:>4} ms  ({:>2}%){}",
            metrics_data.exec_ms,
            pct(metrics_data.exec_ms),
            bottleneck_marker("exec")
        );
        info!(
            "  |- merkle:   {:>4} ms  ({:>2}%){}  [concurrent: {} ms, drain: {} ms, overlap: {}%, queue: {}]",
            metrics_data.merkle_drain_ms,
            pct(metrics_data.merkle_drain_ms),
            bottleneck_marker("merkle"),
            metrics_data.merkle_concurrent_ms,
            metrics_data.merkle_drain_ms,
            metrics_data.merkle_overlap_pct,
            metrics_data.merkle_queue_length,
        );
        info!(
            "  |- store:    {:>4} ms  ({:>2}%){}",
            metrics_data.store_ms,
            pct(metrics_data.store_ms),
            bottleneck_marker("store")
        );
        info!(
            "  `- warmer:   {:>4} ms         [finished: {} ms {}]",
            metrics_data.warmer_ms,
            metrics_data.warmer_early_ms.unsigned_abs(),
            warmer_relation,
        );

        metrics!(
            METRICS_BLOCKS.set_block_number(metrics_data.block_number);
            METRICS_BLOCKS.set_latest_gas_used(metrics_data.gas_used as f64);
            METRICS_BLOCKS.set_latest_block_gas_limit(metrics_data.gas_limit as f64);
            METRICS_BLOCKS.set_latest_gigagas(metrics_data.throughput_ggas_per_s);
            METRICS_BLOCKS.set_transaction_count(metrics_data.transactions_count as i64);
            METRICS_BLOCKS.set_validate_ms(metrics_data.validate_ms as i64);
            METRICS_BLOCKS.set_execution_ms(metrics_data.exec_ms as i64);
            METRICS_BLOCKS.set_merkle_concurrent_ms(metrics_data.merkle_concurrent_ms as i64);
            METRICS_BLOCKS.set_merkle_drain_ms(metrics_data.merkle_drain_ms as i64);
            METRICS_BLOCKS.set_merkle_ms(metrics_data.merkle_total_ms as i64);
            METRICS_BLOCKS.set_merkle_overlap_pct(metrics_data.merkle_overlap_pct as i64);
            METRICS_BLOCKS.set_store_ms(metrics_data.store_ms as i64);
            METRICS_BLOCKS.set_warmer_ms(metrics_data.warmer_ms as i64);
            METRICS_BLOCKS.set_warmer_early_ms(metrics_data.warmer_early_ms);
        );
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

fn branchify(node: Node) -> Box<BranchNode> {
    match node {
        Node::Branch(branch_node) => branch_node,
        Node::Extension(extension_node) => {
            let index = extension_node.prefix.as_ref()[0];
            let noderef = if extension_node.prefix.len() == 1 {
                extension_node.child
            } else {
                let prefix = extension_node.prefix.offset(1);
                let node = ExtensionNode::new(prefix, extension_node.child);
                NodeRef::from(Arc::new(node.into()))
            };
            let mut choices = BranchNode::EMPTY_CHOICES;
            choices[index as usize] = noderef;
            Box::new(BranchNode::new(choices))
        }
        Node::Leaf(leaf_node) => {
            let index = leaf_node.partial.as_ref()[0];
            let node = LeafNode::new(leaf_node.partial.offset(1), leaf_node.value);
            let mut choices = BranchNode::EMPTY_CHOICES;
            choices[index as usize] = NodeRef::from(Arc::new(node.into()));
            Box::new(BranchNode::new(choices))
        }
    }
}

fn collect_trie(index: u8, mut trie: Trie) -> Result<(Box<BranchNode>, Vec<TrieNode>), TrieError> {
    let root = branchify(
        trie.root_node()?
            .map(Arc::unwrap_or_clone)
            .unwrap_or_else(|| Node::Branch(Box::default())),
    );
    trie.root = Node::Branch(root).into();
    let (_, mut nodes) = trie.collect_changes_since_last_hash();
    nodes.retain(|(nib, _)| nib.as_ref().first() == Some(&index));

    let Some(Node::Branch(root)) = trie.root_node()?.map(Arc::unwrap_or_clone) else {
        return Err(TrieError::InvalidInput);
    };
    Ok((root, nodes))
}
