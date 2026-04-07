use std::collections::BTreeMap;
use std::path::Path;

use ethereum_types::{Address, H256, U256};
use ethrex_common::types::{
    AccountState, Block, BlockBody, BlockHash, BlockHeader, BlockNumber, Receipt, Transaction,
    TxType,
};
use ethrex_storage::{EngineType, Store};

pub use error::ExplorerError;

mod error;

/// Transaction location: (block_number, tx_index)
pub type TxLocation = (BlockNumber, usize);

/// Read-only explorer for an ethrex mainnet database.
pub struct DbExplorer {
    store: Store,
    rt: tokio::runtime::Runtime,
}

impl DbExplorer {
    /// Open an existing database at the given path (read-only usage).
    ///
    /// The database must already exist and have a valid schema.
    pub fn open(datadir: impl AsRef<Path>) -> Result<Self, ExplorerError> {
        let store = Store::new(datadir, EngineType::RocksDB)?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ExplorerError::Runtime(e.to_string()))?;
        // Populate the latest block header cache from the DB.
        rt.block_on(store.load_initial_state())?;
        Ok(Self { store, rt })
    }

    /// The underlying store, for advanced usage.
    pub fn store(&self) -> &Store {
        &self.store
    }

    // ── Chain metadata ───────────────────────────────────────────────

    /// Latest block number in the database.
    pub fn latest_block_number(&self) -> Result<BlockNumber, ExplorerError> {
        Ok(self.rt.block_on(self.store.get_latest_block_number())?)
    }

    /// Earliest block number in the database.
    pub fn earliest_block_number(&self) -> Result<BlockNumber, ExplorerError> {
        Ok(self.rt.block_on(self.store.get_earliest_block_number())?)
    }

    /// Canonical block hash for a given block number.
    pub fn canonical_hash(&self, number: BlockNumber) -> Result<Option<BlockHash>, ExplorerError> {
        Ok(self.store.get_canonical_block_hash_sync(number)?)
    }

    // ── Single-item lookups ──────────────────────────────────────────

    /// Get a block header by number.
    pub fn header(&self, number: BlockNumber) -> Result<Option<BlockHeader>, ExplorerError> {
        Ok(self.store.get_block_header(number)?)
    }

    /// Get a block body by number.
    pub fn body(&self, number: BlockNumber) -> Result<Option<BlockBody>, ExplorerError> {
        Ok(self.rt.block_on(self.store.get_block_body(number))?)
    }

    /// Get a full block by number.
    pub fn block(&self, number: BlockNumber) -> Result<Option<Block>, ExplorerError> {
        Ok(self.rt.block_on(self.store.get_block_by_number(number))?)
    }

    /// Get a full block by hash.
    pub fn block_by_hash(&self, hash: BlockHash) -> Result<Option<Block>, ExplorerError> {
        Ok(self.rt.block_on(self.store.get_block_by_hash(hash))?)
    }

    /// Get a single transaction by hash.
    pub fn transaction_by_hash(
        &self,
        hash: H256,
    ) -> Result<Option<Transaction>, ExplorerError> {
        Ok(self
            .rt
            .block_on(self.store.get_transaction_by_hash(hash))?)
    }

    /// Get all receipts for a block (by hash).
    pub fn receipts(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, ExplorerError> {
        Ok(self
            .rt
            .block_on(async { self.store.get_receipts_for_block(block_hash).await })?)
    }

    // ── State queries (recent state) ─────────────────────────────────

    /// Get account state at a given block number.
    pub fn account_state(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<AccountState>, ExplorerError> {
        Ok(self
            .rt
            .block_on(self.store.get_account_state(block_number, address))?)
    }

    /// Get a storage slot value at a given block number.
    pub fn storage_at(
        &self,
        block_number: BlockNumber,
        address: Address,
        slot: H256,
    ) -> Result<Option<U256>, ExplorerError> {
        Ok(self
            .store
            .get_storage_at(block_number, address, slot)?)
    }

    /// Get contract code at a given block number.
    pub fn code(
        &self,
        block_number: BlockNumber,
        address: Address,
    ) -> Result<Option<Vec<u8>>, ExplorerError> {
        let code = self
            .rt
            .block_on(self.store.get_code_by_account_address(block_number, address))?;
        Ok(code.map(|c| c.bytecode.to_vec()))
    }

    // ── Range iterators ──────────────────────────────────────────────

    /// Iterate block headers over a range of block numbers.
    /// Missing headers are silently skipped.
    pub fn headers<'a>(
        &'a self,
        range: impl Iterator<Item = BlockNumber> + 'a,
    ) -> impl Iterator<Item = (BlockNumber, BlockHeader)> + 'a {
        range.filter_map(|n| {
            self.store
                .get_block_header(n)
                .ok()
                .flatten()
                .map(|h| (n, h))
        })
    }

    /// Iterate full blocks over a range of block numbers.
    /// Missing blocks are silently skipped.
    pub fn blocks<'a>(
        &'a self,
        range: impl Iterator<Item = BlockNumber> + 'a,
    ) -> impl Iterator<Item = Block> + 'a {
        range.filter_map(|n| {
            self.rt
                .block_on(self.store.get_block_by_number(n))
                .ok()
                .flatten()
        })
    }

    /// Iterate all transactions in a range of block numbers,
    /// yielding `(block_number, tx_index, transaction)`.
    pub fn transactions<'a>(
        &'a self,
        range: impl Iterator<Item = BlockNumber> + 'a,
    ) -> impl Iterator<Item = (BlockNumber, usize, Transaction)> + 'a {
        range.flat_map(|n| {
            let body = self
                .rt
                .block_on(self.store.get_block_body(n))
                .ok()
                .flatten();
            let txs = body
                .map(|b| b.transactions)
                .unwrap_or_default();
            txs.into_iter()
                .enumerate()
                .map(move |(i, tx)| (n, i, tx))
        })
    }

    // ── Search helpers ───────────────────────────────────────────────

    /// Find blocks in a range matching a predicate.
    pub fn find_blocks<'a>(
        &'a self,
        range: impl Iterator<Item = BlockNumber> + 'a,
        predicate: impl Fn(&Block) -> bool + 'a,
    ) -> impl Iterator<Item = Block> + 'a {
        self.blocks(range).filter(move |b| predicate(b))
    }

    /// Find transactions in a range matching a predicate.
    pub fn find_transactions<'a>(
        &'a self,
        range: impl Iterator<Item = BlockNumber> + 'a,
        predicate: impl Fn(&Transaction) -> bool + 'a,
    ) -> impl Iterator<Item = (BlockNumber, usize, Transaction)> + 'a {
        self.transactions(range).filter(move |(_, _, tx)| predicate(tx))
    }

    // ── Transaction statistics ───────────────────────────────────────

    /// Compute transaction statistics for a single block.
    pub fn block_tx_stats(&self, number: BlockNumber) -> Result<Option<BlockTxStats>, ExplorerError> {
        let block = match self.block(number)? {
            Some(b) => b,
            None => return Ok(None),
        };

        let block_hash = match self.canonical_hash(number)? {
            Some(h) => h,
            None => return Ok(None),
        };

        let receipts = self.receipts(&block_hash)?;
        let txs = &block.body.transactions;

        let mut stats = BlockTxStats {
            block_number: number,
            total_txs: txs.len(),
            gas_used: block.header.gas_used,
            gas_limit: block.header.gas_limit,
            ..Default::default()
        };

        for tx in txs {
            match tx.tx_type() {
                TxType::Legacy => stats.legacy_count += 1,
                TxType::EIP2930 => stats.eip2930_count += 1,
                TxType::EIP1559 => stats.eip1559_count += 1,
                TxType::EIP4844 => stats.eip4844_count += 1,
                TxType::EIP7702 => stats.eip7702_count += 1,
                _ => stats.other_count += 1,
            }

            stats.total_value = stats.total_value.saturating_add(tx.value());
        }

        // Per-tx gas from receipts (cumulative_gas_used diffs)
        let mut prev_cumulative = 0u64;
        for receipt in &receipts {
            let gas = receipt.cumulative_gas_used.saturating_sub(prev_cumulative);
            prev_cumulative = receipt.cumulative_gas_used;
            stats.total_log_count += receipt.logs.len();
            if !receipt.succeeded {
                stats.failed_tx_count += 1;
            }
            stats.per_tx_gas.push(gas);
        }

        Ok(Some(stats))
    }

    /// Compute aggregated transaction statistics over a range of blocks.
    pub fn range_tx_stats(
        &self,
        range: impl Iterator<Item = BlockNumber>,
    ) -> Result<RangeTxStats, ExplorerError> {
        let mut agg = RangeTxStats::default();

        for n in range {
            if let Some(stats) = self.block_tx_stats(n)? {
                agg.blocks_scanned += 1;
                agg.total_txs += stats.total_txs;
                agg.total_gas_used += stats.gas_used;
                agg.legacy_count += stats.legacy_count;
                agg.eip2930_count += stats.eip2930_count;
                agg.eip1559_count += stats.eip1559_count;
                agg.eip4844_count += stats.eip4844_count;
                agg.eip7702_count += stats.eip7702_count;
                agg.other_count += stats.other_count;
                agg.failed_tx_count += stats.failed_tx_count;
                agg.total_log_count += stats.total_log_count;
            }
        }

        Ok(agg)
    }

    // ── State statistics ─────────────────────────────────────────────

    /// Get the state root for a given block number.
    pub fn state_root(&self, block_number: BlockNumber) -> Result<Option<H256>, ExplorerError> {
        Ok(self.header(block_number)?.map(|h| h.state_root))
    }

    /// Check whether the state trie for a given root is available in the DB.
    pub fn has_state_root(&self, state_root: H256) -> Result<bool, ExplorerError> {
        Ok(self.store.has_state_root(state_root)?)
    }

    /// Count the number of storage slots for a single account via FKV prefix scan.
    ///
    /// Warning: this can be very slow for large contracts (millions of slots).
    pub fn storage_slot_count(
        &self,
        hashed_address: H256,
    ) -> Result<u64, ExplorerError> {
        Ok(self.store.count_fkv_storage_slots(hashed_address)? as u64)
    }

    /// Compute state statistics at a given block number.
    ///
    /// `count_slots`: if true, counts storage slots per account (VERY slow on
    /// mainnet — can take hours even for small samples). If false, only counts
    /// accounts and their properties (fast).
    ///
    /// `max_accounts`: stop after this many accounts (sampling).
    /// `report_every`: call `progress` every N accounts.
    pub fn state_stats(
        &self,
        block_number: BlockNumber,
        count_slots: bool,
        max_accounts: Option<usize>,
        report_every: usize,
        progress: impl Fn(&StateStats),
    ) -> Result<StateStats, ExplorerError> {
        let state_root = self
            .state_root(block_number)?
            .ok_or_else(|| ExplorerError::Runtime(format!("Block {block_number} not found")))?;

        let mut stats = StateStats {
            block_number,
            state_root,
            ..Default::default()
        };

        let empty_code_hash: H256 =
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
                .parse()
                .expect("valid hex");

        let fkv_iter = self.store.iter_fkv_accounts()?;

        fkv_iter.for_each(|hashed_addr, account| {
            stats.total_accounts += 1;

            let has_storage = account.storage_root != H256::from_low_u64_be(0)
                && account.storage_root
                    != "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"
                        .parse::<H256>()
                        .expect("valid hex");

            if has_storage {
                stats.accounts_with_storage += 1;
            }

            if count_slots && has_storage {
                let slot_count = self.storage_slot_count(hashed_addr).unwrap_or(0);
                stats.total_slots += slot_count;

                let bucket = slot_count_bucket(slot_count);
                *stats.slot_histogram.entry(bucket).or_insert(0) += 1;

                // Track top-N accounts by storage size
                if stats.top_accounts_by_slots.len() < 100
                    || stats
                        .top_accounts_by_slots
                        .last()
                        .is_some_and(|(_, c)| slot_count > *c)
                {
                    stats.top_accounts_by_slots.push((hashed_addr, slot_count));
                    stats
                        .top_accounts_by_slots
                        .sort_by(|a, b| b.1.cmp(&a.1));
                    stats.top_accounts_by_slots.truncate(100);
                }
            }

            let is_contract = account.code_hash != H256::from_low_u64_be(0)
                && account.code_hash != empty_code_hash;
            if is_contract {
                stats.contract_accounts += 1;
            }

            if account.balance > U256::zero() {
                stats.accounts_with_balance += 1;
            }

            if stats.total_accounts % report_every == 0 {
                progress(&stats);
            }

            if max_accounts.is_some_and(|max| stats.total_accounts >= max) {
                stats.sampled = true;
                return false;
            }
            true
        })?;

        progress(&stats);
        Ok(stats)
    }
}

