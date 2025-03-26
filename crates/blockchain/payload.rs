use std::{
    cmp::{max, Ordering},
    collections::HashMap,
    ops::Div,
    time::Instant,
};

use ethrex_common::{
    constants::GAS_PER_BLOB,
    types::{
        calc_excess_blob_gas, calculate_base_fee_per_blob_gas, calculate_base_fee_per_gas,
        compute_receipts_root, compute_transactions_root, compute_withdrawals_root,
        requests::{compute_requests_hash, EncodedRequests},
        BlobsBundle, Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig,
        MempoolTransaction, Receipt, Transaction, Withdrawal, DEFAULT_OMMERS_HASH,
        DEFAULT_REQUESTS_HASH,
    },
    Address, Bloom, Bytes, H256, U256,
};

use ethrex_vm::{
    EvmError, {Evm, EvmEngine},
};

use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{error::StoreError, AccountUpdate, Store};

use sha3::{Digest, Keccak256};

use ethrex_metrics::metrics;

#[cfg(feature = "metrics")]
use ethrex_metrics::metrics_transactions::{MetricsTxStatus, MetricsTxType, METRICS_TX};

use crate::{
    constants::{GAS_LIMIT_BOUND_DIVISOR, MIN_GAS_LIMIT, TX_GAS_COST},
    error::{ChainError, InvalidBlockError},
    mempool::PendingTxFilter,
    Blockchain,
};

use tracing::{debug, error};

pub struct BuildPayloadArgs {
    pub parent: BlockHash,
    pub timestamp: u64,
    pub fee_recipient: Address,
    pub random: H256,
    pub withdrawals: Option<Vec<Withdrawal>>,
    pub beacon_root: Option<H256>,
    pub version: u8,
}

impl BuildPayloadArgs {
    /// Computes an 8-byte identifier by hashing the components of the payload arguments.
    pub fn id(&self) -> u64 {
        let mut hasher = Keccak256::new();
        hasher.update(self.parent);
        hasher.update(self.timestamp.to_be_bytes());
        hasher.update(self.random);
        hasher.update(self.fee_recipient);
        if let Some(withdrawals) = &self.withdrawals {
            hasher.update(withdrawals.encode_to_vec());
        }
        if let Some(beacon_root) = self.beacon_root {
            hasher.update(beacon_root);
        }
        let res = &mut hasher.finalize()[..8];
        res[0] = self.version;
        u64::from_be_bytes(res.try_into().unwrap())
    }
}

/// Creates a new payload based on the payload arguments
// Basic payload block building, can and should be improved
pub fn create_payload(args: &BuildPayloadArgs, storage: &Store) -> Result<Block, ChainError> {
    let parent_block = storage
        .get_block_header_by_hash(args.parent)?
        .ok_or_else(|| ChainError::ParentNotFound)?;
    let chain_config = storage.get_chain_config()?;
    let gas_limit = calc_gas_limit(parent_block.gas_limit);
    let excess_blob_gas = chain_config
        .get_fork_blob_schedule(args.timestamp)
        .map(|schedule| {
            calc_excess_blob_gas(
                parent_block.excess_blob_gas.unwrap_or_default(),
                parent_block.blob_gas_used.unwrap_or_default(),
                schedule.target,
            )
        });

    let header = BlockHeader {
        parent_hash: args.parent,
        ommers_hash: *DEFAULT_OMMERS_HASH,
        coinbase: args.fee_recipient,
        state_root: parent_block.state_root,
        transactions_root: compute_transactions_root(&[]),
        receipts_root: compute_receipts_root(&[]),
        logs_bloom: Bloom::default(),
        difficulty: U256::zero(),
        number: parent_block.number.saturating_add(1),
        gas_limit,
        gas_used: 0,
        timestamp: args.timestamp,
        // TODO: should use builder config's extra_data
        extra_data: Bytes::new(),
        prev_randao: args.random,
        nonce: 0,
        base_fee_per_gas: calculate_base_fee_per_gas(
            gas_limit,
            parent_block.gas_limit,
            parent_block.gas_used,
            parent_block.base_fee_per_gas.unwrap_or_default(),
        ),
        withdrawals_root: chain_config
            .is_shanghai_activated(args.timestamp)
            .then_some(compute_withdrawals_root(
                args.withdrawals.as_ref().unwrap_or(&Vec::new()),
            )),
        blob_gas_used: chain_config
            .is_cancun_activated(args.timestamp)
            .then_some(0),
        excess_blob_gas,
        parent_beacon_block_root: args.beacon_root,
        requests_hash: chain_config
            .is_prague_activated(args.timestamp)
            .then_some(*DEFAULT_REQUESTS_HASH),
    };

    let body = BlockBody {
        transactions: Vec::new(),
        ommers: Vec::new(),
        withdrawals: args.withdrawals.clone(),
    };

    // Delay applying withdrawals until the payload is requested and built
    Ok(Block::new(header, body))
}

