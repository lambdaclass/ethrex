use std::{
    cmp::{min, Ordering},
    collections::HashMap,
};

use ethereum_rust_core::{
    types::{
        calculate_base_fee_per_blob_gas, calculate_base_fee_per_gas, compute_receipts_root,
        compute_transactions_root, compute_withdrawals_root, BlobsBundle, Block, BlockBody,
        BlockHash, BlockHeader, BlockNumber, ChainConfig, MempoolTransaction, Receipt, Transaction,
        Withdrawal, DEFAULT_OMMERS_HASH,
    },
    Address, Bloom, Bytes, H256, U256,
};
use ethereum_rust_rlp::encode::RLPEncode;
use ethereum_rust_storage::{error::StoreError, Store};
use ethereum_rust_vm::{
    beacon_root_contract_call, evm_state, execute_tx, get_state_transitions, process_withdrawals,
    spec_id, EvmError, EvmState, SpecId,
};
use sha3::{Digest, Keccak256};

use crate::{
    constants::{
        GAS_LIMIT_BOUND_DIVISOR, GAS_PER_BLOB, MAX_BLOB_GAS_PER_BLOCK, MIN_GAS_LIMIT,
        TARGET_BLOB_GAS_PER_BLOCK, TX_GAS_COST,
    },
    error::ChainError,
    mempool::{self, PendingTxFilter},
};

use tracing::debug;

pub struct BuildPayloadArgs {
    pub parent: BlockHash,
    pub timestamp: u64,
    pub fee_recipient: Address,
    pub random: H256,
    pub withdrawals: Vec<Withdrawal>,
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
        hasher.update(self.withdrawals.encode_to_vec());
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
    // TODO: check where we should get builder values from
    const DEFAULT_BUILDER_GAS_CEIL: u64 = 30_000_000;
    let parent_block = storage
        .get_block_header_by_hash(args.parent)?
        .ok_or_else(|| ChainError::ParentNotFound)?;
    let chain_config = storage.get_chain_config()?;
    let gas_limit = calc_gas_limit(parent_block.gas_limit, DEFAULT_BUILDER_GAS_CEIL);
    let payload = Block {
        header: BlockHeader {
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
                .then_some(compute_withdrawals_root(&args.withdrawals)),
            blob_gas_used: Some(0),
            excess_blob_gas: chain_config.is_cancun_activated(args.timestamp).then_some(
                calc_excess_blob_gas(
                    parent_block.excess_blob_gas.unwrap_or_default(),
                    parent_block.blob_gas_used.unwrap_or_default(),
                ),
            ),
            parent_beacon_block_root: args.beacon_root,
        },
        // Empty body as we just created this payload
        body: BlockBody {
            transactions: Vec::new(),
            ommers: Vec::new(),
            withdrawals: Some(args.withdrawals.clone()),
        },
    };
    // Delay applying withdrawals until the payload is requested and built
    Ok(payload)
}

fn calc_gas_limit(parent_gas_limit: u64, desired_limit: u64) -> u64 {
    let delta = parent_gas_limit / GAS_LIMIT_BOUND_DIVISOR - 1;
    let mut limit = parent_gas_limit;
    let desired_limit = min(desired_limit, MIN_GAS_LIMIT);
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

fn calc_excess_blob_gas(parent_excess_blob_gas: u64, parent_blob_gas_used: u64) -> u64 {
    let excess_blob_gas = parent_excess_blob_gas + parent_blob_gas_used;
    if excess_blob_gas < TARGET_BLOB_GAS_PER_BLOCK {
        0
    } else {
        excess_blob_gas - TARGET_BLOB_GAS_PER_BLOCK
    }
}

pub struct PayloadBuildContext<'a> {
    pub payload: &'a mut Block,
    pub evm_state: &'a mut EvmState,
    pub remaining_gas: u64,
    pub receipts: Vec<Receipt>,
    pub block_value: U256,
    base_fee_per_blob_gas: U256,
    pub blobs_bundle: BlobsBundle,
}

impl<'a> PayloadBuildContext<'a> {
    fn new(payload: &'a mut Block, evm_state: &'a mut EvmState) -> Self {
        PayloadBuildContext {
            remaining_gas: payload.header.gas_limit,
            receipts: vec![],
            block_value: U256::zero(),
            base_fee_per_blob_gas: U256::from(calculate_base_fee_per_blob_gas(
                payload.header.excess_blob_gas.unwrap_or_default(),
            )),
            payload,
            evm_state,
            blobs_bundle: BlobsBundle::default(),
        }
    }
}

