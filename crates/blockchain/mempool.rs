use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::RwLock,
};

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
    types::{BlobsBundle, BlockHeader, ChainConfig, MempoolTransaction, Transaction, TxType},
};
use ethrex_storage::error::StoreError;
use std::collections::HashSet;
use tracing::warn;

#[derive(Debug, Default)]
struct MempoolInner {
    broadcast_pool: HashSet<H256>,
    transaction_pool: HashMap<H256, MempoolTransaction>,
    blobs_bundle_pool: HashMap<H256, BlobsBundle>,
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
            transaction_pool: HashMap::with_capacity(max_mempool_size),
            max_mempool_size,
            mempool_prune_threshold: max_mempool_size + max_mempool_size / 2,
            ..Default::default()
        }
    }

    /// Remove a transaction from the pool with the transaction pool lock already taken
    fn remove_transaction_with_lock(&mut self, hash: &H256) -> Result<(), StoreError> {
        if let Some(tx) = self.transaction_pool.get(hash) {
            if matches!(tx.tx_type(), TxType::EIP4844) {
                self.blobs_bundle_pool.remove(hash);
            }

            self.txs_by_sender_nonce.remove(&(tx.sender(), tx.nonce()));
            self.transaction_pool.remove(hash);
            self.broadcast_pool.remove(hash);
        };

        Ok(())
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
}

impl Mempool {
    pub fn new(max_mempool_size: usize) -> Self {
        Mempool {
            inner: RwLock::new(MempoolInner::new(max_mempool_size)),
        }
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

    pub fn clear_broadcasted_txs(&self) -> Result<(), StoreError> {
        self.write()?.broadcast_pool.clear();
        Ok(())
    }

    /// Add a blobs bundle to the pool by its blob transaction hash
    pub fn add_blobs_bundle(
        &self,
        tx_hash: H256,
        blobs_bundle: BlobsBundle,
    ) -> Result<(), StoreError> {
        self.write()?
            .blobs_bundle_pool
            .insert(tx_hash, blobs_bundle);
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
    ) -> Result<HashMap<Address, Vec<MempoolTransaction>>, StoreError> {
        let filter_tx = |tx: &Transaction| -> bool {
            // Filter by tx type
            let is_blob_tx = matches!(tx, Transaction::EIP4844Transaction(_));
            if filter.only_plain_txs && is_blob_tx || filter.only_blob_txs && !is_blob_tx {
                return false;
            }

            // Filter by tip & base_fee
            if let Some(min_tip) = filter.min_tip {
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
                && tx.max_fee_per_blob_gas().is_none_or(|fee| fee < blob_fee)
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
    ) -> Result<HashMap<Address, Vec<MempoolTransaction>>, StoreError> {
        let mut txs_by_sender: HashMap<Address, Vec<MempoolTransaction>> =
            HashMap::with_capacity(128);
        let tx_pool = &self.read()?.transaction_pool;

        for (_, tx) in tx_pool.iter() {
            txs_by_sender
                .entry(tx.sender())
                .or_insert_with(|| Vec::with_capacity(128))
                .push(tx.clone())
        }

        txs_by_sender.iter_mut().for_each(|(_, txs)| txs.sort_unstable());
        Ok(txs_by_sender)
    }

    /// Applies the filter and returns a set of suitable transactions from the mempool.
    /// These transactions will be grouped by sender and sorted by nonce
    pub fn filter_transactions_with_filter_fn(
        &self,
        filter: &dyn Fn(&Transaction) -> bool,
    ) -> Result<HashMap<Address, Vec<MempoolTransaction>>, StoreError> {
        let mut txs_by_sender: HashMap<Address, Vec<MempoolTransaction>> =
            HashMap::with_capacity(128);
        let tx_pool = &self.read()?.transaction_pool;

        for (_, tx) in tx_pool.iter() {
            if filter(tx) {
                txs_by_sender
                    .entry(tx.sender())
                    .or_insert_with(|| Vec::with_capacity(128))
                    .push(tx.clone())
            }
        }

        txs_by_sender.iter_mut().for_each(|(_, txs)| txs.sort_unstable());
        Ok(txs_by_sender)
    }

    /// Gets hashes from possible_hashes that are not already known in the mempool.
    pub fn filter_unknown_transactions(
        &self,
        possible_hashes: &[H256],
    ) -> Result<Vec<H256>, StoreError> {
        let tx_pool = &self.read()?.transaction_pool;

        Ok(possible_hashes
            .iter()
            .filter(|hash| !tx_pool.contains_key(hash))
            .copied()
            .collect())
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

#[derive(Debug, Default)]
pub struct PendingTxFilter {
    pub min_tip: Option<u64>,
    pub base_fee: Option<u64>,
    pub blob_fee: Option<U256>,
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