pub fn calc_gas_limit(parent_gas_limit: u64) -> u64 {
    // TODO: check where we should get builder values from
    const DEFAULT_BUILDER_GAS_CEIL: u64 = 30_000_000;
    let delta = parent_gas_limit / GAS_LIMIT_BOUND_DIVISOR - 1;
    let mut limit = parent_gas_limit;
    let desired_limit = max(DEFAULT_BUILDER_GAS_CEIL, MIN_GAS_LIMIT);
    if limit < desired_limit {
        limit = parent_gas_limit + delta;
        if limit > desired_limit {
            limit = desired_limit
        }
        return limit;
    }
    if limit > desired_limit {
        limit = parent_gas_limit - delta;
        if limit < desired_limit {
            limit = desired_limit
        }
    }
    limit
}

pub struct PayloadBuildContext<'a> {
    pub payload: &'a mut Block,
    pub remaining_gas: u64,
    pub receipts: Vec<Receipt>,
    pub requests: Vec<EncodedRequests>,
    pub requests_hash: Option<H256>,
    pub block_value: U256,
    base_fee_per_blob_gas: U256,
    pub blobs_bundle: BlobsBundle,
    pub store: Store,
    pub vm: Evm,
    pub account_updates: Vec<AccountUpdate>,
}

impl<'a> PayloadBuildContext<'a> {
    fn new(
        payload: &'a mut Block,
        evm_engine: EvmEngine,
        storage: &Store,
    ) -> Result<Self, EvmError> {
        let config = storage.get_chain_config()?;
        let base_fee_per_blob_gas = calculate_base_fee_per_blob_gas(
            payload.header.excess_blob_gas.unwrap_or_default(),
            config
                .get_fork_blob_schedule(payload.header.timestamp)
                .map(|schedule| schedule.base_fee_update_fraction)
                .unwrap_or_default(),
        );
        let vm = Evm::new(evm_engine, storage.clone(), payload.header.parent_hash);

        Ok(PayloadBuildContext {
            remaining_gas: payload.header.gas_limit,
            receipts: vec![],
            requests: vec![],
            requests_hash: None,
            block_value: U256::zero(),
            base_fee_per_blob_gas: U256::from(base_fee_per_blob_gas),
            payload,
            blobs_bundle: BlobsBundle::default(),
            store: storage.clone(),
            vm,
            account_updates: Vec::new(),
        })
    }
}

impl<'a> PayloadBuildContext<'a> {
    fn parent_hash(&self) -> BlockHash {
        self.payload.header.parent_hash
    }

    fn block_number(&self) -> BlockNumber {
        self.payload.header.number
    }

    fn chain_config(&self) -> Result<ChainConfig, EvmError> {
        Ok(self.store.get_chain_config()?)
    }

    fn base_fee_per_gas(&self) -> Option<u64> {
        self.payload.header.base_fee_per_gas
    }
}