impl<'a> PayloadBuildContext<'a> {
    fn parent_hash(&self) -> BlockHash {
        self.payload.header.parent_hash
    }

    fn block_number(&self) -> BlockNumber {
        self.payload.header.number
    }

    fn store(&self) -> Option<&Store> {
        self.evm_state.database()
    }

    fn chain_config(&self) -> Result<ChainConfig, EvmError> {
        self.evm_state.chain_config()
    }

    fn base_fee_per_gas(&self) -> Option<u64> {
        self.payload.header.base_fee_per_gas
    }
}

/// Completes the payload building process, return the block value
pub fn build_payload(
    payload: &mut Block,
    store: &Store,
) -> Result<(BlobsBundle, U256), ChainError> {
    debug!("Building payload");
    let mut evm_state = evm_state(store.clone(), payload.header.parent_hash);
    let mut context = PayloadBuildContext::new(payload, &mut evm_state);
    apply_withdrawals(&mut context)?;
    fill_transactions(&mut context)?;
    finalize_payload(&mut context)?;
    Ok((context.blobs_bundle, context.block_value))
}

pub fn apply_withdrawals(context: &mut PayloadBuildContext) -> Result<(), EvmError> {
    // Apply withdrawals & call beacon root contract, and obtain the new state root
    let spec_id = spec_id(&context.chain_config()?, context.payload.header.timestamp);
    if context.payload.header.parent_beacon_block_root.is_some() && spec_id == SpecId::CANCUN {
        beacon_root_contract_call(context.evm_state, &context.payload.header, spec_id)?;
    }
    let withdrawals = context.payload.body.withdrawals.clone().unwrap_or_default();
    process_withdrawals(context.evm_state, &withdrawals)?;
    Ok(())
}

