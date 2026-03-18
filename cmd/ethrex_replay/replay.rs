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
use tracing::{debug, error, info};

pub struct BlockReplayer {
    /// Source of blocks (existing MPT node's DB).
    store: Store,
    /// Binary trie state (in-memory).
    state: Arc<RwLock<BinaryTrieState>>,
    /// Chain configuration (from genesis).
    chain_config: ChainConfig,
    /// Block hashes for BLOCKHASH opcode (last 256 entries).
    block_hashes: BTreeMap<BlockNumber, H256>,
}

impl BlockReplayer {
    /// Create a new replayer from a genesis JSON and an existing store.
    ///
    /// Applies genesis allocations to a fresh `BinaryTrieState` and registers
    /// the genesis block hash so that block 1 can call BLOCKHASH(0).
    pub fn new(genesis: Genesis, store: Store) -> Result<Self> {
        let chain_config = genesis.config;

        let mut state = BinaryTrieState::new();
        state
            .apply_genesis(&genesis.alloc)
            .context("Failed to apply genesis to BinaryTrieState")?;

        let genesis_root = state.state_root();
        info!("Genesis binary trie root: 0x{}", hex::encode(genesis_root));

        // Register genesis block hash so block 1 can call BLOCKHASH(0).
        let genesis_block = genesis.get_block();
        let genesis_hash = genesis_block.hash();
        let mut block_hashes = BTreeMap::new();
        block_hashes.insert(0u64, genesis_hash);

        Ok(Self {
            store,
            state: Arc::new(RwLock::new(state)),
            chain_config,
            block_hashes,
        })
    }

    /// Replay blocks from `start` to `end` (inclusive).
    ///
    /// Logs the binary trie root every `log_interval` blocks and on the final block.
    pub async fn replay(
        &mut self,
        start: BlockNumber,
        end: BlockNumber,
        log_interval: u64,
    ) -> Result<()> {
        let start_time = Instant::now();

        for block_number in start..=end {
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
                let blocks_done = block_number - start + 1;
                let bps = blocks_done as f64 / elapsed;
                info!(
                    "Block {block_number}: root=0x{} ({:.1} blocks/sec)",
                    hex::encode(root),
                    bps
                );
            }
        }

        let elapsed = start_time.elapsed().as_secs_f64();
        let total = end - start + 1;
        info!(
            "Replay complete: {total} blocks in {elapsed:.1}s ({:.1} blocks/sec)",
            total as f64 / elapsed
        );

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