pub struct PayloadBuildResult {
    pub blobs_bundle: BlobsBundle,
    pub block_value: U256,
    pub receipts: Vec<Receipt>,
    pub requests: Vec<EncodedRequests>,
    pub account_updates: Vec<AccountUpdate>,
}

impl<'a> From<PayloadBuildContext<'a>> for PayloadBuildResult {
    fn from(value: PayloadBuildContext) -> Self {
        let PayloadBuildContext {
            blobs_bundle,
            block_value,
            requests,
            receipts,
            account_updates,
            ..
        } = value;

        Self {
            blobs_bundle,
            block_value,
            requests,
            receipts,
            account_updates,
        }
    }
}

impl Blockchain {
    /// Completes the payload building process, return the block value
    pub fn build_payload(&self, payload: &mut Block) -> Result<PayloadBuildResult, ChainError> {
        let since = Instant::now();
        let gas_limit = payload.header.gas_limit;

        debug!("Building payload");
        let mut context = PayloadBuildContext::new(payload, self.evm_engine, &self.storage)?;

        #[cfg(not(feature = "l2"))]
        self.apply_system_operations(&mut context)?;
        self.apply_withdrawals(&mut context)?;
        self.fill_transactions(&mut context)?;
        self.extract_requests(&mut context)?;
        self.finalize_payload(&mut context)?;

        let interval = Instant::now().duration_since(since).as_millis();
        tracing::info!("[METRIC] BUILDING PAYLOAD TOOK: {interval} ms");
        if let Some(gas_used) = gas_limit.checked_sub(context.remaining_gas) {
            let as_gigas = (gas_used as f64).div(10_f64.powf(9_f64));

            if interval != 0 {
                let throughput = (as_gigas) / (interval as f64) * 1000_f64;
                tracing::info!(
                "[METRIC] BLOCK BUILDING THROUGHPUT: {throughput} Gigagas/s TIME SPENT: {interval} msecs"
            );
            }
        }

        Ok(context.into())
    }

    fn apply_withdrawals(&self, context: &mut PayloadBuildContext) -> Result<(), EvmError> {
        let binding = Vec::new();
        let withdrawals = context
            .payload
            .body
            .withdrawals
            .as_ref()
            .unwrap_or(&binding);
        context
            .vm
            .process_withdrawals(withdrawals, &context.payload.header)
            .map_err(EvmError::from)
    }

    // This function applies system level operations:
    // - Call beacon root contract, and obtain the new state root
    // - Call block hash process contract, and store parent block hash
    pub fn apply_system_operations(
        &self,
        context: &mut PayloadBuildContext,
    ) -> Result<(), EvmError> {
        context.vm.apply_system_calls(&context.payload.header)
    }

    /// Fetches suitable transactions from the mempool
    /// Returns two transaction queues, one for plain and one for blob txs
    fn fetch_mempool_transactions(
        &self,
        context: &mut PayloadBuildContext,
    ) -> Result<(TransactionQueue, TransactionQueue), ChainError> {
        let tx_filter = PendingTxFilter {
            /*TODO(https://github.com/lambdaclass/ethrex/issues/680): add tip filter */
            base_fee: context.base_fee_per_gas(),
            blob_fee: Some(context.base_fee_per_blob_gas),
            ..Default::default()
        };
        let plain_tx_filter = PendingTxFilter {
            only_plain_txs: true,
            ..tx_filter
        };
        let blob_tx_filter = PendingTxFilter {
            only_blob_txs: true,
            ..tx_filter
        };
        Ok((
            // Plain txs
            TransactionQueue::new(
                self.mempool.filter_transactions(&plain_tx_filter)?,
                context.base_fee_per_gas(),
            )?,
            // Blob txs
            TransactionQueue::new(
                self.mempool.filter_transactions(&blob_tx_filter)?,
                context.base_fee_per_gas(),
            )?,
        ))
    }

