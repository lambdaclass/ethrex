use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap, hash_map::Entry},
    sync::RwLock,
};

use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    constants::{
        TX_ACCESS_LIST_ADDRESS_GAS, TX_ACCESS_LIST_STORAGE_KEY_GAS, TX_CREATE_GAS_COST,
        TX_DATA_NON_ZERO_GAS, TX_DATA_NON_ZERO_GAS_EIP2028, TX_DATA_ZERO_GAS_COST, TX_GAS_COST,
        TX_INIT_CODE_WORD_GAS_COST,
    },
    error::MempoolError,
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        BlobTuple, BlobsBundle, BlockHeader, ChainConfig, MempoolTransaction, Transaction, TxType,
        kzg_commitment_to_versioned_hash,
    },
};
use ethrex_storage::error::StoreError;
use tracing::warn;

/// Numerator of the heap-size pruning factor. The eviction heap is rebuilt
/// from the live `transaction_pool` once it grows past this multiple of
/// `max_mempool_size`. The factor is expressed as
/// `MEMPOOL_PRUNE_THRESHOLD_NUM / MEMPOOL_PRUNE_THRESHOLD_DEN` so the integer
/// arithmetic stays exact and no magic floats appear in the code.
const MEMPOOL_PRUNE_THRESHOLD_NUM: usize = 3;
/// Denominator of the heap-size pruning factor. Together with
/// `MEMPOOL_PRUNE_THRESHOLD_NUM` it encodes the 1.5x threshold.
const MEMPOOL_PRUNE_THRESHOLD_DEN: usize = 2;

#[derive(Debug, Default)]
struct MempoolInner {
    broadcast_pool: FxHashSet<H256>,
    transaction_pool: FxHashMap<H256, MempoolTransaction>,
    blobs_bundle_pool: FxHashMap<H256, BlobsBundle>,
    /// Transaction hashes that have been requested via GetPooledTransactions
    /// but whose responses haven't arrived yet. Used to avoid sending duplicate
    /// requests when multiple peers announce the same transaction.
    in_flight_txs: FxHashSet<H256>,
    /// Maps blob versioned hashes to transaction hashes that include them and a position inside
    /// blob bundle where blob and its adjacent data is available.
    blobs_bundle_by_versioned_hash: FxHashMap<H256, FxHashMap<H256, usize>>,
    txs_by_sender_nonce: BTreeMap<(H160, u64), H256>,
    /// Min-heap (via `Reverse`) of `(gas_tip_cap, hash)` used to pick the
    /// lowest-tip-cap transaction to evict when the mempool is full. The
    /// key is the raw `Transaction::gas_tip_cap()` projected to `u64`, NOT
    /// the base-fee-adjusted effective tip — admission decisions stay stable
    /// as base fee oscillates. Entries are removed lazily — a popped hash
    /// that is no longer in `transaction_pool` is treated as a tombstone
    /// and skipped.
    txs_by_tip: BinaryHeap<Reverse<(u64, H256)>>,
    max_mempool_size: usize,
    /// Max number of entries to let the eviction heap grow before rebuilding
    /// it from `transaction_pool` to drop tombstones.
    mempool_prune_threshold: usize,
}

impl MempoolInner {
    fn new(max_mempool_size: usize) -> Self {
        MempoolInner {
            txs_by_tip: BinaryHeap::with_capacity(max_mempool_size * 2),
            transaction_pool: FxHashMap::with_capacity_and_hasher(
                max_mempool_size,
                Default::default(),
            ),
            max_mempool_size,
            mempool_prune_threshold: max_mempool_size * MEMPOOL_PRUNE_THRESHOLD_NUM
                / MEMPOOL_PRUNE_THRESHOLD_DEN,
            ..Default::default()
        }
    }

