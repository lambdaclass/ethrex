use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, RwLock},
};

use crate::{
    constants::{
        MAX_INITCODE_SIZE, TX_ACCESS_LIST_ADDRESS_GAS, TX_ACCESS_LIST_STORAGE_KEY_GAS,
        TX_CREATE_GAS_COST, TX_DATA_NON_ZERO_GAS, TX_DATA_NON_ZERO_GAS_EIP2028,
        TX_DATA_ZERO_GAS_COST, TX_GAS_COST, TX_INIT_CODE_WORD_GAS_COST,
    },
    error::MempoolError,
};
use ethrex_common::{
    constants::MIN_BASE_FEE_PER_BLOB_GAS,
    types::{
        BlobsBundle, BlockHeader, ChainConfig, EIP4844Transaction, MempoolTransaction, Transaction,
        TxType,
    },
    Address, H256, U256,
};
use ethrex_storage::{error::StoreError, Store};

#[derive(Debug, Clone, Default)]
pub struct Mempool {
    pub transaction_pool: Arc<RwLock<HashMap<H256, MempoolTransaction>>>,
    pub blobs_bundle_pool: Arc<Mutex<HashMap<H256, BlobsBundle>>>,
}
impl Mempool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add transaction to the pool
    pub fn add_transaction_to_pool(
        &self,
        hash: H256,
        transaction: MempoolTransaction,
    ) -> Result<(), StoreError> {
        let mut tx_pool = self
            .transaction_pool
            .write()
            .map_err(|error| StoreError::MempoolWriteLock(error.to_string()))?;
        tx_pool.insert(hash, transaction);

        Ok(())
    }

    /// Add a blobs bundle to the pool by its blob transaction hash
    pub fn add_blobs_bundle_to_pool(
        &self,
        tx_hash: H256,
        blobs_bundle: BlobsBundle,
    ) -> Result<(), StoreError> {
        self.blobs_bundle_pool
            .lock()
            .map_err(|error| StoreError::Custom(error.to_string()))?
            .insert(tx_hash, blobs_bundle);
        Ok(())
    }

    /// Get a blobs bundle to the pool given its blob transaction hash
    pub fn get_blobs_bundle_from_pool(
        &self,
        tx_hash: H256,
    ) -> Result<Option<BlobsBundle>, StoreError> {
        Ok(self
            .blobs_bundle_pool
            .lock()
            .map_err(|error| StoreError::Custom(error.to_string()))?
            .get(&tx_hash)
            .cloned())
    }

    /// Remove a transaction from the pool
    pub fn remove_transaction_from_pool(&self, hash: &H256) -> Result<(), StoreError> {
        let mut tx_pool = self
            .transaction_pool
            .write()
            .map_err(|error| StoreError::MempoolWriteLock(error.to_string()))?;
        if let Some(tx) = tx_pool.get(hash) {
            if matches!(tx.tx_type(), TxType::EIP4844) {
                self.blobs_bundle_pool
                    .lock()
                    .map_err(|error| StoreError::Custom(error.to_string()))?
                    .remove(&tx.compute_hash());
            }

            tx_pool.remove(hash);
        };

        Ok(())
    }

    pub fn remove_transactions_from_pool(&self, filter: &[Transaction]) -> Result<(), StoreError> {
        let mut tx_pool = self
            .transaction_pool
            .write()
            .map_err(|err| StoreError::MempoolWriteLock(err.to_string()))?;
        for tx in filter {
            tx_pool.remove(&tx.compute_hash());
        }
        Ok(())
    }

    /// Applies the filter and returns a set of suitable transactions from the mempool.
    /// These transactions will be grouped by sender and sorted by nonce
    pub fn filter_pool_transactions(
        &self,
        filter: &dyn Fn(&Transaction) -> bool,
    ) -> Result<HashMap<Address, Vec<MempoolTransaction>>, StoreError> {
        let mut txs_by_sender: HashMap<Address, Vec<MempoolTransaction>> = HashMap::new();
        let tx_pool = self
            .transaction_pool
            .read()
            .map_err(|error| StoreError::MempoolReadLock(error.to_string()))?;

        for (_, tx) in tx_pool.iter() {
            if filter(tx) {
                txs_by_sender
                    .entry(tx.sender())
                    .or_default()
                    .push(tx.clone())
            }
        }

        txs_by_sender.iter_mut().for_each(|(_, txs)| txs.sort());
        Ok(txs_by_sender)
    }

    /// Gets hashes from possible_hashes that are not already known in the mempool.
    pub fn filter_unknown_transactions(
        &self,
        possible_hashes: &[H256],
    ) -> Result<Vec<H256>, StoreError> {
        let tx_pool = self
            .transaction_pool
            .read()
            .map_err(|error| StoreError::MempoolReadLock(error.to_string()))?;

        let tx_set: HashSet<_> = tx_pool.iter().map(|(hash, _)| hash).collect();
        Ok(possible_hashes
            .iter()
            .filter(|hash| !tx_set.contains(hash))
            .copied()
            .collect())
    }

    pub fn get_transaction_by_hash_from_pool(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        let tx = self
            .transaction_pool
            .read()
            .map_err(|error| StoreError::MempoolReadLock(error.to_string()))?
            .get(&transaction_hash)
            .map(|e| e.clone().into());

        Ok(tx)
    }
}

