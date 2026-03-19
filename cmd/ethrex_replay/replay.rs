use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
    time::Instant,
};

use anyhow::{Context, Result, anyhow};
use ethrex_binary_trie::state::BinaryTrieState;
use ethrex_blockchain::binary_trie_db::BinaryTrieVmDb;
use ethrex_common::{
    H256,
    types::{Block, BlockNumber, ChainConfig, Genesis},
    validation::{validate_gas_used, validate_receipts_root},
};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::Store;
use ethrex_vm::backends::Evm;
use tracing::{debug, error, info, warn};

pub struct BlockReplayer {
    /// Source of blocks (existing MPT node's DB).
    store: Store,
    /// Binary trie state (in-memory, or RocksDB-backed when a path is configured).
    state: Arc<RwLock<BinaryTrieState>>,
    /// Chain configuration (from genesis).
    chain_config: ChainConfig,
    /// Block hashes for BLOCKHASH opcode (last 256 entries).
    block_hashes: BTreeMap<BlockNumber, H256>,
    /// Flush binary trie state to disk every this many blocks (0 = never).
    checkpoint_interval: u64,
}

impl BlockReplayer {
    /// Create a new replayer from a genesis JSON and an existing store.
    ///
    /// When `trie_db_path` is `Some`, the binary trie state is opened from (or
    /// created at) that RocksDB path.  If a checkpoint exists in the database,
    /// replay resumes from the block after the checkpoint; block_hashes for the
    /// last 256 blocks are reconstructed from the store.  If no checkpoint
    /// exists, genesis allocations are applied and replay starts from block 1.
    ///
    /// When `trie_db_path` is `None`, state is kept in memory only (original
    /// behaviour).
    pub async fn new(
        genesis: Genesis,
        store: Store,
        trie_db_path: Option<&std::path::Path>,
        checkpoint_interval: u64,
    ) -> Result<Self> {
        let chain_config = genesis.config;

        // Register genesis block hash so that block 1 can call BLOCKHASH(0).
        let genesis_block = genesis.get_block();
        let genesis_hash = genesis_block.hash();

        let (state, block_hashes) = if let Some(path) = trie_db_path {
            info!("Opening binary trie DB at {}", path.display());
            let mut state =
                BinaryTrieState::open(path).context("Failed to open binary trie RocksDB")?;

            let mut block_hashes = BTreeMap::new();
            block_hashes.insert(0u64, genesis_hash);

            if state.has_data() {
                // Resume: reconstruct block_hashes from the store for the last
                // 256 blocks before the checkpoint so BLOCKHASH works correctly.
                let checkpoint = state.checkpoint_block().unwrap_or(0);
                info!("Resuming from checkpoint at block {checkpoint}");

                let hash_start = checkpoint.saturating_sub(256);
                block_hashes = reconstruct_block_hashes(&store, hash_start, checkpoint).await?;
                // Always include genesis hash.
                block_hashes.entry(0).or_insert(genesis_hash);
            } else {
                // New database — apply genesis.
                info!("No checkpoint found; applying genesis");
                state
                    .apply_genesis(&genesis.alloc)
                    .context("Failed to apply genesis to BinaryTrieState")?;
                let genesis_root = state.state_root();
                info!("Genesis binary trie root: 0x{}", hex::encode(genesis_root));
            }

            (state, block_hashes)
        } else {
            // In-memory path (original behaviour).
            let mut state = BinaryTrieState::new();
            state
                .apply_genesis(&genesis.alloc)
                .context("Failed to apply genesis to BinaryTrieState")?;

            let genesis_root = state.state_root();
            info!("Genesis binary trie root: 0x{}", hex::encode(genesis_root));

            let mut block_hashes = BTreeMap::new();
            block_hashes.insert(0u64, genesis_hash);

            (state, block_hashes)
        };

        Ok(Self {
            store,
            state: Arc::new(RwLock::new(state)),
            chain_config,
            block_hashes,
            checkpoint_interval,
        })
    }

    /// Returns the block number to start replaying from.
    ///
    /// If a checkpoint is recorded in the state, resume from the next block;
    /// otherwise return `requested_start`.
    pub fn effective_start(&self, requested_start: BlockNumber) -> BlockNumber {
        let checkpoint = self.state.read().ok().and_then(|s| s.checkpoint_block());
        match checkpoint {
            Some(n) if n >= requested_start => {
                let resume = n + 1;
                info!("Checkpoint found at block {n}; resuming from block {resume}");
                resume
            }
            _ => requested_start,
        }
    }

