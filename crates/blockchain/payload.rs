use std::{
    cmp::{Ordering, max},
    collections::HashMap,
    ops::Div,
    sync::Arc,
    time::{Duration, Instant},
};

use ethrex_common::{
    Address, Bloom, Bytes, H256, U256,
    constants::{DEFAULT_OMMERS_HASH, DEFAULT_REQUESTS_HASH, GAS_PER_BLOB},
    types::{
        AccountUpdate, BlobsBundle, Block, BlockBody, BlockHash, BlockHeader, BlockNumber,
        ChainConfig, MempoolTransaction, Receipt, Transaction, TxType, Withdrawal, bloom_from_logs,
        calc_excess_blob_gas, calculate_base_fee_per_blob_gas, calculate_base_fee_per_gas,
        compute_receipts_root, compute_transactions_root, compute_withdrawals_root,
        requests::{EncodedRequests, compute_requests_hash},
    },
};

use ethrex_vm::{Evm, EvmError};

use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{Store, error::StoreError};

use sha3::{Digest, Keccak256};

use ethrex_metrics::metrics;

#[cfg(feature = "metrics")]
use ethrex_metrics::metrics_transactions::{METRICS_TX, MetricsTxType};
use tokio_util::sync::CancellationToken;

use crate::{
    Blockchain, BlockchainType, MAX_PAYLOADS,
    constants::{GAS_LIMIT_BOUND_DIVISOR, MIN_GAS_LIMIT, TX_GAS_COST},
    error::{ChainError, InvalidBlockError},
    mempool::PendingTxFilter,
    vm::StoreVmDatabase,
};

use thiserror::Error;
use tracing::{debug, error};

#[derive(Debug)]
pub struct PayloadBuildTask {
    task: tokio::task::JoinHandle<Result<PayloadBuildResult, ChainError>>,
    cancel: CancellationToken,
}

#[derive(Debug)]
pub enum PayloadOrTask {
    Payload(Box<PayloadBuildResult>),
    Task(PayloadBuildTask),
}

impl PayloadBuildTask {
    /// Finishes the current payload build process and returns its result
    pub async fn finish(self) -> Result<PayloadBuildResult, ChainError> {
        self.cancel.cancel();
        self.task
            .await
            .map_err(|_| ChainError::Custom("Failed to join task".to_string()))?
    }
}

impl PayloadOrTask {
    /// Converts self into a `PayloadOrTask::Payload` by finishing the current build task
    /// If self is already a `PayloadOrTask::Payload` this is a NoOp
    pub async fn to_payload(self) -> Result<Self, ChainError> {
        Ok(match self {
            PayloadOrTask::Payload(_) => self,
            PayloadOrTask::Task(task) => PayloadOrTask::Payload(Box::new(task.finish().await?)),
        })
    }
}

pub struct BuildPayloadArgs {
    pub parent: BlockHash,
    pub timestamp: u64,
    pub fee_recipient: Address,
    pub random: H256,
    pub withdrawals: Option<Vec<Withdrawal>>,
    pub beacon_root: Option<H256>,
    pub version: u8,
    pub elasticity_multiplier: u64,
    pub gas_ceil: u64,
}

#[derive(Debug, Error)]
pub enum BuildPayloadArgsError {
    #[error("Payload hashed has wrong size")]
    FailedToConvertPayload,
}

impl BuildPayloadArgs {
    /// Computes an 8-byte identifier by hashing the components of the payload arguments.
    pub fn id(&self) -> Result<u64, BuildPayloadArgsError> {
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
        Ok(u64::from_be_bytes(res.try_into().map_err(|_| {
            BuildPayloadArgsError::FailedToConvertPayload
        })?))
    }
}

/// Creates a new payload based on the payload arguments
// Basic payload block building, can and should be improved
pub fn create_payload(args: &BuildPayloadArgs, storage: &Store) -> Result<Block, ChainError> {
    let parent_block = storage
        .get_block_header_by_hash(args.parent)?
        .ok_or_else(|| ChainError::ParentNotFound)?;
    let chain_config = storage.get_chain_config()?;
    let fork = chain_config.fork(args.timestamp);
    let gas_limit = calc_gas_limit(parent_block.gas_limit, args.gas_ceil);
    let excess_blob_gas = chain_config
        .get_fork_blob_schedule(args.timestamp)
        .map(|schedule| calc_excess_blob_gas(&parent_block, schedule, fork));

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
            args.elasticity_multiplier,
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
        ..Default::default()
    };

    let body = BlockBody {
        transactions: Vec::new(),
        ommers: Vec::new(),
        withdrawals: args.withdrawals.clone(),
    };

    // Delay applying withdrawals until the payload is requested and built
    Ok(Block::new(header, body))
}