    /// Fills the payload with transactions taken from the mempool
    /// Returns the block value
    pub fn fill_transactions(&self, context: &mut PayloadBuildContext) -> Result<(), ChainError> {
        let chain_config = context.chain_config()?;
        let max_blob_number_per_block = chain_config
            .get_fork_blob_schedule(context.payload.header.timestamp)
            .map(|schedule| schedule.max)
            .unwrap_or_default() as usize;

        debug!("Fetching transactions from mempool");
        // Fetch mempool transactions
        let (mut plain_txs, mut blob_txs) = self.fetch_mempool_transactions(context)?;
        // Execute and add transactions to payload (if suitable)
        loop {
            // Check if we have enough gas to run more transactions
            if context.remaining_gas < TX_GAS_COST {
                debug!("No more gas to run transactions");
                break;
            };
            if !blob_txs.is_empty() && context.blobs_bundle.blobs.len() >= max_blob_number_per_block
            {
                debug!("No more blob gas to run blob transactions");
                blob_txs.clear();
            }
            // Fetch the next transactions
            let (head_tx, is_blob) = match (plain_txs.peek(), blob_txs.peek()) {
                (None, None) => break,
                (None, Some(tx)) => (tx, true),
                (Some(tx), None) => (tx, false),
                (Some(a), Some(b)) if b < a => (b, true),
                (Some(tx), _) => (tx, false),
            };

            let txs = if is_blob {
                &mut blob_txs
            } else {
                &mut plain_txs
            };

            // Check if we have enough gas to run the transaction
            if context.remaining_gas < head_tx.tx.gas_limit() {
                debug!(
                    "Skipping transaction: {}, no gas left",
                    head_tx.tx.compute_hash()
                );
                // We don't have enough gas left for the transaction, so we skip all txs from this account
                txs.pop();
                continue;
            }

            // TODO: maybe fetch hash too when filtering mempool so we don't have to compute it here (we can do this in the same refactor as adding timestamp)
            let tx_hash = head_tx.tx.compute_hash();

            // Check wether the tx is replay-protected
            if head_tx.tx.protected() && !chain_config.is_eip155_activated(context.block_number()) {
                // Ignore replay protected tx & all txs from the sender
                // Pull transaction from the mempool
                debug!("Ignoring replay-protected transaction: {}", tx_hash);
                txs.pop();
                self.remove_transaction_from_pool(&head_tx.tx.compute_hash())?;
                continue;
            }

            // Increment the total transaction counter
            // CHECK: do we want it here to count every processed transaction
            // or we want it before the return?
            metrics!(METRICS_TX.inc_tx());

            // Execute tx
            let receipt = match self.apply_transaction(&head_tx, context) {
                Ok(receipt) => {
                    txs.shift()?;
                    // Pull transaction from the mempool
                    self.remove_transaction_from_pool(&head_tx.tx.compute_hash())?;

                    metrics!(METRICS_TX.inc_tx_with_status_and_type(
                        MetricsTxStatus::Succeeded,
                        MetricsTxType(head_tx.tx_type())
                    ));
                    receipt
                }
                // Ignore following txs from sender
                Err(e) => {
                    error!("Failed to execute transaction: {tx_hash:x}, {e}");
                    metrics!(METRICS_TX.inc_tx_with_status_and_type(
                        MetricsTxStatus::Failed,
                        MetricsTxType(head_tx.tx_type())
                    ));
                    txs.pop();
                    continue;
                }
            };
            // Add transaction to block
            debug!("Adding transaction: {} to payload", tx_hash);
            context.payload.body.transactions.push(head_tx.into());
            // Save receipt for hash calculation
            context.receipts.push(receipt);
        }
        Ok(())
    }

