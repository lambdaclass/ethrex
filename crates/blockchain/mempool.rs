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
use ethrex_vm::{intrinsic_gas_dimensions, intrinsic_gas_floor};
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
        price_bump_percent: u64,
        blob_price_bump_percent: u64,
    ) -> Result<Option<H256>, MempoolError> {
        let Some(tx_in_pool) = self.contains_sender_nonce(sender, nonce, tx.hash())? else {
            return Ok(None);
        };

        // Reject type-change replacements. Peer clients keep blob and
        // non-blob transactions in separate sub-pools precisely so a
        // cheap-to-replicate non-blob tx can't displace a blob tx (with
        // its expensive sidecar) or vice versa. ethrex has a single pool,
        // so the same guarantee has to be enforced here.
        if std::mem::discriminant(tx) != std::mem::discriminant(tx_in_pool.transaction()) {
            return Err(MempoolError::ReplacementTypeMismatch);
        }

        // Blob replacements use a stricter bump (default 100%) because blob
        // sidecars are expensive to re-propagate; all other tx types use the
        // base bump (default 10%).
        let bump = if matches!(tx, Transaction::EIP4844Transaction(_)) {
            blob_price_bump_percent
        } else {
            price_bump_percent
        };

        // The new tx must bump every applicable fee field on its own tx type
        // by at least `bump` percent compared to the in-pool tx at the same
        // (sender, nonce). Each peer EL client enforces this independently per
        // field.
        let is_a_replacement_tx = match tx {
            Transaction::LegacyTransaction(_) => {
                is_bumped_u256(tx_in_pool.gas_price(), tx.gas_price(), bump)
            }
            Transaction::EIP4844Transaction(_) => {
                let bumped_fee = is_bumped_u64(
                    tx_in_pool.max_fee_per_gas().unwrap_or_default(),
                    tx.max_fee_per_gas().unwrap_or_default(),
                    bump,
                );
                let bumped_tip = is_bumped_u64(
                    tx_in_pool.max_priority_fee().unwrap_or_default(),
                    tx.max_priority_fee().unwrap_or_default(),
                    bump,
                );
                let bumped_blob = is_bumped_u256(
                    tx_in_pool.max_fee_per_blob_gas().unwrap_or_default(),
                    tx.max_fee_per_blob_gas().unwrap_or_default(),
                    bump,
                );
                bumped_fee && bumped_tip && bumped_blob
            }
            // EIP-2930 / EIP-1559 / EIP-7702 / FeeToken / Privileged: 1559-style
            // pair of fee fields. (PrivilegedL2 transactions short-circuit
            // before `validate_transaction` ever reaches `find_tx_to_replace`,
            // so the Privileged variant of this arm is unreachable in practice;
            // the other variants do hit it.)
            _ => {
                let bumped_fee = is_bumped_u64(
                    tx_in_pool.max_fee_per_gas().unwrap_or_default(),
                    tx.max_fee_per_gas().unwrap_or_default(),
                    bump,
                );
                let bumped_tip = is_bumped_u64(
                    tx_in_pool.max_priority_fee().unwrap_or_default(),
                    tx.max_priority_fee().unwrap_or_default(),
                    bump,
                );
                bumped_fee && bumped_tip
            }
        };

        if !is_a_replacement_tx {
            return Err(MempoolError::UnderpricedReplacement);
        }

        Ok(Some(tx_in_pool.hash()))
    }
}

/// Returns true iff `new >= floor(existing * (100 + bump_percent) / 100)`.
/// Uses `u128` intermediates with checked arithmetic so an overflow on the
/// threshold computation is treated as "reject" rather than silently
/// admitting an under-priced replacement. A `bump_percent` of 0 collapses to
/// `new >= existing`.
fn is_bumped_u64(existing: u64, new: u64, bump_percent: u64) -> bool {
    let multiplier = 100u128 + bump_percent as u128;
    let Some(threshold) = (existing as u128).checked_mul(multiplier).map(|v| v / 100) else {
        return false;
    };
    (new as u128) >= threshold
}