pub fn calc_gas_limit(parent_gas_limit: u64, builder_gas_ceil: u64) -> u64 {
    // TODO: check where we should get builder values from
    let delta = parent_gas_limit / GAS_LIMIT_BOUND_DIVISOR - 1;
    let mut limit = parent_gas_limit;
    let desired_limit = max(builder_gas_ceil, MIN_GAS_LIMIT);
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

#[derive(Clone)]
pub struct PayloadBuildContext {
    pub payload: Block,
    pub remaining_gas: u64,
    pub receipts: Vec<Receipt>,
    pub requests: Option<Vec<EncodedRequests>>,
    pub block_value: U256,
    base_fee_per_blob_gas: U256,
    pub blobs_bundle: BlobsBundle,
    pub store: Store,
    pub vm: Evm,
    pub account_updates: Vec<AccountUpdate>,
}

impl PayloadBuildContext {
    pub fn new(
        payload: Block,
        storage: &Store,
        blockchain_type: BlockchainType,
    ) -> Result<Self, EvmError> {
        let config = storage
            .get_chain_config()
            .map_err(|e| EvmError::DB(e.to_string()))?;
        let base_fee_per_blob_gas = calculate_base_fee_per_blob_gas(
            payload.header.excess_blob_gas.unwrap_or_default(),
            config
                .get_fork_blob_schedule(payload.header.timestamp)
                .map(|schedule| schedule.base_fee_update_fraction)
                .unwrap_or_default(),
        );

        let vm_db = StoreVmDatabase::new(storage.clone(), payload.header.parent_hash);
        let vm = match blockchain_type {
            BlockchainType::L1 => Evm::new_for_l1(vm_db),
            BlockchainType::L2 => Evm::new_for_l2(vm_db)?,
        };

        Ok(PayloadBuildContext {
            remaining_gas: payload.header.gas_limit,
            receipts: vec![],
            requests: config
                .is_prague_activated(payload.header.timestamp)
                .then_some(Vec::new()),
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

impl PayloadBuildContext {
    fn parent_hash(&self) -> BlockHash {
        self.payload.header.parent_hash
    }

    pub fn block_number(&self) -> BlockNumber {
        self.payload.header.number
    }

    fn chain_config(&self) -> Result<ChainConfig, EvmError> {
        self.store
            .get_chain_config()
            .map_err(|e| EvmError::DB(e.to_string()))
    }

    fn base_fee_per_gas(&self) -> Option<u64> {
        self.payload.header.base_fee_per_gas
    }
}

#[derive(Debug, Clone)]
pub struct PayloadBuildResult {
    pub blobs_bundle: BlobsBundle,
    pub block_value: U256,
    pub receipts: Vec<Receipt>,
    pub requests: Vec<EncodedRequests>,
    pub account_updates: Vec<AccountUpdate>,
    pub payload: Block,
}

impl From<PayloadBuildContext> for PayloadBuildResult {
    fn from(value: PayloadBuildContext) -> Self {
        let PayloadBuildContext {
            blobs_bundle,
            block_value,
            requests,
            receipts,
            account_updates,
            payload,
            ..
        } = value;

        Self {
            blobs_bundle,
            block_value,
            requests: requests.unwrap_or_default(),
            receipts,
            account_updates,
            payload,
        }
    }
}

impl Blockchain {
    /// Attempts to fetch a payload given it's id. If the payload is still being built, it will be finished.
    /// Fails if there is no payload or active payload build task for the given id.
    pub async fn get_payload(&self, payload_id: u64) -> Result<PayloadBuildResult, ChainError> {
        let mut payloads = self.payloads.lock().await;
        // Find the given payload and finish the active build process if needed
        let idx = payloads
            .iter()
            .position(|(id, _)| id == &payload_id)
            .ok_or(ChainError::UnknownPayload)?;
        let finished_payload = (payload_id, payloads.remove(idx).1.to_payload().await?);
        payloads.insert(idx, finished_payload);
        // Return the held payload
        match &payloads[idx].1 {
            PayloadOrTask::Payload(payload) => Ok(*payload.clone()),
            _ => unreachable!("we already converted the payload into a finished version"),
        }
    }

    /// Starts a payload build process. The built payload can be retrieved by calling `get_payload`.
    /// The build process will run for the full block building timeslot or until `get_payload` is called
    pub async fn initiate_payload_build(self: Arc<Blockchain>, payload: Block, payload_id: u64) {
        let self_clone = self.clone();
        let cancel_token = CancellationToken::new();
        let cancel_token_clone = cancel_token.clone();
        let payload_build_task = tokio::task::spawn(async move {
            self_clone
                .build_payload_loop(payload, cancel_token_clone)
                .await
        });
        let mut payloads = self.payloads.lock().await;
        if payloads.len() >= MAX_PAYLOADS {
            // Remove oldest unclaimed payload
            payloads.remove(0);
        }
        payloads.push((
            payload_id,
            PayloadOrTask::Task(PayloadBuildTask {
                task: payload_build_task,
                cancel: cancel_token,
            }),
        ));
    }

    /// Build the given payload and keep on rebuilding it until either the time slot
    /// given by `SECONDS_PER_SLOT` is up or the `cancel_token` is cancelled
    pub async fn build_payload_loop(
        self: Arc<Blockchain>,
        payload: Block,
        cancel_token: CancellationToken,
    ) -> Result<PayloadBuildResult, ChainError> {
        let start = Instant::now();
        let self_clone = self.clone();
        const SECONDS_PER_SLOT: Duration = Duration::from_secs(12);
        // Attempt to rebuild the payload as many times within the given timeframe to maximize fee revenue
        let mut res = self_clone.build_payload(payload.clone()).await?;
        while start.elapsed() < SECONDS_PER_SLOT && !cancel_token.is_cancelled() {
            let payload = payload.clone();
            // Cancel the current build process and return the previous payload if it is requested earlier
            if let Some(current_res) = cancel_token
                .run_until_cancelled(self_clone.build_payload(payload))
                .await
            {
                res = current_res?;
            }
        }
        Ok(res)
    }

    /// Completes the payload building process, return the block value
    pub async fn build_payload(&self, payload: Block) -> Result<PayloadBuildResult, ChainError> {
        let since = Instant::now();
        let gas_limit = payload.header.gas_limit;

        debug!("Building payload");
        let base_fee = payload.header.base_fee_per_gas.unwrap_or_default();
        let mut context = PayloadBuildContext::new(payload, &self.storage, self.r#type.clone())?;

        if let BlockchainType::L1 = self.r#type {
            self.apply_system_operations(&mut context)?;
        }
        self.apply_withdrawals(&mut context)?;
        self.fill_transactions(&mut context)?;
        self.extract_requests(&mut context)?;
        self.finalize_payload(&mut context).await?;

        let interval = Instant::now().duration_since(since).as_millis();
        tracing::info!(
            "[METRIC] BUILDING PAYLOAD TOOK: {interval} ms, base fee {}",
            base_fee
        );
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

    pub fn apply_withdrawals(&self, context: &mut PayloadBuildContext) -> Result<(), EvmError> {
        let binding = Vec::new();
        let withdrawals = context
            .payload
            .body
            .withdrawals
            .as_ref()
            .unwrap_or(&binding);
        context.vm.process_withdrawals(withdrawals)
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
    pub fn fetch_mempool_transactions(
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
                debug!("Skipping transaction: {}, no gas left", head_tx.tx.hash());
                // We don't have enough gas left for the transaction, so we skip all txs from this account
                txs.pop();
                continue;
            }

            // TODO: maybe fetch hash too when filtering mempool so we don't have to compute it here (we can do this in the same refactor as adding timestamp)
            let tx_hash = head_tx.tx.hash();

            // Check whether the tx is replay-protected
            if head_tx.tx.protected() && !chain_config.is_eip155_activated(context.block_number()) {
                // Ignore replay protected tx & all txs from the sender
                // Pull transaction from the mempool
                debug!("Ignoring replay-protected transaction: {}", tx_hash);
                txs.pop();
                self.remove_transaction_from_pool(&tx_hash)?;
                continue;
            }

            // Execute tx
            let receipt = match self.apply_transaction(&head_tx, context) {
                Ok(receipt) => {
                    txs.shift()?;
                    metrics!(METRICS_TX.inc_tx_with_type(MetricsTxType(head_tx.tx_type())));
                    receipt
                }
                // Ignore following txs from sender
                Err(e) => {
                    error!("Failed to execute transaction: {tx_hash:x}, {e}");
                    metrics!(METRICS_TX.inc_tx_errors(e.to_metric()));
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
            _ => apply_plain_transaction(head, context),
        }
    }

    /// Runs a blob transaction, updates the gas count & blob data and returns the receipt
    fn apply_blob_transaction(
        &self,
        head: &HeadTransaction,
        context: &mut PayloadBuildContext,
    ) -> Result<Receipt, ChainError> {
        // Fetch blobs bundle
        let tx_hash = head.tx.hash();
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
        let receipt = apply_plain_transaction(head, context)?;
        // Update context with blob data
        let prev_blob_gas = context.payload.header.blob_gas_used.unwrap_or_default();
        context.payload.header.blob_gas_used =
            Some(prev_blob_gas + (blobs_bundle.blobs.len() * GAS_PER_BLOB as usize) as u64);
        context.blobs_bundle += blobs_bundle;
        Ok(receipt)
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

        context.requests = Some(requests.iter().map(|r| r.encode()).collect());

        Ok(())
    }

    pub async fn finalize_payload(
        &self,
        context: &mut PayloadBuildContext,
    ) -> Result<(), ChainError> {
        let account_updates = context.vm.get_state_transitions()?;

        let ret_acount_updates_list = self
            .storage
            .apply_account_updates_batch(context.parent_hash(), &account_updates)
            .await?
            .ok_or(ChainError::ParentStateNotFound)?;

        let state_root = ret_acount_updates_list.state_trie_hash;

        context.payload.header.state_root = state_root;
        context.payload.header.transactions_root =
            compute_transactions_root(&context.payload.body.transactions);
        context.payload.header.receipts_root = compute_receipts_root(&context.receipts);
        context.payload.header.requests_hash = context
            .requests
            .as_ref()
            .map(|requests| compute_requests_hash(requests));
        context.payload.header.gas_used = context.payload.header.gas_limit - context.remaining_gas;
        context.account_updates = account_updates;

        let mut logs = vec![];
        for receipt in context.receipts.iter().cloned() {
            for log in receipt.logs {
                logs.push(log);
            }
        }

        context.payload.header.logs_bloom = bloom_from_logs(&logs);
        Ok(())
    }
}

/// Runs a plain (non blob) transaction, updates the gas count and returns the receipt
pub fn apply_plain_transaction(
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

/// A struct representing suitable mempool transactions waiting to be included in a block
// TODO: Consider using VecDequeue instead of Vec
pub struct TransactionQueue {
    // The first transaction for each account along with its tip, sorted by highest tip
    heads: Vec<HeadTransaction>,
    // The remaining txs grouped by account and sorted by nonce
    txs: HashMap<Address, Vec<MempoolTransaction>>,
    // Base Fee stored for tip calculations
    base_fee: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadTransaction {
    pub tx: MempoolTransaction,
    pub tip: u64,
}

impl std::ops::Deref for HeadTransaction {
    type Target = Transaction;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

impl From<HeadTransaction> for Transaction {
    fn from(val: HeadTransaction) -> Self {
        val.tx.transaction().clone()
    }
}

impl TransactionQueue {
    /// Creates a new TransactionQueue from a set of transactions grouped by sender and sorted by nonce
    fn new(
        mut txs: HashMap<Address, Vec<MempoolTransaction>>,
        base_fee: Option<u64>,
    ) -> Result<Self, ChainError> {
        let mut heads = Vec::with_capacity(100);
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
    pub fn clear(&mut self) {
        self.heads.clear();
        self.txs.clear();
    }

    /// Returns true if there are no more transactions in the queue
    pub fn is_empty(&self) -> bool {
        self.heads.is_empty()
    }

    /// Returns the head transaction with the highest tip
    /// If there is more than one transaction with the highest tip, return the one with the lowest timestamp
    pub fn peek(&self) -> Option<HeadTransaction> {
        self.heads.first().cloned()
    }

    /// Removes current head transaction and all transactions from the given sender
    pub fn pop(&mut self) {
        if !self.is_empty() {
            let sender = self.heads.remove(0).tx.sender();
            self.txs.remove(&sender);
        }
    }

    /// Remove the top transaction
    /// Add a tx from the same sender to the head transactions
    pub fn shift(&mut self) -> Result<(), ChainError> {
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
        match (self.tx_type(), other.tx_type()) {
            (TxType::Privileged, TxType::Privileged) => return self.nonce().cmp(&other.nonce()),
            (TxType::Privileged, _) => return Ordering::Less,
            (_, TxType::Privileged) => return Ordering::Greater,
            _ => (),
        };
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