    /// Remove a transaction from the pool with the transaction pool lock already taken
    fn remove_transaction_with_lock(&mut self, hash: &H256) -> Result<(), StoreError> {
        let Some(tx) = self.transaction_pool.remove(hash) else {
            return Ok(());
        };
        if matches!(tx.tx_type(), TxType::EIP4844) {
            self.remove_blob_bundle(hash);
        }

        self.txs_by_sender_nonce.remove(&(tx.sender(), tx.nonce()));
        self.broadcast_pool.remove(hash);

        Ok(())
    }

    /// Remove a blobs bundle from the pool
    pub fn remove_blob_bundle(&mut self, hash: &H256) {
        let Some(h) = self.blobs_bundle_pool.remove(hash) else {
            return;
        };

        for commitment in &h.commitments {
            let versioned_hash = kzg_commitment_to_versioned_hash(commitment);
            if let Entry::Occupied(mut entry) =
                self.blobs_bundle_by_versioned_hash.entry(versioned_hash)
            {
                let txn_to_bundle = entry.get_mut();
                txn_to_bundle.remove(hash);
                if txn_to_bundle.is_empty() {
                    entry.remove();
                }
            }
        }
    }

    /// Evict the lowest-tip transaction(s) from the pool until it is below
    /// `max_mempool_size`. Uses lazy deletion: heap entries whose hash is no
    /// longer in `transaction_pool` are skipped without rebuilding the heap.
    fn evict_lowest_tip_transaction(&mut self) -> Result<(), StoreError> {
        while self.transaction_pool.len() >= self.max_mempool_size {
            let Some(Reverse((_tip, hash))) = self.txs_by_tip.pop() else {
                warn!(
                    "Mempool is full but there are no transactions to remove, this should not happen and will make the mempool grow indefinitely"
                );
                break;
            };
            // Skip tombstones — entries whose tx has already been removed.
            if !self.transaction_pool.contains_key(&hash) {
                continue;
            }
            self.remove_transaction_with_lock(&hash)?;
        }

        Ok(())
    }

    /// Rebuild the eviction heap from the live `transaction_pool`, dropping
    /// all tombstones. Used when too many lazy-deleted entries accumulate.
    fn rebuild_tip_heap(&mut self) {
        let mut heap = BinaryHeap::with_capacity(self.max_mempool_size * 2);
        for (hash, tx) in self.transaction_pool.iter() {
            heap.push(Reverse((tip_key(tx.transaction()), *hash)));
        }
        self.txs_by_tip = heap;
    }
}

/// Project a transaction's raw tip cap (`gas_tip_cap`) into a `u64` heap key.
/// Tips above `u64::MAX` saturate so astronomically-large fees rank highest
/// (and are therefore evicted last by the min-heap).
fn tip_key(tx: &Transaction) -> u64 {
    u64::try_from(tx.gas_tip_cap()).unwrap_or(u64::MAX)
}

#[derive(Debug, Default)]
pub struct Mempool {
    inner: RwLock<MempoolInner>,
    /// Signaled on transaction and blobs bundle insertions so payload
    /// builders can await new work instead of busy-looping.
    tx_added: tokio::sync::Notify,
}

impl Mempool {
    pub fn new(max_mempool_size: usize) -> Self {
        Mempool {
            inner: RwLock::new(MempoolInner::new(max_mempool_size)),
            tx_added: tokio::sync::Notify::new(),
        }
    }

    pub(crate) fn tx_added(&self) -> &tokio::sync::Notify {
        &self.tx_added
    }