/// U256 variant of [`is_bumped_u64`]. Used for `gas_price` (legacy) and
/// `max_fee_per_blob_gas` (EIP-4844). Same overflow → reject semantic via
/// `checked_mul`.
fn is_bumped_u256(existing: U256, new: U256, bump_percent: u64) -> bool {
    let multiplier = U256::from(100u64 + bump_percent);
    let Some(threshold) = existing
        .checked_mul(multiplier)
        .map(|v| v / U256::from(100u64))
    else {
        return false;
    };
    new >= threshold
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
    // Amsterdam (EIP-8037): the VM splits intrinsic into (regular, state) and uses
    // `REGULAR_GAS_CREATE = 9000` + `STATE_BYTES_PER_NEW_ACCOUNT * cpsb` for CREATE
    // instead of the legacy `TX_CREATE_GAS_COST = 53000`. Mempool admission must
    // match VM charge or we spuriously reject (or admit) transactions.
    //
    // The VM enforces `gas_limit >= max(intrinsic_regular + intrinsic_state,
    // floor)` via two separate checks in `validate_gas_allowance` +
    // `validate_min_gas_limit`. Apply the same max here so we don't admit
    // txs whose calldata floor exceeds the weighted intrinsic — those would
    // pass mempool and then fail at block inclusion, polluting the pool.
    if config.is_amsterdam_activated(header.timestamp) {
        let fork = config.fork(header.timestamp);
        let (regular, state) = intrinsic_gas_dimensions(tx, fork, header.gas_limit)
            .map_err(|_| MempoolError::TxGasOverflowError)?;
        let intrinsic = regular
            .checked_add(state)
            .ok_or(MempoolError::TxGasOverflowError)?;
        let floor = intrinsic_gas_floor(tx, fork).map_err(|_| MempoolError::TxGasOverflowError)?;
        // Block-level gas = max(regular_dim, state_dim); regular_dim itself is
        // `max(tx_regular, calldata_floor)` per EIP-7778. Use the same max so
        // admission mirrors the VM's effective minimum.
        return Ok(intrinsic.max(floor));
    }

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
    use ethrex_common::types::{EIP1559Transaction, EIP4844Transaction};

    // --- helpers --------------------------------------------------------

    fn add_to_pool(pool: &Mempool, sender: Address, tx: Transaction) -> H256 {
        let mtx = MempoolTransaction::new(tx, sender);
        let hash = mtx.hash();
        pool.add_transaction(hash, sender, mtx).unwrap();
        hash
    }

    fn eip1559(nonce: u64, max_fee: u64, max_priority: u64) -> Transaction {
        Transaction::EIP1559Transaction(EIP1559Transaction {
            nonce,
            max_fee_per_gas: max_fee,
            max_priority_fee_per_gas: max_priority,
            ..Default::default()
        })
    }

    fn eip4844(nonce: u64, max_fee: u64, max_priority: u64, blob_fee: u64) -> Transaction {
        Transaction::EIP4844Transaction(EIP4844Transaction {
            nonce,
            max_fee_per_gas: max_fee,
            max_priority_fee_per_gas: max_priority,
            max_fee_per_blob_gas: U256::from(blob_fee),
            ..Default::default()
        })
    }

    // --- is_bumped_u64 -------------------------------------------------

    #[test]
    fn is_bumped_u64_exact_10_percent_accepted() {
        assert!(is_bumped_u64(100, 110, 10));
    }

    #[test]
    fn is_bumped_u64_just_below_10_percent_rejected() {
        assert!(!is_bumped_u64(100, 109, 10));
    }

    #[test]
    fn is_bumped_u64_zero_bump_allows_equal() {
        assert!(is_bumped_u64(100, 100, 0));
    }

    #[test]
    fn is_bumped_u64_zero_existing_always_accepted() {
        assert!(is_bumped_u64(0, 0, 100));
        assert!(is_bumped_u64(0, 1, 100));
    }

    #[test]
    fn is_bumped_u64_huge_existing_rejects_under_floor() {
        // At `existing = u64::MAX` the 100%-bumped threshold is ~3.69e19,
        // which doesn't fit in u64. Any new value (capped at u64::MAX,
        // ~1.84e19) is strictly below threshold and must be rejected.
        // The previous saturating-mul implementation silently *admitted*
        // this case because it saturated the threshold to u64::MAX/100;
        // the new checked-arithmetic implementation rejects, which is
        // the correct semantic.
        assert!(!is_bumped_u64(u64::MAX, u64::MAX, 100));
        // And the helper does not panic on extreme inputs.
        let _ = is_bumped_u64(u64::MAX, 0, u64::MAX);
    }

    // --- is_bumped_u256 ------------------------------------------------

    #[test]
    fn is_bumped_u256_blob_100_percent_accepted() {
        assert!(is_bumped_u256(U256::from(50u64), U256::from(100u64), 100));
    }

    #[test]
    fn is_bumped_u256_blob_99_percent_rejected() {
        assert!(!is_bumped_u256(U256::from(100u64), U256::from(199u64), 100));
    }

    // --- find_tx_to_replace ---------------------------------------------

    #[test]
    fn replacement_1_wei_bump_rejected_at_10_percent() {
        // Spec scenario: "Strict-greater-than but below 10% bump rejected"
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(1);
        let old = eip1559(0, 1_000, 100);
        add_to_pool(&pool, sender, old);

        let new = eip1559(0, 1_001, 100); // +1 wei on max_fee, no bump on tip
        let err = pool
            .find_tx_to_replace(sender, 0, &new, 10, 100)
            .unwrap_err();
        assert!(matches!(err, MempoolError::UnderpricedReplacement));
    }

    #[test]
    fn replacement_full_10_percent_bump_on_both_axes_accepted() {
        // Spec scenario: "10% bump on both axes accepted"
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(1);
        let old = eip1559(0, 1_000, 100);
        let old_hash = add_to_pool(&pool, sender, old);

        let new = eip1559(0, 1_100, 110);
        let found = pool
            .find_tx_to_replace(sender, 0, &new, 10, 100)
            .unwrap()
            .expect("replacement should be admitted");
        assert_eq!(found, old_hash);
    }

    #[test]
    fn replacement_asymmetric_bump_rejected() {
        // Spec scenario: "10% bump on only one axis rejected"
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(1);
        let old = eip1559(0, 1_000, 100);
        add_to_pool(&pool, sender, old);

        let new = eip1559(0, 1_100, 105); // 10% on fee cap, 5% on tip
        let err = pool
            .find_tx_to_replace(sender, 0, &new, 10, 100)
            .unwrap_err();
        assert!(matches!(err, MempoolError::UnderpricedReplacement));
    }

    #[test]
    fn blob_replacement_50_percent_bump_rejected() {
        // Spec scenario: "Blob 50% bump rejected"
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(2);
        let old = eip4844(0, 1_000, 100, 50);
        add_to_pool(&pool, sender, old);

        // 50% bump on all three fields, but blob threshold is 100%.
        let new = eip4844(0, 1_500, 150, 75);
        let err = pool
            .find_tx_to_replace(sender, 0, &new, 10, 100)
            .unwrap_err();
        assert!(matches!(err, MempoolError::UnderpricedReplacement));
    }

    #[test]
    fn blob_replacement_100_percent_bump_on_three_axes_accepted() {
        // Spec scenario: "Blob 100% bump on three axes accepted"
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(2);
        let old = eip4844(0, 1_000, 100, 50);
        let old_hash = add_to_pool(&pool, sender, old);

        let new = eip4844(0, 2_000, 200, 100);
        let found = pool
            .find_tx_to_replace(sender, 0, &new, 10, 100)
            .unwrap()
            .expect("blob replacement should be admitted");
        assert_eq!(found, old_hash);
    }

    #[test]
    fn no_existing_tx_returns_none() {
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(1);
        let new = eip1559(0, 1_000, 100);
        let res = pool.find_tx_to_replace(sender, 0, &new, 10, 100).unwrap();
        assert!(res.is_none());
    }

    // --- type-change rejection -----------------------------------------

    #[test]
    fn blob_cannot_be_replaced_by_non_blob() {
        // An EIP-4844 tx with a blob sidecar must not be displaced by a
        // cheaper non-blob tx at the same (sender, nonce), regardless of
        // the fee bump.
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(3);
        add_to_pool(&pool, sender, eip4844(0, 1_000, 100, 50));

        let new = eip1559(0, u64::MAX, u64::MAX); // very high non-blob fees
        let err = pool
            .find_tx_to_replace(sender, 0, &new, 10, 100)
            .unwrap_err();
        assert!(matches!(err, MempoolError::ReplacementTypeMismatch));
    }

    #[test]
    fn non_blob_cannot_be_replaced_by_blob() {
        // The inverse: a 1559 tx in the pool can't be replaced by a 4844
        // tx that suddenly demands sidecar handling.
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(3);
        add_to_pool(&pool, sender, eip1559(0, 1_000, 100));

        let new = eip4844(0, 2_000, 200, 100);
        let err = pool
            .find_tx_to_replace(sender, 0, &new, 10, 100)
            .unwrap_err();
        assert!(matches!(err, MempoolError::ReplacementTypeMismatch));
    }

    // --- legacy path ---------------------------------------------------

    #[test]
    fn legacy_replacement_requires_10_percent_bump() {
        // The legacy branch was missing test coverage. Pin it.
        let pool = Mempool::new(64);
        let sender = Address::from_low_u64_be(4);

        let old = Transaction::LegacyTransaction(ethrex_common::types::LegacyTransaction {
            nonce: 0,
            gas_price: U256::from(1_000u64),
            ..Default::default()
        });
        add_to_pool(&pool, sender, old);

        // 1-wei bump rejected.
        let too_small = Transaction::LegacyTransaction(ethrex_common::types::LegacyTransaction {
            nonce: 0,
            gas_price: U256::from(1_001u64),
            ..Default::default()
        });
        assert!(matches!(
            pool.find_tx_to_replace(sender, 0, &too_small, 10, 100)
                .unwrap_err(),
            MempoolError::UnderpricedReplacement
        ));

        // 10% bump accepted.
        let ok = Transaction::LegacyTransaction(ethrex_common::types::LegacyTransaction {
            nonce: 0,
            gas_price: U256::from(1_100u64),
            ..Default::default()
        });
        let found = pool
            .find_tx_to_replace(sender, 0, &ok, 10, 100)
            .unwrap()
            .expect("legacy replacement at 10% bump should be admitted");
        assert!(!found.is_zero());
    }
}