/// Map a slot count to a histogram bucket label.
fn slot_count_bucket(count: u64) -> &'static str {
    match count {
        0 => "0",
        1..=10 => "1-10",
        11..=100 => "11-100",
        101..=1_000 => "101-1K",
        1_001..=10_000 => "1K-10K",
        10_001..=100_000 => "10K-100K",
        _ => "100K+",
    }
}

/// State-level statistics at a point in time.
#[derive(Debug, Default)]
pub struct StateStats {
    pub block_number: BlockNumber,
    pub state_root: H256,
    pub total_accounts: usize,
    pub contract_accounts: usize,
    pub accounts_with_balance: usize,
    pub accounts_with_storage: usize,
    pub total_slots: u64,
    /// Whether these stats come from a sampled (truncated) run.
    pub sampled: bool,
    /// Histogram: bucket label -> account count.
    pub slot_histogram: BTreeMap<&'static str, usize>,
    /// Top 100 accounts by storage slot count: (hashed_address, slot_count).
    pub top_accounts_by_slots: Vec<(H256, u64)>,
}

impl std::fmt::Display for StateStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "State stats at block #{} {}",
            self.block_number,
            if self.sampled { "(sampled)" } else { "" }
        )?;
        writeln!(f, "  State root:     {:?}", self.state_root)?;
        writeln!(f, "  Total accounts: {}", self.total_accounts)?;
        writeln!(f, "  Contracts:      {}", self.contract_accounts)?;
        writeln!(f, "  With balance:   {}", self.accounts_with_balance)?;
        writeln!(f, "  With storage:   {}", self.accounts_with_storage)?;
        writeln!(f, "  Total slots:    {}", self.total_slots)?;
        if self.total_accounts > 0 {
            writeln!(
                f,
                "  Avg slots/account: {:.2}",
                self.total_slots as f64 / self.total_accounts as f64
            )?;
        }
        writeln!(f, "  Slot distribution (accounts per bucket):")?;
        let bucket_order = ["0", "1-10", "11-100", "101-1K", "1K-10K", "10K-100K", "100K+"];
        for bucket in bucket_order {
            let count = self.slot_histogram.get(bucket).copied().unwrap_or(0);
            if count > 0 {
                writeln!(f, "    {:>8} slots: {} accounts", bucket, count)?;
            }
        }
        if !self.top_accounts_by_slots.is_empty() {
            writeln!(f, "  Top accounts by storage:")?;
            for (i, (hash, count)) in self.top_accounts_by_slots.iter().take(20).enumerate() {
                writeln!(f, "    {:>3}. {:?}  {} slots", i + 1, hash, count)?;
            }
        }
        Ok(())
    }
}

