use std::{
    collections::{BTreeMap, VecDeque, hash_map::Entry},
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
    txs_order: VecDeque<H256>,
    max_mempool_size: usize,
    // Max number of transactions to let the mempool order queue grow before pruning it
    mempool_prune_threshold: usize,
}

impl MempoolInner {
    fn new(max_mempool_size: usize) -> Self {
        MempoolInner {
            txs_order: VecDeque::with_capacity(max_mempool_size * 2),
            transaction_pool: FxHashMap::with_capacity_and_hasher(
                max_mempool_size,
                Default::default(),
            ),
            max_mempool_size,
            mempool_prune_threshold: max_mempool_size + max_mempool_size / 2,
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

    /// Remove the oldest transaction in the pool
    fn remove_oldest_transaction(&mut self) -> Result<(), StoreError> {
        // Remove elements from the order queue until one is present in the pool
        while self.transaction_pool.len() >= self.max_mempool_size {
            if let Some(oldest_hash) = self.txs_order.pop_front() {
                self.remove_transaction_with_lock(&oldest_hash)?;
            } else {
                warn!(
                    "Mempool is full but there are no transactions to remove, this should not happen and will make the mempool grow indefinitely"
                );
                break;
            }
        }

        Ok(())
    }
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

    /// Add transaction to the pool without doing validity checks.
    ///
    /// Enforces the per-sender pending-tx cap atomically: the count is
    /// re-checked under the same write lock that performs the insertion.
    /// Replacement candidates (same `(sender, nonce)`) must have already
    /// been removed via `remove_transaction` so this counter reflects the
    /// post-replacement state. Returns
    /// [`MempoolError::MaxPendingTxsPerAccountExceeded`] if the cap would
    /// be exceeded.
    ///
    /// When `punish_spammer` is true, breaching the cap additionally
    /// drops the highest-nonce half of the sender's existing pool entries
    /// (rounded up for odd counts) before returning the error. Erigon-style
    /// punishment: a sender hitting the cap is likely spamming future
    /// nonces, so freeing those slots reclaims pool budget for transactions
    /// more likely to execute next. The new transaction is still rejected.
    pub fn add_transaction(
        &self,
        hash: H256,
        sender: Address,
        transaction: MempoolTransaction,
        max_pending_txs_per_account: usize,
        punish_spammer: bool,
    ) -> Result<(), MempoolError> {
        let mut inner = self.write()?;
        let count = inner
            .txs_by_sender_nonce
            .range((sender, 0)..=(sender, u64::MAX))
            .count();
        if count >= max_pending_txs_per_account {
            // Drop the upper half of the sender's pool entries, rounded up so
            // odd counts still lose the median entry (`ceil(count / 2)`).
            // Skip the prune when `count <= 1`: dropping the only entry adds
            // no useful punishment (the sender already gets the new tx
            // rejected) and turns interactions with delegated cap=1 into
            // "every cap-breach wipes the sender's lone tx".
            let mut dropped_count = 0usize;
            if punish_spammer && count > 1 {
                // Collect the sender's nonce range once, in reverse, so we
                // can read victims and new_top_nonce in a single pass and
                // release the immutable borrow before mutating the map.
                let entries: Vec<(u64, H256)> = inner
                    .txs_by_sender_nonce
                    .range((sender, 0)..=(sender, u64::MAX))
                    .rev()
                    .map(|((_, nonce), hash)| (*nonce, *hash))
                    .collect();
                dropped_count = count.div_ceil(2);
                let new_top_nonce = entries.get(dropped_count).map(|(nonce, _)| *nonce);
                for (_, victim_hash) in entries.iter().take(dropped_count) {
                    inner.remove_transaction_with_lock(victim_hash)?;
                }
                warn!(
                    target: "mempool",
                    sender = ?sender,
                    dropped_count,
                    new_top_nonce = ?new_top_nonce,
                    "punishSpammer: per-sender cap breached; dropped highest-nonce half of sender's pool entries"
                );
            }
            return Err(MempoolError::MaxPendingTxsPerAccountExceeded {
                count: count.saturating_sub(dropped_count),
                limit: max_pending_txs_per_account,
            });
        }
        // Prune the order queue if it has grown too much
        if inner.txs_order.len() > inner.mempool_prune_threshold {
            // NOTE: we do this to avoid borrow checker errors
            let txpool = core::mem::take(&mut inner.transaction_pool);
            inner.txs_order.retain(|tx| txpool.contains_key(tx));
            inner.transaction_pool = txpool;
        }
        if inner.transaction_pool.len() >= inner.max_mempool_size {
            inner.remove_oldest_transaction()?;
        }
        inner.txs_order.push_back(hash);
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

    /// Returns the number of pending transactions currently held in the
    /// mempool for `sender`. Used by the per-sender slot cap at admission.
    pub fn count_for_sender(&self, sender: Address) -> Result<usize, MempoolError> {
        let inner = self.read()?;
        let count = inner
            .txs_by_sender_nonce
            .range((sender, 0)..=(sender, u64::MAX))
            .count();
        Ok(count)
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
    use ethrex_common::types::EIP1559Transaction;

    fn build_tx(nonce: u64) -> Transaction {
        Transaction::EIP1559Transaction(EIP1559Transaction {
            nonce,
            ..Default::default()
        })
    }

    fn add_tx(pool: &Mempool, sender: Address, nonce: u64) -> H256 {
        let tx = build_tx(nonce);
        let mtx = MempoolTransaction::new(tx, sender);
        let hash = mtx.hash();
        pool.add_transaction(hash, sender, mtx, usize::MAX, true)
            .unwrap();
        hash
    }

    #[test]
    fn count_for_sender_empty_pool() {
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(1);
        assert_eq!(pool.count_for_sender(sender).unwrap(), 0);
    }

    #[test]
    fn count_for_sender_one_tx() {
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(1);
        add_tx(&pool, sender, 0);
        assert_eq!(pool.count_for_sender(sender).unwrap(), 1);
    }

    #[test]
    fn count_for_sender_many_nonces() {
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(1);
        for nonce in 0..5 {
            add_tx(&pool, sender, nonce);
        }
        assert_eq!(pool.count_for_sender(sender).unwrap(), 5);
    }

    #[test]
    fn count_for_sender_isolates_senders() {
        let pool = Mempool::new(64);
        let a = Address::from_low_u64_be(1);
        let b = Address::from_low_u64_be(2);
        add_tx(&pool, a, 0);
        add_tx(&pool, a, 1);
        add_tx(&pool, b, 0);
        assert_eq!(pool.count_for_sender(a).unwrap(), 2);
        assert_eq!(pool.count_for_sender(b).unwrap(), 1);
    }

    #[test]
    fn count_for_sender_unknown_returns_zero() {
        let pool = Mempool::new(64);
        let a = Address::from_low_u64_be(1);
        let b = Address::from_low_u64_be(2);
        add_tx(&pool, a, 0);
        assert_eq!(pool.count_for_sender(b).unwrap(), 0);
    }

    /// Helper that submits a new tx for `sender` at `nonce` with the given
    /// cap and punish flag, expecting it to be rejected with
    /// `MaxPendingTxsPerAccountExceeded`.
    fn submit_at_cap(
        pool: &Mempool,
        sender: Address,
        nonce: u64,
        cap: usize,
        punish_spammer: bool,
    ) {
        let tx = build_tx(nonce);
        let mtx = MempoolTransaction::new(tx, sender);
        let hash = mtx.hash();
        let err = pool
            .add_transaction(hash, sender, mtx, cap, punish_spammer)
            .expect_err("expected per-account cap rejection");
        assert!(
            matches!(err, MempoolError::MaxPendingTxsPerAccountExceeded { .. }),
            "expected MaxPendingTxsPerAccountExceeded, got {err:?}"
        );
    }

    #[test]
    fn punish_spammer_drops_highest_nonce_half_at_cap() {
        let pool = Mempool::new(128);
        let sender = Address::from_low_u64_be(1);
        // Fill the sender up to a cap of 16.
        for nonce in 0..16u64 {
            add_tx(&pool, sender, nonce);
        }
        assert_eq!(pool.count_for_sender(sender).unwrap(), 16);

        // 17th tx is rejected and triggers punishment.
        submit_at_cap(&pool, sender, 16, 16, true);

        // 8 highest-nonce entries (nonces 8..16) should be dropped.
        assert_eq!(pool.count_for_sender(sender).unwrap(), 8);
        let inner = pool.read().unwrap();
        let remaining_nonces: Vec<u64> = inner
            .txs_by_sender_nonce
            .range((sender, 0)..=(sender, u64::MAX))
            .map(|((_, n), _)| *n)
            .collect();
        assert_eq!(remaining_nonces, (0..8u64).collect::<Vec<_>>());
    }

    #[test]
    fn punish_spammer_disabled_leaves_existing_txs() {
        let pool = Mempool::new(128);
        let sender = Address::from_low_u64_be(1);
        for nonce in 0..16u64 {
            add_tx(&pool, sender, nonce);
        }
        assert_eq!(pool.count_for_sender(sender).unwrap(), 16);

        // With punish_spammer = false, the new tx is rejected but the
        // existing 16 entries are untouched.
        submit_at_cap(&pool, sender, 16, 16, false);

        assert_eq!(pool.count_for_sender(sender).unwrap(), 16);
    }

    #[test]
    fn below_cap_admits_normally_without_punishment() {
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(1);
        // Cap is 2; sender currently has 1 pending.
        add_tx(&pool, sender, 0);
        assert_eq!(pool.count_for_sender(sender).unwrap(), 1);

        let tx = build_tx(1);
        let mtx = MempoolTransaction::new(tx, sender);
        let hash = mtx.hash();
        pool.add_transaction(hash, sender, mtx, 2, true)
            .expect("below-cap submission should be admitted");

        assert_eq!(pool.count_for_sender(sender).unwrap(), 2);
    }

    #[test]
    fn punish_spammer_removes_blob_bundles_for_dropped_blob_txs() {
        use ethrex_common::types::{BlobsBundle, EIP4844Transaction};

        let pool = Mempool::new(128);
        let sender = Address::from_low_u64_be(1);

        // Build 16 entries alternating blob and non-blob txs. Blob txs sit
        // at odd nonces so half of them (the highest-nonce ones: 9, 11, 13, 15)
        // fall into the dropped upper half.
        let mut blob_hashes_in_upper_half: Vec<H256> = Vec::new();
        let mut blob_hashes_in_lower_half: Vec<H256> = Vec::new();
        for nonce in 0..16u64 {
            let (tx, is_blob) = if nonce % 2 == 1 {
                (
                    Transaction::EIP4844Transaction(EIP4844Transaction {
                        nonce,
                        ..Default::default()
                    }),
                    true,
                )
            } else {
                (build_tx(nonce), false)
            };
            let mtx = MempoolTransaction::new(tx, sender);
            let hash = mtx.hash();
            if is_blob {
                pool.add_blobs_bundle(hash, BlobsBundle::default()).unwrap();
                if nonce >= 8 {
                    blob_hashes_in_upper_half.push(hash);
                } else {
                    blob_hashes_in_lower_half.push(hash);
                }
            }
            pool.add_transaction(hash, sender, mtx, usize::MAX, true)
                .unwrap();
        }
        assert_eq!(pool.count_for_sender(sender).unwrap(), 16);
        for h in &blob_hashes_in_upper_half {
            assert!(pool.get_blobs_bundle(*h).unwrap().is_some());
        }

        // Trigger punishment with cap = 16.
        submit_at_cap(&pool, sender, 16, 16, true);

        // Upper-half blob bundles must be gone; lower-half blob bundles
        // must remain.
        assert_eq!(pool.count_for_sender(sender).unwrap(), 8);
        for h in &blob_hashes_in_upper_half {
            assert!(
                pool.get_blobs_bundle(*h).unwrap().is_none(),
                "blob bundle for dropped tx {h:?} should have been removed"
            );
        }
        for h in &blob_hashes_in_lower_half {
            assert!(
                pool.get_blobs_bundle(*h).unwrap().is_some(),
                "blob bundle for surviving tx {h:?} should still be present"
            );
        }
    }

    #[test]
    fn punish_spammer_skips_prune_when_count_is_one() {
        // With cap = 1 and a single pending tx, the prune-on-breach policy
        // would wipe the sender's only tx on every cap-breach attempt, which
        // is especially harmful when combined with delegated cap=1 (every
        // collision wipes the prior tx). The implementation skips the prune
        // when `count <= 1`.
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(7);
        add_tx(&pool, sender, 0);
        assert_eq!(pool.count_for_sender(sender).unwrap(), 1);

        submit_at_cap(&pool, sender, 1, 1, true);

        // The pre-existing single tx must survive.
        assert_eq!(pool.count_for_sender(sender).unwrap(), 1);
    }

    #[test]
    fn punish_spammer_reports_post_prune_count() {
        // The rejection error must reflect the sender's count AFTER the prune
        // so RPC clients see the actual post-state, not the pre-prune count.
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(8);
        for nonce in 0..16u64 {
            add_tx(&pool, sender, nonce);
        }

        let tx = build_tx(16);
        let mtx = MempoolTransaction::new(tx, sender);
        let hash = mtx.hash();
        let err = pool
            .add_transaction(hash, sender, mtx, 16, true)
            .expect_err("expected per-account cap rejection");

        match err {
            MempoolError::MaxPendingTxsPerAccountExceeded { count, limit } => {
                // 16 - ceil(16/2) = 16 - 8 = 8 remaining after the prune.
                assert_eq!(count, 8);
                assert_eq!(limit, 16);
            }
            other => panic!("expected MaxPendingTxsPerAccountExceeded, got {other:?}"),
        }
    }
}