    fn write(&self) -> Result<std::sync::RwLockWriteGuard<'_, MempoolInner>, StoreError> {
        self.inner
            .write()
            .map_err(|error| StoreError::MempoolWriteLock(error.to_string()))
    }

    fn read(&self) -> Result<std::sync::RwLockReadGuard<'_, MempoolInner>, StoreError> {
        self.inner
            .read()
            .map_err(|error| StoreError::MempoolReadLock(error.to_string()))
    }

    /// Add transaction to the pool without doing validity checks
    pub fn add_transaction(
        &self,
        hash: H256,
        sender: Address,
        transaction: MempoolTransaction,
    ) -> Result<(), StoreError> {
        let mut inner = self.write()?;
        // Rebuild the eviction heap if tombstones have accumulated past the
        // configured threshold (heap-size > max_mempool_size *
        // MEMPOOL_PRUNE_THRESHOLD_NUM / MEMPOOL_PRUNE_THRESHOLD_DEN).
        if inner.txs_by_tip.len() > inner.mempool_prune_threshold {
            inner.rebuild_tip_heap();
        }
        if inner.transaction_pool.len() >= inner.max_mempool_size {
            inner.evict_lowest_tip_transaction()?;
        }
        let tip = tip_key(transaction.transaction());
        inner.txs_by_tip.push(Reverse((tip, hash)));
        inner
            .txs_by_sender_nonce
            .insert((sender, transaction.nonce()), hash);
        inner.transaction_pool.insert(hash, transaction);
        inner.broadcast_pool.insert(hash);
        // Drop the write lock before notifying to avoid holding it while waking waiters
        drop(inner);
        self.tx_added.notify_waiters();

        Ok(())
    }

    pub fn get_txs_for_broadcast(&self) -> Result<Vec<MempoolTransaction>, StoreError> {
        let inner = self.read()?;
        let txs = inner
            .transaction_pool
            .iter()
            .filter_map(|(hash, tx)| {
                if !inner.broadcast_pool.contains(hash) {
                    None
                } else {
                    Some(tx.clone())
                }
            })
            .collect::<Vec<_>>();
        Ok(txs)
    }

    pub fn remove_broadcasted_txs(&self, hashes: &[H256]) -> Result<(), StoreError> {
        let mut inner = self.write()?;
        for hash in hashes {
            inner.broadcast_pool.remove(hash);
        }
        Ok(())
    }

    /// Add a blobs bundle to the pool by its blob transaction hash
    pub fn add_blobs_bundle(
        &self,
        tx_hash: H256,
        blobs_bundle: BlobsBundle,
    ) -> Result<(), StoreError> {
        let mut mempool = self.write()?;
        for (i, c) in blobs_bundle.commitments.iter().enumerate() {
            let versioned_hash = kzg_commitment_to_versioned_hash(c);
            mempool
                .blobs_bundle_by_versioned_hash
                .entry(versioned_hash)
                .or_default()
                .insert(tx_hash, i);
        }
        mempool.blobs_bundle_pool.insert(tx_hash, blobs_bundle);
        Ok(())
    }

    /// Get a blobs bundle to the pool given its blob transaction hash
    pub fn get_blobs_bundle(&self, tx_hash: H256) -> Result<Option<BlobsBundle>, StoreError> {
        Ok(self.read()?.blobs_bundle_pool.get(&tx_hash).cloned())
    }

    /// Remove a transaction from the pool
    pub fn remove_transaction(&self, hash: &H256) -> Result<(), StoreError> {
        let mut inner = self.write()?;
        inner.remove_transaction_with_lock(hash)?;
        Ok(())
    }

    /// Applies the filter and returns a set of suitable transactions from the mempool.
    /// These transactions will be grouped by sender and sorted by nonce
    pub fn filter_transactions(
        &self,
        filter: &PendingTxFilter,
    ) -> Result<FxHashMap<Address, Vec<MempoolTransaction>>, StoreError> {
        let filter_tx = |tx: &Transaction| -> bool {
            // Filter by tx type
            let is_blob_tx = matches!(tx, Transaction::EIP4844Transaction(_));
            if filter.only_plain_txs && is_blob_tx || filter.only_blob_txs && !is_blob_tx {
                return false;
            }

            // Filter by tip & base_fee
            if let Some(min_tip) = filter.min_tip.map(U256::from) {
                if tx
                    .effective_gas_tip(filter.base_fee)
                    .is_none_or(|tip| tip < min_tip)
                {
                    return false;
                }
            // This is a temporary fix to avoid invalid transactions to be included.
            // This should be removed once https://github.com/lambdaclass/ethrex/issues/680
            // is addressed.
            } else if tx.effective_gas_tip(filter.base_fee).is_none() {
                return false;
            }

            // Filter by blob gas fee
            if is_blob_tx
                && let Some(blob_fee) = filter.blob_fee
                && tx
                    .max_fee_per_blob_gas()
                    .is_none_or(|fee| fee < blob_fee.into())
            {
                return false;
            }
            true
        };
        self.filter_transactions_with_filter_fn(&filter_tx)
    }

    /// Gets all the transactions in the mempool
    pub fn get_all_txs_by_sender(
        &self,
    ) -> Result<FxHashMap<Address, Vec<MempoolTransaction>>, StoreError> {
        let mut txs_by_sender: FxHashMap<Address, Vec<MempoolTransaction>> =
            FxHashMap::with_capacity_and_hasher(128, Default::default());
        let tx_pool = &self.read()?.transaction_pool;

        for (_, tx) in tx_pool.iter() {
            txs_by_sender
                .entry(tx.sender())
                .or_insert_with(|| Vec::with_capacity(128))
                .push(tx.clone())
        }

        txs_by_sender.iter_mut().for_each(|(_, txs)| txs.sort());
        Ok(txs_by_sender)
    }

    /// Applies the filter and returns a set of suitable transactions from the mempool.
    /// These transactions will be grouped by sender and sorted by nonce
    pub fn filter_transactions_with_filter_fn(
        &self,
        filter: &dyn Fn(&Transaction) -> bool,
    ) -> Result<FxHashMap<Address, Vec<MempoolTransaction>>, StoreError> {
        let mut txs_by_sender: FxHashMap<Address, Vec<MempoolTransaction>> =
            FxHashMap::with_capacity_and_hasher(128, Default::default());
        let tx_pool = &self.read()?.transaction_pool;

        for (_, tx) in tx_pool.iter() {
            if filter(tx) {
                txs_by_sender
                    .entry(tx.sender())
                    .or_insert_with(|| Vec::with_capacity(128))
                    .push(tx.clone())
            }
        }

        txs_by_sender.iter_mut().for_each(|(_, txs)| txs.sort());
        Ok(txs_by_sender)
    }

    /// Filters hashes to those not already in the mempool or in-flight, and
    /// atomically marks the returned hashes as in-flight under a single write
    /// lock so that concurrent peer handlers cannot request the same hashes.
    pub fn reserve_unknown_hashes(
        &self,
        possible_hashes: &[H256],
    ) -> Result<Vec<H256>, StoreError> {
        let mut inner = self.write()?;

        let unknown: Vec<H256> = possible_hashes
            .iter()
            .filter(|hash| {
                !inner.in_flight_txs.contains(hash) && !inner.transaction_pool.contains_key(hash)
            })
            .copied()
            .collect();

        inner.in_flight_txs.extend(unknown.iter().copied());
        Ok(unknown)
    }

    /// Removes transaction hashes from the in-flight set, typically called
    /// when the GetPooledTransactions response arrives (or the connection drops).
    pub fn clear_in_flight_txs(&self, hashes: &[H256]) -> Result<(), StoreError> {
        let mut inner = self.write()?;
        for hash in hashes {
            inner.in_flight_txs.remove(hash);
        }
        Ok(())
    }

    pub fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        let tx = self
            .read()?
            .transaction_pool
            .get(&transaction_hash)
            .map(|e| e.transaction().clone());

        Ok(tx)
    }

    pub fn get_nonce(&self, address: &Address) -> Result<Option<u64>, MempoolError> {
        Ok(self
            .read()?
            .txs_by_sender_nonce
            .range((*address, 0)..=(*address, u64::MAX))
            .last()
            .map(|((_address, nonce), _hash)| nonce + 1))
    }

    pub fn get_mempool_size(&self) -> Result<(u64, u64), MempoolError> {
        let txs_size = {
            let pool_lock = &self.read()?.transaction_pool;
            pool_lock.len()
        };
        let blobs_size = {
            let pool_lock = &self.read()?.blobs_bundle_pool;
            pool_lock.len()
        };

        Ok((txs_size as u64, blobs_size as u64))
    }

    /// Returns all transactions currently in the pool
    pub fn content(&self) -> Result<Vec<Transaction>, MempoolError> {
        let pooled_transactions = &self.read()?.transaction_pool;
        Ok(pooled_transactions
            .values()
            .map(MempoolTransaction::transaction)
            .cloned()
            .collect())
    }

    /// Returns all blobs bundles currently in the pool
    pub fn get_blobs_bundle_pool(&self) -> Result<Vec<BlobsBundle>, MempoolError> {
        let blobs_bundle_pool = &self.read()?.blobs_bundle_pool;
        Ok(blobs_bundle_pool.values().cloned().collect())
    }

    /// Returns blobs data (blob, commitment, proof) associated with the versioned hashes
    pub fn get_blobs_data_by_versioned_hashes(
        &self,
        versioned_hashes: &[H256],
    ) -> Result<Vec<Option<BlobTuple>>, MempoolError> {
        let mempool = self.read()?;
        let blobs_bundle_pool = &mempool.blobs_bundle_pool;
        let blobs_bundle_by_versioned_hash = &mempool.blobs_bundle_by_versioned_hash;
        let mut res = vec![None; versioned_hashes.len()];
        for (idx, vh) in versioned_hashes.iter().enumerate() {
            if let Some((found_hash, inner_pos)) = blobs_bundle_by_versioned_hash
                .get(vh)
                .and_then(|h| h.iter().next())
            {
                res[idx] = blobs_bundle_pool
                    .get(found_hash)
                    .and_then(|b| b.get_blob_tuple_by_index(*inner_pos))
            }
        }
        Ok(res)
    }

    /// Returns the status of the mempool, which is the number of transactions currently in
    /// the pool. Until we add "queue" transactions.
    pub fn status(&self) -> Result<u64, MempoolError> {
        let pool_lock = &self.read()?.transaction_pool;

        Ok(pool_lock.len() as u64)
    }

    pub fn contains_sender_nonce(
        &self,
        sender: Address,
        nonce: u64,
        received_hash: H256,
    ) -> Result<Option<MempoolTransaction>, MempoolError> {
        let Some(hash) = self
            .read()?
            .txs_by_sender_nonce
            .get(&(sender, nonce))
            .cloned()
        else {
            return Ok(None);
        };
        if hash == received_hash {
            return Ok(None);
        }

        let transaction_pool = &self.read()?.transaction_pool;
        let tx = transaction_pool.get(&hash).cloned();
        Ok(tx)
    }

    pub fn contains_tx(&self, tx_hash: H256) -> Result<bool, MempoolError> {
        let contains = self.read()?.transaction_pool.contains_key(&tx_hash);
        Ok(contains)
    }

    pub fn find_tx_to_replace(
        &self,
        sender: Address,
        nonce: u64,
        tx: &Transaction,
    ) -> Result<Option<H256>, MempoolError> {
        let Some(tx_in_pool) = self.contains_sender_nonce(sender, nonce, tx.hash())? else {
            return Ok(None);
        };
        let is_a_replacement_tx = {
            // EIP-1559 values
            let old_tx_max_fee_per_gas = tx_in_pool.max_fee_per_gas().unwrap_or_default();
            let old_tx_max_priority_fee_per_gas = tx_in_pool.max_priority_fee().unwrap_or_default();
            let new_tx_max_fee_per_gas = tx.max_fee_per_gas().unwrap_or_default();
            let new_tx_max_priority_fee_per_gas = tx.max_priority_fee().unwrap_or_default();

            // Legacy tx values
            let old_tx_gas_price = tx_in_pool.gas_price();
            let new_tx_gas_price = tx.gas_price();

            // EIP-4844 values
            let old_tx_max_fee_per_blob = tx_in_pool.max_fee_per_blob_gas();
            let new_tx_max_fee_per_blob = tx.max_fee_per_blob_gas();

            let eip4844_higher_fees = if let (Some(old_blob_fee), Some(new_blob_fee)) =
                (old_tx_max_fee_per_blob, new_tx_max_fee_per_blob)
            {
                new_blob_fee > old_blob_fee
            } else {
                true // We are marking it as always true if the tx is not eip-4844
            };

            let eip1559_higher_fees = new_tx_max_fee_per_gas > old_tx_max_fee_per_gas
                && new_tx_max_priority_fee_per_gas > old_tx_max_priority_fee_per_gas;
            let legacy_higher_fees = new_tx_gas_price > old_tx_gas_price;

            eip4844_higher_fees && (eip1559_higher_fees || legacy_higher_fees)
        };

        if !is_a_replacement_tx {
            return Err(MempoolError::UnderpricedReplacement);
        }

        Ok(Some(tx_in_pool.hash()))
    }
}