/// Add a blob transaction and its blobs bundle to the mempool
#[cfg(feature = "c-kzg")]
pub fn add_blob_transaction(
    transaction: EIP4844Transaction,
    blobs_bundle: BlobsBundle,
    store: &Store,
    mempool: &Mempool,
) -> Result<H256, MempoolError> {
    // Validate blobs bundle
    blobs_bundle.validate(&transaction)?;

    let transaction = Transaction::EIP4844Transaction(transaction);
    let sender = transaction.sender();

    // Validate transaction
    validate_transaction(&transaction, sender, store.clone())?;

    // Add transaction and blobs bundle to storage
    let hash = transaction.compute_hash();
    mempool.add_transaction_to_pool(hash, MempoolTransaction::new(transaction, sender))?;
    mempool.add_blobs_bundle_to_pool(hash, blobs_bundle)?;
    Ok(hash)
}

/// Add a transaction to the mempool
pub fn add_transaction(
    transaction: Transaction,
    store: &Store,
    mempool: &Mempool,
) -> Result<H256, MempoolError> {
    // Blob transactions should be submitted via add_blob_transaction along with the corresponding blobs bundle
    if matches!(transaction, Transaction::EIP4844Transaction(_)) {
        return Err(MempoolError::BlobTxNoBlobsBundle);
    }
    let sender = transaction.sender();
    // Validate transaction
    validate_transaction(&transaction, sender, store.clone())?;

    let hash = transaction.compute_hash();

    // Add transaction to storage
    mempool.add_transaction_to_pool(hash, MempoolTransaction::new(transaction, sender))?;

    Ok(hash)
}

/// Fetch a blobs bundle from the mempool given its blob transaction hash
pub fn get_blobs_bundle(
    tx_hash: H256,
    mempool: &Mempool,
) -> Result<Option<BlobsBundle>, MempoolError> {
    Ok(mempool.get_blobs_bundle_from_pool(tx_hash)?)
}