    /// Executes the transaction, updates gas-related context values & return the receipt
    /// The payload build context should have enough remaining gas to cover the transaction's gas_limit
    fn apply_transaction(
        &self,
        head: &HeadTransaction,
        context: &mut PayloadBuildContext,
    ) -> Result<Receipt, ChainError> {
        match **head {
            Transaction::EIP4844Transaction(_) => self.apply_blob_transaction(head, context),
            _ => self.apply_plain_transaction(head, context),
        }
    }

    /// Runs a blob transaction, updates the gas count & blob data and returns the receipt
    fn apply_blob_transaction(
        &self,
        head: &HeadTransaction,
        context: &mut PayloadBuildContext,
    ) -> Result<Receipt, ChainError> {
        // Fetch blobs bundle
        let tx_hash = head.tx.compute_hash();
        let chain_config = context.chain_config()?;
        let max_blob_number_per_block = chain_config
            .get_fork_blob_schedule(context.payload.header.timestamp)
            .map(|schedule| schedule.max)
            .unwrap_or_default() as usize;
        let Some(blobs_bundle) = self.mempool.get_blobs_bundle(tx_hash)? else {
            // No blob tx should enter the mempool without its blobs bundle so this is an internal error
            return Err(
                StoreError::Custom(format!("No blobs bundle found for blob tx {tx_hash}")).into(),
            );
        };
        if context.blobs_bundle.blobs.len() + blobs_bundle.blobs.len() > max_blob_number_per_block {
            // This error will only be used for debug tracing
            return Err(EvmError::Custom("max data blobs reached".to_string()).into());
        };
        // Apply transaction
        let receipt = self.apply_plain_transaction(head, context)?;
        // Update context with blob data
        let prev_blob_gas = context.payload.header.blob_gas_used.unwrap_or_default();
        context.payload.header.blob_gas_used =
            Some(prev_blob_gas + blobs_bundle.blobs.len() as u64 * GAS_PER_BLOB);
        context.blobs_bundle += blobs_bundle;
        Ok(receipt)
    }

    /// Runs a plain (non blob) transaction, updates the gas count and returns the receipt
    fn apply_plain_transaction(
        &self,
        head: &HeadTransaction,
        context: &mut PayloadBuildContext,
    ) -> Result<Receipt, ChainError> {
        let (report, gas_used) = context.vm.execute_tx(
            &head.tx,
            &context.payload.header,
            &mut context.remaining_gas,
            head.tx.sender(),
        )?;
        context.block_value += U256::from(gas_used) * head.tip;
        Ok(report)
    }

    pub fn extract_requests(&self, context: &mut PayloadBuildContext) -> Result<(), EvmError> {
        if !context
            .chain_config()?
            .is_prague_activated(context.payload.header.timestamp)
        {
            return Ok(());
        };

        let requests = context
            .vm
            .extract_requests(&context.receipts, &context.payload.header)?;

        context.requests = requests.iter().map(|r| r.encode()).collect();
        context.requests_hash = Some(compute_requests_hash(&context.requests));

        Ok(())
    }

    fn finalize_payload(&self, context: &mut PayloadBuildContext) -> Result<(), ChainError> {
        let parent_hash = context.payload.header.parent_hash;
        let account_updates = context.vm.get_state_transitions(parent_hash)?;

        context.payload.header.state_root = context
            .store
            .apply_account_updates(context.parent_hash(), &account_updates)?
            .unwrap_or_default();
        context.payload.header.transactions_root =
            compute_transactions_root(&context.payload.body.transactions);
        context.payload.header.receipts_root = compute_receipts_root(&context.receipts);
        context.payload.header.requests_hash = context.requests_hash;
        context.payload.header.gas_used = context.payload.header.gas_limit - context.remaining_gas;
        context.account_updates = account_updates;
        Ok(())
    }
}