    /// Replay blocks from `start` to `end` (inclusive).
    ///
    /// If a checkpoint is recorded in the state and it falls within the
    /// requested range, replay automatically resumes from the block after the
    /// checkpoint.
    ///
    /// Logs the binary trie root every `log_interval` blocks and on the final
    /// block.  Flushes state to disk every `checkpoint_interval` blocks when a
    /// trie DB path was configured.
    pub async fn replay(
        &mut self,
        start: BlockNumber,
        end: BlockNumber,
        log_interval: u64,
    ) -> Result<()> {
        let effective_start = self.effective_start(start);

        if effective_start > end {
            info!(
                "Nothing to replay: checkpoint ({}) is already at or beyond end block ({end})",
                effective_start - 1
            );
            return Ok(());
        }

        let start_time = Instant::now();

        for block_number in effective_start..=end {
            let block = self
                .store
                .get_block_by_number(block_number)
                .await
                .with_context(|| format!("Store error reading block {block_number}"))?
                .ok_or_else(|| anyhow!("block {block_number} not found in store"))?;

            let root = match self.execute_block(&block) {
                Ok(root) => root,
                Err(e) => {
                    error!("Failed to execute block {block_number}: {e:#}");
                    return Err(e);
                }
            };

            // Register this block's hash for future BLOCKHASH lookups.
            let block_hash = block.hash();
            self.register_block_hash(block_number, block_hash);

            debug!("Block {block_number}: root=0x{}", hex::encode(root));

            if block_number % log_interval == 0 || block_number == end {
                let elapsed = start_time.elapsed().as_secs_f64();
                let blocks_done = block_number - effective_start + 1;
                let bps = blocks_done as f64 / elapsed;
                info!(
                    "Block {block_number}: root=0x{} ({:.1} blocks/sec)",
                    hex::encode(root),
                    bps
                );
            }

            // Flush checkpoint if configured.
            if self.checkpoint_interval > 0 && block_number % self.checkpoint_interval == 0 {
                if let Err(e) = self.flush_checkpoint(block_number) {
                    warn!("Checkpoint flush failed at block {block_number}: {e:#}");
                }
            }
        }

        // Final flush to capture the last partial interval.
        if self.checkpoint_interval > 0 && end % self.checkpoint_interval != 0 {
            if let Err(e) = self.flush_checkpoint(end) {
                warn!("Final checkpoint flush failed: {e:#}");
            }
        }

        let elapsed = start_time.elapsed().as_secs_f64();
        let total = end - effective_start + 1;
        info!(
            "Replay complete: {total} blocks in {elapsed:.1}s ({:.1} blocks/sec)",
            total as f64 / elapsed
        );

        Ok(())
    }

    /// Flush the binary trie state to disk, recording `block_number` as the checkpoint.
    ///
    /// This is a no-op when the state is not backed by a RocksDB database.
    fn flush_checkpoint(&self, block_number: BlockNumber) -> Result<()> {
        let mut state = self
            .state
            .write()
            .map_err(|e| anyhow!("state RwLock poisoned: {e}"))?;
        state
            .flush(block_number)
            .with_context(|| format!("Failed to flush checkpoint at block {block_number}"))?;
        info!("Checkpoint saved at block {block_number}");
        Ok(())
    }

    /// Execute a single block against the binary trie state and apply the resulting
    /// account updates. Returns the new binary trie state root.
    fn execute_block(&mut self, block: &Block) -> Result<[u8; 32]> {
        // Build the VmDb adapter backed by the current binary trie state.
        let vm_db = BinaryTrieVmDb::new(self.state.clone(), self.chain_config);
        vm_db.add_block_hashes(self.block_hashes.iter().map(|(&n, &h)| (n, h)));

        // Create the EVM and execute the block.
        let mut evm = Evm::new_for_l1(vm_db, Arc::new(NativeCrypto));
        let (execution_result, _bal) = evm.execute_block(block).with_context(|| {
            format!("EVM execute_block failed for block {}", block.header.number)
        })?;

        // Validate execution results against block header.
        validate_gas_used(execution_result.block_gas_used, &block.header).with_context(|| {
            format!(
                "gas mismatch at block {}: got {}, expected {}",
                block.header.number, execution_result.block_gas_used, block.header.gas_used
            )
        })?;
        validate_receipts_root(&block.header, &execution_result.receipts)
            .with_context(|| format!("receipts root mismatch at block {}", block.header.number))?;

        // Collect state changes and apply them to the binary trie.
        let account_updates = evm
            .get_state_transitions()
            .context("get_state_transitions failed")?;

        {
            let mut state = self
                .state
                .write()
                .map_err(|e| anyhow!("state RwLock poisoned: {e}"))?;
            for update in &account_updates {
                state.apply_account_update(update).with_context(|| {
                    format!(
                        "apply_account_update failed for {:?} in block {}",
                        update.address, block.header.number
                    )
                })?;
            }
        }

        let root = self
            .state
            .write()
            .map_err(|e| anyhow!("state RwLock poisoned: {e}"))?
            .state_root();

        Ok(root)
    }

    /// Register a block hash for the BLOCKHASH opcode and prune entries older
    /// than 256 blocks.
    fn register_block_hash(&mut self, number: BlockNumber, hash: H256) {
        self.block_hashes.insert(number, hash);
        // Keep only the last 256 hashes (BLOCKHASH only looks back 256 blocks).
        if number > 256 {
            self.block_hashes = self.block_hashes.split_off(&(number - 256));
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Fetch block hashes for `start..=end` from the store.
///
/// Used on resume to seed the `block_hashes` map with the last 256 entries
/// preceding the checkpoint so BLOCKHASH works correctly for the first block
/// after resume.
async fn reconstruct_block_hashes(
    store: &Store,
    start: BlockNumber,
    end: BlockNumber,
) -> Result<BTreeMap<BlockNumber, H256>> {
    let mut map = BTreeMap::new();
    for n in start..=end {
        match store.get_block_by_number(n).await {
            Ok(Some(block)) => {
                map.insert(n, block.hash());
            }
            Ok(None) => {
                // Block not in store (e.g. before the store's earliest block).
                // This is acceptable — BLOCKHASH will return zero for missing entries.
            }
            Err(e) => {
                return Err(anyhow!(
                    "store error reading block {n} for hash reconstruction: {e}"
                ));
            }
        }
    }
    Ok(map)
}