/// Transaction statistics for a single block.
#[derive(Debug, Default)]
pub struct BlockTxStats {
    pub block_number: BlockNumber,
    pub total_txs: usize,
    pub gas_used: u64,
    pub gas_limit: u64,
    pub legacy_count: usize,
    pub eip2930_count: usize,
    pub eip1559_count: usize,
    pub eip4844_count: usize,
    pub eip7702_count: usize,
    pub other_count: usize,
    pub failed_tx_count: usize,
    pub total_log_count: usize,
    pub total_value: U256,
    /// Gas used per transaction (derived from receipt cumulative diffs).
    pub per_tx_gas: Vec<u64>,
}

/// Aggregated transaction statistics over a range of blocks.
#[derive(Debug, Default)]
pub struct RangeTxStats {
    pub blocks_scanned: usize,
    pub total_txs: usize,
    pub total_gas_used: u64,
    pub legacy_count: usize,
    pub eip2930_count: usize,
    pub eip1559_count: usize,
    pub eip4844_count: usize,
    pub eip7702_count: usize,
    pub other_count: usize,
    pub failed_tx_count: usize,
    pub total_log_count: usize,
}

impl std::fmt::Display for BlockTxStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Block #{}", self.block_number)?;
        writeln!(
            f,
            "  Transactions: {} ({} failed)",
            self.total_txs, self.failed_tx_count
        )?;
        writeln!(
            f,
            "  Gas: {}/{} ({:.1}% full)",
            self.gas_used,
            self.gas_limit,
            if self.gas_limit > 0 {
                self.gas_used as f64 / self.gas_limit as f64 * 100.0
            } else {
                0.0
            }
        )?;
        writeln!(
            f,
            "  Types: legacy={} 2930={} 1559={} 4844={} 7702={} other={}",
            self.legacy_count,
            self.eip2930_count,
            self.eip1559_count,
            self.eip4844_count,
            self.eip7702_count,
            self.other_count,
        )?;
        writeln!(f, "  Logs emitted: {}", self.total_log_count)?;
        writeln!(f, "  Total value transferred: {}", self.total_value)?;
        Ok(())
    }
}

impl std::fmt::Display for RangeTxStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Range stats ({} blocks)", self.blocks_scanned)?;
        writeln!(
            f,
            "  Transactions: {} ({} failed)",
            self.total_txs, self.failed_tx_count
        )?;
        writeln!(f, "  Total gas used: {}", self.total_gas_used)?;
        writeln!(
            f,
            "  Types: legacy={} 2930={} 1559={} 4844={} 7702={} other={}",
            self.legacy_count,
            self.eip2930_count,
            self.eip1559_count,
            self.eip4844_count,
            self.eip7702_count,
            self.other_count,
        )?;
        writeln!(f, "  Total logs: {}", self.total_log_count)?;
        if self.blocks_scanned > 0 {
            writeln!(
                f,
                "  Avg txs/block: {:.1}",
                self.total_txs as f64 / self.blocks_scanned as f64
            )?;
            writeln!(
                f,
                "  Avg gas/block: {:.0}",
                self.total_gas_used as f64 / self.blocks_scanned as f64
            )?;
        }
        Ok(())
    }
}