/// Applies the filter and returns a set of suitable transactions from the mempool.
/// These transactions will be grouped by sender and sorted by nonce
pub fn filter_transactions(
    filter: &PendingTxFilter,
    mempool: &Mempool,
) -> Result<HashMap<Address, Vec<MempoolTransaction>>, StoreError> {
    let filter_tx = |tx: &Transaction| -> bool {
        // Filter by tx type
        let is_blob_tx = matches!(tx, Transaction::EIP4844Transaction(_));
        if filter.only_plain_txs && is_blob_tx || filter.only_blob_txs && !is_blob_tx {
            return false;
        }

        // Filter by tip & base_fee
        if let Some(min_tip) = filter.min_tip {
            if !tx
                .effective_gas_tip(filter.base_fee)
                .is_some_and(|tip| tip >= min_tip)
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
        if let (true, Some(blob_fee)) = (is_blob_tx, filter.blob_fee) {
            if !tx.max_fee_per_blob_gas().is_some_and(|fee| fee >= blob_fee) {
                return false;
            }
        }
        true
    };
    mempool.filter_pool_transactions(&filter_tx)
}

/// Remove a transaction from the mempool
pub fn remove_transaction(hash: &H256, mempool: &Mempool) -> Result<(), StoreError> {
    mempool.remove_transaction_from_pool(hash)
}

pub fn get_nonce(address: &Address, mempool: &Mempool) -> Result<Option<u64>, MempoolError> {
    let pending_filter = PendingTxFilter {
        min_tip: None,
        base_fee: None,
        blob_fee: None,
        only_plain_txs: false,
        only_blob_txs: false,
    };

    let pending_txs = filter_transactions(&pending_filter, mempool)?;
    let nonce = match pending_txs.get(address) {
        Some(txs) => txs.last().map(|tx| tx.nonce() + 1),
        None => None,
    };

    Ok(nonce)
}

#[derive(Debug, Default)]
pub struct PendingTxFilter {
    pub min_tip: Option<u64>,
    pub base_fee: Option<u64>,
    pub blob_fee: Option<U256>,
    pub only_plain_txs: bool,
    pub only_blob_txs: bool,
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

fn validate_transaction(
    tx: &Transaction,
    sender: Address,
    store: Store,
) -> Result<(), MempoolError> {
    // TODO: Add validations here

    let header_no = store.get_latest_block_number()?;
    let header = store
        .get_block_header(header_no)?
        .ok_or(MempoolError::NoBlockHeaderError)?;
    let config = store.get_chain_config()?;

    // NOTE: We could add a tx size limit here, but it's not in the actual spec

    // Check init code size
    if config.is_shanghai_activated(header.timestamp)
        && tx.is_contract_creation()
        && tx.data().len() > MAX_INITCODE_SIZE
    {
        return Err(MempoolError::TxMaxInitCodeSizeError);
    }

    // Check gas limit is less than header's gas limit
    if header.gas_limit < tx.gas_limit() {
        return Err(MempoolError::TxGasLimitExceededError);
    }

    // Check priority fee is less or equal than gas fee gap
    if tx.max_priority_fee().unwrap_or(0) > tx.max_fee_per_gas().unwrap_or(0) {
        return Err(MempoolError::TxTipAboveFeeCapError);
    }

    // Check that the gas limit is covers the gas needs for transaction metadata.
    if tx.gas_limit() < transaction_intrinsic_gas(tx, &header, &config)? {
        return Err(MempoolError::TxIntrinsicGasCostAboveLimitError);
    }

    // Check that the specified blob gas fee is above the minimum value
    if let Some(fee) = tx.max_fee_per_blob_gas() {
        // Blob tx fee checks
        if fee < MIN_BASE_FEE_PER_BLOB_GAS.into() {
            return Err(MempoolError::TxBlobBaseFeeTooLowError);
        }
    };

    let maybe_sender_acc_info = store.get_account_info(header_no, sender)?;

    if let Some(sender_acc_info) = maybe_sender_acc_info {
        if tx.nonce() < sender_acc_info.nonce {
            return Err(MempoolError::InvalidNonce);
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

    if let Some(chain_id) = tx.chain_id() {
        if chain_id != config.chain_id {
            return Err(MempoolError::InvalidChainId(config.chain_id));
        }
    }

    Ok(())
}

fn transaction_intrinsic_gas(
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
    use crate::error::MempoolError;
    use crate::mempool::{
        Mempool, MAX_INITCODE_SIZE, TX_ACCESS_LIST_ADDRESS_GAS, TX_ACCESS_LIST_STORAGE_KEY_GAS,
        TX_CREATE_GAS_COST, TX_DATA_NON_ZERO_GAS, TX_DATA_NON_ZERO_GAS_EIP2028,
        TX_DATA_ZERO_GAS_COST, TX_GAS_COST, TX_INIT_CODE_WORD_GAS_COST,
    };
    use std::collections::HashMap;

    use super::{transaction_intrinsic_gas, validate_transaction};
    use ethrex_common::types::{
        BlobsBundle, BlockHeader, ChainConfig, EIP1559Transaction, EIP4844Transaction,
        MempoolTransaction, Transaction, TxKind, BYTES_PER_BLOB,
    };
    use ethrex_common::{Address, Bytes, H256, U256};
    use ethrex_storage::EngineType;
    use ethrex_storage::{error::StoreError, Store};

    fn setup_storage(config: ChainConfig, header: BlockHeader) -> Result<Store, StoreError> {
        let store = Store::new("test", EngineType::InMemory)?;
        let block_number = header.number;
        let block_hash = header.compute_block_hash();
        store.add_block_header(block_hash, header)?;
        store.set_canonical_block(block_number, block_hash)?;
        store.update_latest_block_number(block_number)?;
        store.set_chain_config(&config)?;

        Ok(store)
    }

    fn build_basic_config_and_header(
        istanbul_active: bool,
        shanghai_active: bool,
    ) -> (ChainConfig, BlockHeader) {
        let config = ChainConfig {
            shanghai_time: Some(if shanghai_active { 1 } else { 10 }),
            istanbul_block: Some(if istanbul_active { 1 } else { 10 }),
            ..Default::default()
        };

        let header = BlockHeader {
            number: 5,
            timestamp: 5,
            gas_limit: 100_000_000,
            gas_used: 0,
            ..Default::default()
        };

        (config, header)
    }

    #[test]
    fn normal_transaction_intrinsic_gas() {
        let (config, header) = build_basic_config_and_header(false, false);

        let tx = EIP1559Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            gas_limit: 100_000,
            to: TxKind::Call(Address::from_low_u64_be(1)), // Normal tx
            value: U256::zero(),                           // Value zero
            data: Bytes::default(),                        // No data
            access_list: Default::default(),               // No access list
            ..Default::default()
        };

        let tx = Transaction::EIP1559Transaction(tx);
        let expected_gas_cost = TX_GAS_COST;
        let intrinsic_gas =
            transaction_intrinsic_gas(&tx, &header, &config).expect("Intrinsic gas");
        assert_eq!(intrinsic_gas, expected_gas_cost);
    }

    #[test]
    fn create_transaction_intrinsic_gas() {
        let (config, header) = build_basic_config_and_header(false, false);

        let tx = EIP1559Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            gas_limit: 100_000,
            to: TxKind::Create,              // Create tx
            value: U256::zero(),             // Value zero
            data: Bytes::default(),          // No data
            access_list: Default::default(), // No access list
            ..Default::default()
        };

        let tx = Transaction::EIP1559Transaction(tx);
        let expected_gas_cost = TX_CREATE_GAS_COST;
        let intrinsic_gas =
            transaction_intrinsic_gas(&tx, &header, &config).expect("Intrinsic gas");
        assert_eq!(intrinsic_gas, expected_gas_cost);
    }

    #[test]
    fn transaction_intrinsic_data_gas_pre_istanbul() {
        let (config, header) = build_basic_config_and_header(false, false);

        let tx = EIP1559Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            gas_limit: 100_000,
            to: TxKind::Call(Address::from_low_u64_be(1)), // Normal tx
            value: U256::zero(),                           // Value zero
            data: Bytes::from(vec![0x0, 0x1, 0x1, 0x0, 0x1, 0x1]), // 6 bytes of data
            access_list: Default::default(),               // No access list
            ..Default::default()
        };

        let tx = Transaction::EIP1559Transaction(tx);
        let expected_gas_cost = TX_GAS_COST + 2 * TX_DATA_ZERO_GAS_COST + 4 * TX_DATA_NON_ZERO_GAS;
        let intrinsic_gas =
            transaction_intrinsic_gas(&tx, &header, &config).expect("Intrinsic gas");
        assert_eq!(intrinsic_gas, expected_gas_cost);
    }

    #[test]
    fn transaction_intrinsic_data_gas_post_istanbul() {
        let (config, header) = build_basic_config_and_header(true, false);

        let tx = EIP1559Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            gas_limit: 100_000,
            to: TxKind::Call(Address::from_low_u64_be(1)), // Normal tx
            value: U256::zero(),                           // Value zero
            data: Bytes::from(vec![0x0, 0x1, 0x1, 0x0, 0x1, 0x1]), // 6 bytes of data
            access_list: Default::default(),               // No access list
            ..Default::default()
        };

        let tx = Transaction::EIP1559Transaction(tx);
        let expected_gas_cost =
            TX_GAS_COST + 2 * TX_DATA_ZERO_GAS_COST + 4 * TX_DATA_NON_ZERO_GAS_EIP2028;
        let intrinsic_gas =
            transaction_intrinsic_gas(&tx, &header, &config).expect("Intrinsic gas");
        assert_eq!(intrinsic_gas, expected_gas_cost);
    }

    #[test]
    fn transaction_create_intrinsic_gas_pre_shanghai() {
        let (config, header) = build_basic_config_and_header(false, false);

        let n_words: u64 = 10;
        let n_bytes: u64 = 32 * n_words - 3; // Test word rounding

        let tx = EIP1559Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            gas_limit: 100_000,
            to: TxKind::Create,                                // Create tx
            value: U256::zero(),                               // Value zero
            data: Bytes::from(vec![0x1_u8; n_bytes as usize]), // Bytecode data
            access_list: Default::default(),                   // No access list
            ..Default::default()
        };

        let tx = Transaction::EIP1559Transaction(tx);
        let expected_gas_cost = TX_CREATE_GAS_COST + n_bytes * TX_DATA_NON_ZERO_GAS;
        let intrinsic_gas =
            transaction_intrinsic_gas(&tx, &header, &config).expect("Intrinsic gas");
        assert_eq!(intrinsic_gas, expected_gas_cost);
    }

    #[test]
    fn transaction_create_intrinsic_gas_post_shanghai() {
        let (config, header) = build_basic_config_and_header(false, true);

        let n_words: u64 = 10;
        let n_bytes: u64 = 32 * n_words - 3; // Test word rounding

        let tx = EIP1559Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            gas_limit: 100_000,
            to: TxKind::Create,                                // Create tx
            value: U256::zero(),                               // Value zero
            data: Bytes::from(vec![0x1_u8; n_bytes as usize]), // Bytecode data
            access_list: Default::default(),                   // No access list
            ..Default::default()
        };

        let tx = Transaction::EIP1559Transaction(tx);
        let expected_gas_cost = TX_CREATE_GAS_COST
            + n_bytes * TX_DATA_NON_ZERO_GAS
            + n_words * TX_INIT_CODE_WORD_GAS_COST;
        let intrinsic_gas =
            transaction_intrinsic_gas(&tx, &header, &config).expect("Intrinsic gas");
        assert_eq!(intrinsic_gas, expected_gas_cost);
    }

    #[test]
    fn transaction_intrinsic_gas_access_list() {
        let (config, header) = build_basic_config_and_header(false, false);

        let access_list = vec![
            (Address::zero(), vec![H256::default(); 10]),
            (Address::zero(), vec![]),
            (Address::zero(), vec![H256::default(); 5]),
        ];

        let tx = EIP1559Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            gas_limit: 100_000,
            to: TxKind::Call(Address::from_low_u64_be(1)), // Normal tx
            value: U256::zero(),                           // Value zero
            data: Bytes::default(),                        // No data
            access_list,
            ..Default::default()
        };

        let tx = Transaction::EIP1559Transaction(tx);
        let expected_gas_cost =
            TX_GAS_COST + 3 * TX_ACCESS_LIST_ADDRESS_GAS + 15 * TX_ACCESS_LIST_STORAGE_KEY_GAS;
        let intrinsic_gas =
            transaction_intrinsic_gas(&tx, &header, &config).expect("Intrinsic gas");
        assert_eq!(intrinsic_gas, expected_gas_cost);
    }

    #[test]
    fn transaction_with_big_init_code_in_shanghai_fails() {
        let (config, header) = build_basic_config_and_header(false, true);

        let store = setup_storage(config, header).expect("Storage setup");

        let tx = EIP1559Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            gas_limit: 99_000_000,
            to: TxKind::Create,                                  // Create tx
            value: U256::zero(),                                 // Value zero
            data: Bytes::from(vec![0x1; MAX_INITCODE_SIZE + 1]), // Large init code
            access_list: Default::default(),                     // No access list
            ..Default::default()
        };

        let tx = Transaction::EIP1559Transaction(tx);
        let validation = validate_transaction(&tx, Address::random(), store);
        assert!(matches!(
            validation,
            Err(MempoolError::TxMaxInitCodeSizeError)
        ));
    }

    #[test]
    fn transaction_with_gas_limit_higher_than_of_the_block_should_fail() {
        let (config, header) = build_basic_config_and_header(false, false);

        let store = setup_storage(config, header).expect("Storage setup");

        let tx = EIP1559Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            gas_limit: 100_000_001,
            to: TxKind::Call(Address::from_low_u64_be(1)), // Normal tx
            value: U256::zero(),                           // Value zero
            data: Bytes::default(),                        // No data
            access_list: Default::default(),               // No access list
            ..Default::default()
        };

        let tx = Transaction::EIP1559Transaction(tx);
        let validation = validate_transaction(&tx, Address::random(), store);
        assert!(matches!(
            validation,
            Err(MempoolError::TxGasLimitExceededError)
        ));
    }

    #[test]
    fn transaction_with_priority_fee_higher_than_gas_fee_should_fail() {
        let (config, header) = build_basic_config_and_header(false, false);

        let store = setup_storage(config, header).expect("Storage setup");

        let tx = EIP1559Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 101,
            max_fee_per_gas: 100,
            gas_limit: 50_000_000,
            to: TxKind::Call(Address::from_low_u64_be(1)), // Normal tx
            value: U256::zero(),                           // Value zero
            data: Bytes::default(),                        // No data
            access_list: Default::default(),               // No access list
            ..Default::default()
        };

        let tx = Transaction::EIP1559Transaction(tx);
        let validation = validate_transaction(&tx, Address::random(), store);
        assert!(matches!(
            validation,
            Err(MempoolError::TxTipAboveFeeCapError)
        ));
    }

    #[test]
    fn transaction_with_gas_limit_lower_than_intrinsic_gas_should_fail() {
        let (config, header) = build_basic_config_and_header(false, false);
        let store = setup_storage(config, header).expect("Storage setup");

        let intrinsic_gas_cost = TX_GAS_COST;

        let tx = EIP1559Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            gas_limit: intrinsic_gas_cost - 1,
            to: TxKind::Call(Address::from_low_u64_be(1)), // Normal tx
            value: U256::zero(),                           // Value zero
            data: Bytes::default(),                        // No data
            access_list: Default::default(),               // No access list
            ..Default::default()
        };

        let tx = Transaction::EIP1559Transaction(tx);
        let validation = validate_transaction(&tx, Address::random(), store);
        assert!(matches!(
            validation,
            Err(MempoolError::TxIntrinsicGasCostAboveLimitError)
        ));
    }

    #[test]
    fn transaction_with_blob_base_fee_below_min_should_fail() {
        let (config, header) = build_basic_config_and_header(false, false);
        let store = setup_storage(config, header).expect("Storage setup");

        let tx = EIP4844Transaction {
            nonce: 3,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 0,
            max_fee_per_blob_gas: 0.into(),
            gas: 15_000_000,
            to: Address::from_low_u64_be(1), // Normal tx
            value: U256::zero(),             // Value zero
            data: Bytes::default(),          // No data
            access_list: Default::default(), // No access list
            ..Default::default()
        };

        let tx = Transaction::EIP4844Transaction(tx);
        let validation = validate_transaction(&tx, Address::random(), store);
        assert!(matches!(
            validation,
            Err(MempoolError::TxBlobBaseFeeTooLowError)
        ));
    }
    use hex_literal::hex;

    #[test]
    fn test_filter_mempool_transactions() {
        let plain_tx_decoded = Transaction::decode_canonical(&hex!("f86d80843baa0c4082f618946177843db3138ae69679a54b95cf345ed759450d870aa87bee538000808360306ba0151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65da064c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4")).unwrap();
        let plain_tx_sender = plain_tx_decoded.sender();
        let plain_tx = MempoolTransaction::new(plain_tx_decoded, plain_tx_sender);
        let blob_tx_decoded = Transaction::decode_canonical(&hex!("03f88f0780843b9aca008506fc23ac00830186a09400000000000000000000000000000000000001008080c001e1a0010657f37554c781402a22917dee2f75def7ab966d7b770905398eba3c44401401a0840650aa8f74d2b07f40067dc33b715078d73422f01da17abdbd11e02bbdfda9a04b2260f6022bf53eadb337b3e59514936f7317d872defb891a708ee279bdca90")).unwrap();
        let blob_tx_sender = blob_tx_decoded.sender();
        let blob_tx = MempoolTransaction::new(blob_tx_decoded, blob_tx_sender);
        let plain_tx_hash = plain_tx.compute_hash();
        let blob_tx_hash = blob_tx.compute_hash();
        let mempool = Mempool::new();
        let filter =
            |tx: &Transaction| -> bool { matches!(tx, Transaction::EIP4844Transaction(_)) };
        mempool
            .add_transaction_to_pool(blob_tx_hash, blob_tx.clone())
            .unwrap();
        mempool
            .add_transaction_to_pool(plain_tx_hash, plain_tx)
            .unwrap();
        let txs = mempool.filter_pool_transactions(&filter).unwrap();
        assert_eq!(txs, HashMap::from([(blob_tx.sender(), vec![blob_tx])]));
    }

    #[test]
    fn blobs_bundle_loadtest() {
        // Write a bundle of 6 blobs 10 times
        // If this test fails please adjust the max_size in the DB config
        let mempool = Mempool::new();
        for i in 0..300 {
            let blobs = [[i as u8; BYTES_PER_BLOB]; 6];
            let commitments = [[i as u8; 48]; 6];
            let proofs = [[i as u8; 48]; 6];
            let bundle = BlobsBundle {
                blobs: blobs.to_vec(),
                commitments: commitments.to_vec(),
                proofs: proofs.to_vec(),
            };
            mempool
                .add_blobs_bundle_to_pool(H256::random(), bundle)
                .unwrap();
        }
    }
}