/// Fetches suitable transactions from the mempool
/// Returns two transaction queues, one for plain and one for blob txs
fn fetch_mempool_transactions(
    context: &mut PayloadBuildContext,
) -> Result<(TransactionQueue, TransactionQueue), StoreError> {
    let tx_filter = PendingTxFilter {
        /*TODO(https://github.com/lambdaclass/ethereum_rust/issues/680): add tip filter */
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
    let store = context.store().ok_or(StoreError::Custom(
        "no store in the context (is an ExecutionDB being used?)".to_string(),
    ))?;
    Ok((
        // Plain txs
        TransactionQueue::new(
            mempool::filter_transactions(&plain_tx_filter, store)?,
            context.base_fee_per_gas(),
        ),
        // Blob txs
        TransactionQueue::new(
            mempool::filter_transactions(&blob_tx_filter, store)?,
            context.base_fee_per_gas(),
        ),
    ))
}

/// Fills the payload with transactions taken from the mempool
/// Returns the block value
pub fn fill_transactions(context: &mut PayloadBuildContext) -> Result<(), ChainError> {
    let chain_config = context.chain_config()?;
    debug!("Fetching transactions from mempool");
    // Fetch mempool transactions
    let (mut plain_txs, mut blob_txs) = fetch_mempool_transactions(context)?;
    // Execute and add transactions to payload (if suitable)
    loop {
        // Check if we have enough gas to run more transactions
        if context.remaining_gas < TX_GAS_COST {
            debug!("No more gas to run transactions");
            break;
        };
        if !blob_txs.is_empty()
            && context.blobs_bundle.blobs.len() as u64 * GAS_PER_BLOB >= MAX_BLOB_GAS_PER_BLOCK
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
            mempool::remove_transaction(
                tx_hash,
                context
                    .store()
                    .ok_or(ChainError::StoreError(StoreError::MissingStore))?,
            )?;
            continue;
        }
        // Execute tx
        let receipt = match apply_transaction(&head_tx, context) {
            Ok(receipt) => {
                txs.shift();
                // Pull transaction from the mempool
                mempool::remove_transaction(
                    tx_hash,
                    context
                        .store()
                        .ok_or(ChainError::StoreError(StoreError::MissingStore))?,
                )?;
                receipt
            }
            // Ignore following txs from sender
            Err(e) => {
                debug!("Failed to execute transaction: {}, {e}", tx_hash);
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
    head: &HeadTransaction,
    context: &mut PayloadBuildContext,
) -> Result<Receipt, ChainError> {
    match **head {
        Transaction::EIP4844Transaction(_) => apply_blob_transaction(head, context),
        _ => apply_plain_transaction(head, context),
    }
}

/// Runs a blob transaction, updates the gas count & blob data and returns the receipt
fn apply_blob_transaction(
    head: &HeadTransaction,
    context: &mut PayloadBuildContext,
) -> Result<Receipt, ChainError> {
    // Fetch blobs bundle
    let tx_hash = head.tx.compute_hash();
    let Some(blobs_bundle) = context
        .store()
        .ok_or(ChainError::StoreError(StoreError::MissingStore))?
        .get_blobs_bundle_from_pool(tx_hash)?
    else {
        // No blob tx should enter the mempool without its blobs bundle so this is an internal error
        return Err(
            StoreError::Custom(format!("No blobs bundle found for blob tx {tx_hash}")).into(),
        );
    };
    if (context.blobs_bundle.blobs.len() + blobs_bundle.blobs.len()) as u64 * GAS_PER_BLOB
        > MAX_BLOB_GAS_PER_BLOCK
    {
        // This error will only be used for debug tracing
        return Err(EvmError::Custom("max data blobs reached".to_string()).into());
    };
    // Apply transaction
    let receipt = apply_plain_transaction(head, context)?;
    // Update context with blob data
    let prev_blob_gas = context.payload.header.blob_gas_used.unwrap_or_default();
    context.payload.header.blob_gas_used =
        Some(prev_blob_gas + blobs_bundle.blobs.len() as u64 * GAS_PER_BLOB);
    context.blobs_bundle += blobs_bundle;
    Ok(receipt)
}

/// Runs a plain (non blob) transaction, updates the gas count and returns the receipt
fn apply_plain_transaction(
    head: &HeadTransaction,
    context: &mut PayloadBuildContext,
) -> Result<Receipt, ChainError> {
    let result = execute_tx(
        &head.tx,
        &context.payload.header,
        context.evm_state,
        spec_id(
            &context.chain_config().map_err(ChainError::from)?,
            context.payload.header.timestamp,
        ),
    )?;
    context.remaining_gas = context.remaining_gas.saturating_sub(result.gas_used());
    context.block_value += U256::from(result.gas_used()) * head.tip;
    let receipt = Receipt::new(
        head.tx.tx_type(),
        result.is_success(),
        context.payload.header.gas_limit - context.remaining_gas,
        result.logs(),
    );
    Ok(receipt)
}

fn finalize_payload(context: &mut PayloadBuildContext) -> Result<(), StoreError> {
    let account_updates = get_state_transitions(context.evm_state);
    context.payload.header.state_root = context
        .store()
        .ok_or(StoreError::MissingStore)?
        .apply_account_updates(context.parent_hash(), &account_updates)?
        .unwrap_or_default();
    context.payload.header.transactions_root =
        compute_transactions_root(&context.payload.body.transactions);
    context.payload.header.receipts_root = compute_receipts_root(&context.receipts);
    context.payload.header.gas_used = context.payload.header.gas_limit - context.remaining_gas;
    Ok(())
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
    sender: Address,
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
    fn new(mut txs: HashMap<Address, Vec<MempoolTransaction>>, base_fee: Option<u64>) -> Self {
        let mut heads = Vec::new();
        for (address, txs) in txs.iter_mut() {
            // Pull the first tx from each list and add it to the heads list
            // This should be a newly filtered tx list so we are guaranteed to have a first element
            let head_tx = txs.remove(0);
            heads.push(HeadTransaction {
                // We already ran this method when filtering the transactions from the mempool so it shouldn't fail
                tip: head_tx.effective_gas_tip(base_fee).unwrap(),
                tx: head_tx,
                sender: *address,
            });
        }
        // Sort heads by higest tip (and lowest timestamp if tip is equal)
        heads.sort();
        TransactionQueue {
            heads,
            txs,
            base_fee,
        }
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
            let sender = self.heads.remove(0).sender;
            self.txs.remove(&sender);
        }
    }

    /// Remove the top transaction
    /// Add a tx from the same sender to the head transactions
    fn shift(&mut self) {
        let tx = self.heads.remove(0);
        if let Some(txs) = self.txs.get_mut(&tx.sender) {
            // Fetch next head
            if !txs.is_empty() {
                let head_tx = txs.remove(0);
                let head = HeadTransaction {
                    // We already ran this method when filtering the transactions from the mempool so it shouldn't fail
                    tip: head_tx.effective_gas_tip(self.base_fee).unwrap(),
                    tx: head_tx,
                    sender: tx.sender,
                };
                // Insert head into heads list while maintaing order
                let index = match self.heads.binary_search(&head) {
                    Ok(index) => index, // Same ordering shouldn't be possible when adding timestamps
                    Err(index) => index,
                };
                self.heads.insert(index, head);
            }
        }
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