/// Filter applied by the payload builder when querying pending transactions
/// from the pool. NOT a mempool admission gate — all fields here are
/// query-time filters used to pick block-includable transactions. Admission
/// rules are enforced in `Blockchain::validate_transaction`.
#[derive(Debug, Default)]
pub struct PendingTxFilter {
    /// Minimum effective priority fee for a transaction to be surfaced to
    /// the payload builder. This is a block-building filter, not an
    /// admission check — see `crates/common/types/constants.rs::MIN_GAS_TIP`.
    pub min_tip: Option<u64>,
    pub base_fee: Option<u64>,
    pub blob_fee: Option<u64>,
    pub only_plain_txs: bool,
    pub only_blob_txs: bool,
}

pub fn transaction_intrinsic_gas(
    tx: &Transaction,
    header: &BlockHeader,
    config: &ChainConfig,
) -> Result<u64, MempoolError> {
    let is_contract_creation = tx.is_contract_creation();

    let mut gas = if is_contract_creation {
        TX_CREATE_GAS_COST
    } else {
        TX_GAS_COST
    };

    let data_len = tx.data().len() as u64;

    if data_len > 0 {
        let non_zero_gas_cost = if config.is_istanbul_activated(header.number) {
            TX_DATA_NON_ZERO_GAS_EIP2028
        } else {
            TX_DATA_NON_ZERO_GAS
        };

        let non_zero_count = tx.data().iter().filter(|&&x| x != 0u8).count() as u64;

        gas = gas
            .checked_add(non_zero_count * non_zero_gas_cost)
            .ok_or(MempoolError::TxGasOverflowError)?;

        let zero_count = data_len - non_zero_count;

        gas = gas
            .checked_add(zero_count * TX_DATA_ZERO_GAS_COST)
            .ok_or(MempoolError::TxGasOverflowError)?;

        if is_contract_creation && config.is_shanghai_activated(header.timestamp) {
            // Len in 32 bytes sized words
            let len_in_words = data_len.saturating_add(31) / 32;

            gas = gas
                .checked_add(len_in_words * TX_INIT_CODE_WORD_GAS_COST)
                .ok_or(MempoolError::TxGasOverflowError)?;
        }
    }

    let storage_keys_count: u64 = tx
        .access_list()
        .iter()
        .map(|(_, keys)| keys.len() as u64)
        .sum();

    gas = gas
        .checked_add(tx.access_list().len() as u64 * TX_ACCESS_LIST_ADDRESS_GAS)
        .ok_or(MempoolError::TxGasOverflowError)?;

    gas = gas
        .checked_add(storage_keys_count * TX_ACCESS_LIST_STORAGE_KEY_GAS)
        .ok_or(MempoolError::TxGasOverflowError)?;

    Ok(gas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::{EIP1559Transaction, TxKind};

    /// Build a unique EIP-1559 transaction parametrized by its priority fee.
    /// `nonce` is used to keep the transaction (and therefore its hash) unique
    /// across calls so we exercise distinct mempool entries.
    fn make_tx(tip: u64, nonce: u64) -> (H256, Address, MempoolTransaction) {
        let inner = EIP1559Transaction {
            nonce,
            max_priority_fee_per_gas: tip,
            // Keep `max_fee_per_gas >= max_priority_fee_per_gas` so the tx is
            // structurally valid even though we bypass mempool admission here.
            max_fee_per_gas: tip,
            gas_limit: 21_000,
            to: TxKind::Call(Address::from_low_u64_be(1)),
            ..Default::default()
        };
        let tx = Transaction::EIP1559Transaction(inner);
        let sender = Address::random();
        let hash = H256::random();
        (hash, sender, MempoolTransaction::new(tx, sender))
    }

    fn pool_size(mempool: &Mempool) -> usize {
        mempool.read().expect("read lock").transaction_pool.len()
    }

    fn heap_size(mempool: &Mempool) -> usize {
        mempool.read().expect("read lock").txs_by_tip.len()
    }

    #[test]
    fn evicts_lowest_tip_when_pool_is_full() {
        let max = 4;
        let mempool = Mempool::new(max);

        // Fill the pool with tips 10, 20, 30, 40.
        let mut handles = Vec::new();
        for (i, tip) in [10u64, 20, 30, 40].iter().enumerate() {
            let (hash, sender, tx) = make_tx(*tip, i as u64);
            mempool.add_transaction(hash, sender, tx).unwrap();
            handles.push((hash, *tip));
        }
        assert_eq!(pool_size(&mempool), max);

        // Insert one more with a clearly higher tip; the tip=10 entry should go.
        let (hash, sender, tx) = make_tx(100, 100);
        mempool.add_transaction(hash, sender, tx).unwrap();

        assert_eq!(pool_size(&mempool), max);
        // tip=10 hash must be gone, tip=100 newcomer must be present.
        let low_hash = handles[0].0;
        assert!(!mempool.contains_tx(low_hash).unwrap());
        assert!(mempool.contains_tx(hash).unwrap());
        // The other original entries (tips 20, 30, 40) are still in.
        for (h, _) in handles.iter().skip(1) {
            assert!(mempool.contains_tx(*h).unwrap());
        }
    }

    #[test]
    fn high_tip_newcomer_is_kept_over_existing_low_tip() {
        let max = 2;
        let mempool = Mempool::new(max);

        let (low_hash, low_sender, low_tx) = make_tx(1, 0);
        let (mid_hash, mid_sender, mid_tx) = make_tx(5, 1);
        mempool
            .add_transaction(low_hash, low_sender, low_tx)
            .unwrap();
        mempool
            .add_transaction(mid_hash, mid_sender, mid_tx)
            .unwrap();

        // Pool is full; insert a high-tip newcomer.
        let (hi_hash, hi_sender, hi_tx) = make_tx(1_000, 2);
        mempool.add_transaction(hi_hash, hi_sender, hi_tx).unwrap();

        // The newcomer must NOT be the one evicted.
        assert!(mempool.contains_tx(hi_hash).unwrap());
        // The lowest-tip prior entry must be evicted.
        assert!(!mempool.contains_tx(low_hash).unwrap());
        assert!(mempool.contains_tx(mid_hash).unwrap());
        assert_eq!(pool_size(&mempool), max);
    }

    #[test]
    fn lazy_deletion_skips_dead_heap_entries() {
        let max = 3;
        let mempool = Mempool::new(max);

        // Add three txs with tips 50, 60, 70.
        let (low_hash, low_sender, low_tx) = make_tx(50, 0);
        let (mid_hash, mid_sender, mid_tx) = make_tx(60, 1);
        let (hi_hash, hi_sender, hi_tx) = make_tx(70, 2);
        mempool
            .add_transaction(low_hash, low_sender, low_tx)
            .unwrap();
        mempool
            .add_transaction(mid_hash, mid_sender, mid_tx)
            .unwrap();
        mempool.add_transaction(hi_hash, hi_sender, hi_tx).unwrap();

        // Remove the lowest-tip tx normally — its heap entry becomes a tombstone.
        mempool.remove_transaction(&low_hash).unwrap();
        // Heap still contains the stale entry; the pool does not.
        assert_eq!(pool_size(&mempool), 2);
        assert_eq!(heap_size(&mempool), 3);

        // Fill back to capacity with a tip greater than 60 but less than 70.
        let (replacement_hash, replacement_sender, replacement_tx) = make_tx(65, 3);
        mempool
            .add_transaction(replacement_hash, replacement_sender, replacement_tx)
            .unwrap();
        assert_eq!(pool_size(&mempool), 3);

        // Now adding one more forces eviction. The lazy-deleted (tip=50) entry
        // must be skipped, and tip=60 (the actual lowest live entry) must go.
        let (newcomer_hash, newcomer_sender, newcomer_tx) = make_tx(80, 4);
        mempool
            .add_transaction(newcomer_hash, newcomer_sender, newcomer_tx)
            .unwrap();

        assert_eq!(pool_size(&mempool), 3);
        assert!(!mempool.contains_tx(mid_hash).unwrap());
        assert!(mempool.contains_tx(newcomer_hash).unwrap());
        assert!(mempool.contains_tx(replacement_hash).unwrap());
        assert!(mempool.contains_tx(hi_hash).unwrap());
    }

    #[test]
    fn heap_prune_rebuilds_when_threshold_exceeded() {
        let max = 4;
        let mempool = Mempool::new(max);

        // Fill the pool, then immediately remove every entry. Heap accumulates
        // `max` tombstones (heap_size = 4, pool_size = 0).
        let mut hashes = Vec::new();
        for i in 0..max {
            let (hash, sender, tx) = make_tx(10 + i as u64, i as u64);
            mempool.add_transaction(hash, sender, tx).unwrap();
            hashes.push(hash);
        }
        for hash in &hashes {
            mempool.remove_transaction(hash).unwrap();
        }
        assert_eq!(pool_size(&mempool), 0);
        assert_eq!(heap_size(&mempool), max);

        // Push more tombstones by inserting + removing more txs. Once the heap
        // grows past `MEMPOOL_PRUNE_THRESHOLD_NUM * max / MEMPOOL_PRUNE_THRESHOLD_DEN`
        // (= 6 for max=4), the next `add_transaction` must rebuild the heap.
        let threshold = max * MEMPOOL_PRUNE_THRESHOLD_NUM / MEMPOOL_PRUNE_THRESHOLD_DEN;
        let mut more_hashes = Vec::new();
        while heap_size(&mempool) <= threshold {
            let nonce = (max + more_hashes.len()) as u64;
            let (hash, sender, tx) = make_tx(100, nonce);
            mempool.add_transaction(hash, sender, tx).unwrap();
            more_hashes.push(hash);
            mempool.remove_transaction(&hash).unwrap();
        }
        assert_eq!(pool_size(&mempool), 0);
        assert!(heap_size(&mempool) > threshold);

        // The next insertion triggers the rebuild branch and drops all tombstones.
        let (hash, sender, tx) = make_tx(42, 9999);
        mempool.add_transaction(hash, sender, tx).unwrap();
        // After rebuild + insert, the heap should contain exactly the live txs.
        assert_eq!(pool_size(&mempool), 1);
        assert_eq!(heap_size(&mempool), 1);
    }
}