/// A struct representing suitable mempool transactions waiting to be included in a block
// TODO: Consider using VecDequeue instead of Vec
struct TransactionQueue {
    // The first transaction for each account along with its tip, sorted by highest tip
    heads: Vec<HeadTransaction>,
    // The remaining txs grouped by account and sorted by nonce
    txs: HashMap<Address, Vec<MempoolTransaction>>,
    // Base Fee stored for tip calculations
    base_fee: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HeadTransaction {
    tx: MempoolTransaction,
    tip: u64,
}

impl std::ops::Deref for HeadTransaction {
    type Target = Transaction;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl From<HeadTransaction> for Transaction {
    fn from(val: HeadTransaction) -> Self {
        val.tx.into()
    }
}

impl TransactionQueue {
    /// Creates a new TransactionQueue from a set of transactions grouped by sender and sorted by nonce
    fn new(
        mut txs: HashMap<Address, Vec<MempoolTransaction>>,
        base_fee: Option<u64>,
    ) -> Result<Self, ChainError> {
        let mut heads = Vec::new();
        for (_, txs) in txs.iter_mut() {
            // Pull the first tx from each list and add it to the heads list
            // This should be a newly filtered tx list so we are guaranteed to have a first element
            let head_tx = txs.remove(0);
            heads.push(HeadTransaction {
                // We already ran this method when filtering the transactions from the mempool so it shouldn't fail
                tip: head_tx
                    .effective_gas_tip(base_fee)
                    .ok_or(ChainError::InvalidBlock(
                        InvalidBlockError::InvalidTransaction("Attempted to add an invalid transaction to the block. The transaction filter must have failed.".to_owned()),
                    ))?,
                tx: head_tx,
            });
        }
        // Sort heads by higest tip (and lowest timestamp if tip is equal)
        heads.sort();
        Ok(TransactionQueue {
            heads,
            txs,
            base_fee,
        })
    }

    /// Remove all transactions from the queue
    fn clear(&mut self) {
        self.heads.clear();
        self.txs.clear();
    }

    /// Returns true if there are no more transactions in the queue
    fn is_empty(&self) -> bool {
        self.heads.is_empty()
    }

    /// Returns the head transaction with the highest tip
    /// If there is more than one transaction with the highest tip, return the one with the lowest timestamp
    fn peek(&self) -> Option<HeadTransaction> {
        self.heads.first().cloned()
    }

    /// Removes current head transaction and all transactions from the given sender
    fn pop(&mut self) {
        if !self.is_empty() {
            let sender = self.heads.remove(0).tx.sender();
            self.txs.remove(&sender);
        }
    }

    /// Remove the top transaction
    /// Add a tx from the same sender to the head transactions
    fn shift(&mut self) -> Result<(), ChainError> {
        let tx = self.heads.remove(0);
        if let Some(txs) = self.txs.get_mut(&tx.tx.sender()) {
            // Fetch next head
            if !txs.is_empty() {
                let head_tx = txs.remove(0);
                let head = HeadTransaction {
                    // We already ran this method when filtering the transactions from the mempool so it shouldn't fail
                    tip: head_tx.effective_gas_tip(self.base_fee).ok_or(
                        ChainError::InvalidBlock(
                            InvalidBlockError::InvalidTransaction("Attempted to add an invalid transaction to the block. The transaction filter must have failed.".to_owned()),
                        ),
                    )?,
                    tx: head_tx,
                };
                // Insert head into heads list while maintaing order
                let index = match self.heads.binary_search(&head) {
                    Ok(index) => index, // Same ordering shouldn't be possible when adding timestamps
                    Err(index) => index,
                };
                self.heads.insert(index, head);
            }
        }
        Ok(())
    }
}

// Orders transactions by highest tip, if tip is equal, orders by lowest timestamp
impl Ord for HeadTransaction {
    fn cmp(&self, other: &Self) -> Ordering {
        match other.tip.cmp(&self.tip) {
            Ordering::Equal => self.tx.time().cmp(&other.tx.time()),
            ordering => ordering,
        }
    }
}

impl PartialOrd for HeadTransaction {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
