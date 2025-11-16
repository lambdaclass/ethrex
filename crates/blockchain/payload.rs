use std::{
    cmp::{Ordering, max},
    collections::HashMap,
    ops::Div,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
    u64,
};

use ethrex_common::{
    Address, Bloom, Bytes, H160, H256, Secret, Signature, U256,
    constants::{DEFAULT_OMMERS_HASH, DEFAULT_REQUESTS_HASH, GAS_PER_BLOB, MAX_RLP_BLOCK_SIZE},
    types::{
        AccountUpdate, BlobsBundle, Block, BlockBody, BlockHash, BlockHeader, BlockNumber,
        ChainConfig, EIP1559Transaction, GenericTransaction, MempoolTransaction,
        PrivilegedL2Transaction, Receipt, Transaction, TxKind, TxType, Withdrawal, bloom_from_logs,
        calc_excess_blob_gas, calculate_base_fee_per_blob_gas, calculate_base_fee_per_gas,
        compute_receipts_root, compute_transactions_root, compute_withdrawals_root,
        requests::{EncodedRequests, compute_requests_hash},
    },
    utils::keccak,
};

use ethrex_vm::{Evm, EvmError, ExecutionResult};

use ethrex_rlp::encode::{PayloadRLPEncode, RLPEncode};
use ethrex_storage::{Store, error::StoreError};

use secp256k1::{Message, SECP256K1, SecretKey};
use sha3::{Digest, Keccak256};

use ethrex_metrics::metrics;

#[cfg(feature = "metrics")]
use ethrex_metrics::metrics_blocks::METRICS_BLOCKS;
#[cfg(feature = "metrics")]
use ethrex_metrics::metrics_transactions::{METRICS_TX, MetricsTxType};
use tokio_util::sync::CancellationToken;

use crate::{
    Blockchain, BlockchainType, MAX_PAYLOADS,
    constants::{GAS_LIMIT_BOUND_DIVISOR, MIN_GAS_LIMIT, POST_OSAKA_GAS_LIMIT_CAP, TX_GAS_COST},
    error::{ChainError, InvalidBlockError},
    mempool::PendingTxFilter,
    new_evm,
    vm::StoreVmDatabase,
};

use thiserror::Error;
use tracing::{debug, error, warn};

// 0x7c2626d2e35561138288bbc1a7307fa04d8ba6b7
const ON_CHAIN_PROPOSER_ADDRESS: Address = H160([
    0x7c, 0x26, 0x26, 0xd2, 0xe3, 0x55, 0x61, 0x13, 0x82, 0x88, 0xbb, 0xc1, 0xa7, 0x30, 0x7f, 0xa0,
    0x4d, 0x8b, 0xa6, 0xb7,
]);
// 0x67cad0d689b799f385d2ebcf3a626254a9074e12
const COMMON_BRIDGE_ADDRESS: Address = H160([
    0x67, 0xca, 0xd0, 0xd6, 0x89, 0xb7, 0x99, 0xf3, 0x85, 0xd2, 0xeb, 0xcf, 0x3a, 0x62, 0x62, 0x54,
    0xa9, 0x07, 0x4e, 0x12,
]);

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
pub fn create_payload(
    args: &BuildPayloadArgs,
    storage: &Store,
    extra_data: Bytes,
) -> Result<Block, ChainError> {
    let parent_block = storage
        .get_block_header_by_hash(args.parent)?
        .ok_or_else(|| ChainError::ParentNotFound)?;
    let chain_config = storage.get_chain_config();
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
        extra_data,
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
    pub payload_size: u64,
}

impl PayloadBuildContext {
    pub fn new(
        payload: Block,
        storage: &Store,
        blockchain_type: &BlockchainType,
    ) -> Result<Self, EvmError> {
        let config = storage.get_chain_config();
        let base_fee_per_blob_gas = calculate_base_fee_per_blob_gas(
            payload.header.excess_blob_gas.unwrap_or_default(),
            config
                .get_fork_blob_schedule(payload.header.timestamp)
                .map(|schedule| schedule.base_fee_update_fraction)
                .unwrap_or_default(),
        );

        let parent_header = storage
            .get_block_header_by_hash(payload.header.parent_hash)
            .map_err(|e| EvmError::DB(e.to_string()))?
            .ok_or_else(|| EvmError::DB("parent header not found".to_string()))?;
        let vm_db = StoreVmDatabase::new(storage.clone(), parent_header);
        let vm = new_evm(blockchain_type, vm_db)?;

        let payload_size = payload.encode_to_vec().len() as u64;
        Ok(PayloadBuildContext {
            remaining_gas: payload.header.gas_limit,
            receipts: vec![],
            requests: config
                .is_prague_activated(payload.header.timestamp)
                .then_some(Vec::new()),
            block_value: U256::zero(),
            base_fee_per_blob_gas,
            payload,
            blobs_bundle: BlobsBundle::default(),
            store: storage.clone(),
            vm,
            account_updates: Vec::new(),
            payload_size,
        })
    }

    pub fn gas_used(&self) -> u64 {
        self.payload.header.gas_limit - self.remaining_gas
    }
}

impl PayloadBuildContext {
    fn parent_hash(&self) -> BlockHash {
        self.payload.header.parent_hash
    }

    pub fn block_number(&self) -> BlockNumber {
        self.payload.header.number
    }

    fn chain_config(&self) -> ChainConfig {
        self.store.get_chain_config()
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
    pub async fn initiate_payload_build(
        self: Arc<Blockchain>,
        payload: Block,
        payload_id: u64,
        l2_blockchain: Option<Arc<Blockchain>>,
    ) {
        let self_clone = self.clone();
        let cancel_token = CancellationToken::new();
        let cancel_token_clone = cancel_token.clone();
        let payload_build_task = tokio::task::spawn(async move {
            self_clone
                .build_payload_loop(payload, cancel_token_clone, l2_blockchain)
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
        l2_blockchain: Option<Arc<Blockchain>>,
    ) -> Result<PayloadBuildResult, ChainError> {
        let start = Instant::now();
        const SECONDS_PER_SLOT: Duration = Duration::from_secs(12);
        // Attempt to rebuild the payload as many times within the given timeframe to maximize fee revenue
        // TODO(#4997): start with an empty block
        let res = self
            .build_payload(payload.clone(), l2_blockchain.clone())
            .await?;
        let mut res2 = None;
        while start.elapsed() < SECONDS_PER_SLOT && !cancel_token.is_cancelled() {
            let payload = payload.clone();
            let self_clone = self.clone();
            let l2_blockchain_clone = l2_blockchain.clone();
            let building_task = tokio::task::spawn_blocking(async move || {
                self_clone.build_payload(payload, l2_blockchain_clone).await
            });
            // Cancel the current build process and return the previous payload if it is requested earlier
            // TODO(#5011): this doesn't stop the building task, but only keeps it running in the background,
            //   which wastes CPU resources.
            match cancel_token.run_until_cancelled(building_task).await {
                Some(Ok(current_res)) => {
                    res2 = Some(current_res.await?);
                }
                Some(Err(err)) => {
                    warn!(%err, "Payload-building task panicked");
                }
                None => {}
            }
        }
        Ok(res2.unwrap_or(res))
    }

    /// Completes the payload building process, return the block value
    pub async fn build_payload(
        &self,
        payload: Block,
        l2_blockchain: Option<Arc<Blockchain>>,
    ) -> Result<PayloadBuildResult, ChainError> {
        let since = Instant::now();
        let gas_limit = payload.header.gas_limit;

        debug!("Building payload");
        let base_fee = payload.header.base_fee_per_gas.unwrap_or_default();
        let mut context = PayloadBuildContext::new(payload, &self.storage, &self.options.r#type)?;

        if let BlockchainType::L1 = self.options.r#type {
            self.apply_system_operations(&mut context)?;
        }
        self.apply_withdrawals(&mut context)?;
        self.fill_transactions(&mut context, l2_blockchain).await?;
        self.extract_requests(&mut context)?;
        self.finalize_payload(&mut context)?;

        let interval = Instant::now().duration_since(since).as_millis();

        tracing::debug!(
            "[METRIC] BUILDING PAYLOAD TOOK: {interval} ms, base fee {}",
            base_fee
        );
        metrics!(METRICS_BLOCKS.set_block_building_ms(interval as i64));
        metrics!(METRICS_BLOCKS.set_block_building_base_fee(base_fee as i64));
        if let Some(gas_used) = gas_limit.checked_sub(context.remaining_gas) {
            let as_gigas = (gas_used as f64).div(10_f64.powf(9_f64));

            if interval != 0 {
                let throughput = (as_gigas) / (interval as f64) * 1000_f64;
                metrics!(METRICS_BLOCKS.set_latest_gigagas_block_building(throughput));

                tracing::debug!(
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
    pub async fn fill_transactions(
        &self,
        context: &mut PayloadBuildContext,
        l2_blockchain: Option<Arc<Blockchain>>,
    ) -> Result<(), ChainError> {
        let chain_config = context.chain_config();
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

            // Check adding a transaction wouldn't exceed the Osaka block size limit of 10 MiB
            // if inclusion of the transaction puts the block size over the size limit
            // we don't add any more txs to the payload.
            let potential_rlp_block_size =
                context.payload_size + head_tx.encode_canonical_to_vec().len() as u64;
            if context
                .chain_config()
                .is_osaka_activated(context.payload.header.timestamp)
                && potential_rlp_block_size > MAX_RLP_BLOCK_SIZE
            {
                break;
            }
            context.payload_size = potential_rlp_block_size;

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

            {
                let block_header = self
                    .storage
                    .get_block_header_by_hash(context.parent_hash())
                    .inspect_err(|e| error!("{e}"))?
                    .unwrap();
                let vm_db = StoreVmDatabase::new(self.storage.clone(), block_header.clone());
                let mut vm = self.new_evm(vm_db).inspect_err(|e| error!("{e}"))?;

                let sim = vm
                    .simulate_tx_from_generic(&head_tx.tx.clone().into(), &block_header)
                    .inspect_err(|e| error!("{e}"))?;
                for log in sim.logs() {
                    if log.address == COMMON_BRIDGE_ADDRESS
                        && log.topics.contains(
                            &H256::from_str(
                                "b0e76942d2929d9dcf5c6b8e32bf27df13e118fcaab4cef2e90257551bba0270",
                            )
                            .unwrap(),
                        )
                    {
                        let from = Address::from_slice(log.data.get(0x20 - 20..0x20).unwrap());
                        let to = Address::from_slice(log.data.get(0x40 - 20..0x40).unwrap());
                        let value = U256::from_big_endian(log.data.get(0x40..0x60).unwrap());
                        let data_len =
                            U256::from_big_endian(log.data.get(0x80..0xa0).unwrap()).as_usize();
                        let data = &log.data.iter().as_slice()[0xa0..0xa0 + data_len];

                        let l2 = l2_blockchain.as_ref().unwrap();

                        let transaction = GenericTransaction {
                            r#type: TxType::EIP1559,
                            to: TxKind::Call(to),
                            from,
                            value,
                            input: Bytes::copy_from_slice(data),
                            ..Default::default()
                        };

                        let result = simulate_tx(
                            &transaction,
                            &block_header,
                            l2.storage.clone(),
                            l2.clone(),
                        )
                        .await
                        .inspect_err(|e| error!("SIMULATE ERROR: {e}"))?;

                        // 0x57272f8e
                        // keccak(to || data)
                        // 0x40
                        // response_length
                        // response || padding
                        let response_len = result.output().len();
                        let padding = response_len % 32;

                        let data = [
                            &[0x57, 0x27, 0x2f, 0x8e],
                            keccak([to.as_bytes(), data].concat()).as_bytes(),
                            H256::from_str(
                                "0x0000000000000000000000000000000000000000000000000000000000000040",
                            )
                            .unwrap()
                            .as_bytes(),
                            U256::from_big_endian(&response_len.to_be_bytes())
                                .to_big_endian()
                                .as_slice(),
                            result.output().iter().as_slice(),
                            &vec![0; padding],
                        ]
                        .concat();

                        let pk = SecretKey::from_str(
                            "5a10921bc5815991dd35f29b4a11177c10a1f3f0493f9b6baee20cb7a8187f4e",
                        )
                        .unwrap();
                        let address =
                            Address::from_str("0001a2c749FE0Ab1C09f1131BA17530f9D764fBC").unwrap();

                        let mut tx = EIP1559Transaction {
                            chain_id: self.storage.chain_config.chain_id,
                            nonce: self
                                .storage
                                .get_nonce_by_account_address(
                                    self.storage.get_latest_block_number().await.unwrap(),
                                    address,
                                )
                                .await
                                .unwrap()
                                .unwrap(),
                            max_priority_fee_per_gas: 1000000000000,
                            max_fee_per_gas: 1000000000000,
                            gas_limit: POST_OSAKA_GAS_LIMIT_CAP - 1,
                            to: TxKind::Call(COMMON_BRIDGE_ADDRESS),
                            value: U256::zero(),
                            data: data.into(),
                            access_list: vec![],
                            signature_y_parity: false,
                            signature_r: U256::zero(),
                            signature_s: U256::zero(),
                            inner_hash: Default::default(),
                        };
                        let mut payload = vec![TxType::EIP1559 as u8];
                        payload.append(tx.encode_payload_to_vec().as_mut());

                        let hash = keccak(payload);
                        let msg = Message::from_digest(hash.0);
                        let (recovery_id, signature) = SECP256K1
                            .sign_ecdsa_recoverable(&msg, &pk)
                            .serialize_compact();

                        let signature = Signature::from_slice(
                            &[
                                signature.as_slice(),
                                &[Into::<i32>::into(recovery_id) as u8],
                            ]
                            .concat(),
                        );
                        (tx.signature_r, tx.signature_s, tx.signature_y_parity) = (
                            U256::from_big_endian(&signature[..32]),
                            U256::from_big_endian(&signature[32..64]),
                            signature[64] != 0 && signature[64] != 27,
                        );

                        let mempool_tx =
                            MempoolTransaction::new(Transaction::EIP1559Transaction(tx), address);
                        let head_tx = HeadTransaction {
                            tx: mempool_tx,
                            tip: 0,
                        };

                        let receipt = match self.apply_transaction(&head_tx, context) {
                            Ok(receipt) => {
                                println!(
                                    "[L1 Builder] L2 response preset successfully ({:#x})",
                                    head_tx.tx.hash()
                                );
                                receipt
                            }
                            Err(e) => {
                                error!("ERROR: {e}");
                                panic!(
                                    "[L1 Builder] Failed to preset L2 response ({:#x}): {e}",
                                    head_tx.tx.hash()
                                );
                            }
                        };

                        // Add transaction to block
                        debug!("Adding transaction: {} to payload", head_tx.tx.hash());
                        context.payload.body.transactions.push(head_tx.into());
                        // Save receipt for hash calculation
                        context.receipts.push(receipt);
                    }
                }
            }
            // Execute tx
            let receipt = match self.apply_transaction(&head_tx, context) {
                Ok(receipt) => {
                    if let Some(log) = receipt.logs.iter().find(|log| log.address
                        == COMMON_BRIDGE_ADDRESS
                        && log.topics.first().is_some_and(|topic| *topic == H256::from_str("7d76dd36798b00b9c38def780dc4741f49a0f441afba4260388a8f5634eac186").unwrap())) {

                        let from = Address::from_slice(log.data.get(0x20-20..0x20).unwrap());
                        let to = Address::from_slice(log.data.get(0x40-20..0x40).unwrap());
                        let transaction_id = U256::from_big_endian(log.data.get(0x40..0x60).unwrap());
                        let value = U256::from_big_endian(log.data.get(0x60..0x80).unwrap());
                        let gas_limit = U256::from_big_endian(log.data.get(0x80..0xa0).unwrap());
                        let data_len = U256::from_big_endian(log.data.get(0xc0..0xe0).unwrap()).as_usize();
                        let data = log.data.get(0xe0..0xe0+data_len).unwrap();
                        let l2 = l2_blockchain.as_ref().unwrap();
                        l2.add_transaction_to_pool(Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction{
                            chain_id: l2.storage.chain_config.chain_id,
                            nonce: transaction_id.as_u64(),
                            max_priority_fee_per_gas: 1000000000000,
                            max_fee_per_gas: 1000000000000,
                            gas_limit: gas_limit.as_u64(),
                            to: ethrex_common::types::TxKind::Call(to),
                            value: value,
                            data: Bytes::copy_from_slice(data),
                            access_list: vec![],
                            from: from,
                            inner_hash: Default::default(),
                        })).await.unwrap();
                        tracing::info!("DEPOSIT");
                    }

                    txs.shift()?;
                    metrics!(METRICS_TX.inc_tx_with_type(MetricsTxType(head_tx.tx_type())));
                    receipt
                }
                // Ignore following txs from sender
                Err(e) => {
                    debug!("Failed to execute transaction: {tx_hash:x}, {e}");
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
        let chain_config = context.chain_config();
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
            .chain_config()
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

    pub fn finalize_payload(&self, context: &mut PayloadBuildContext) -> Result<(), ChainError> {
        let account_updates = context.vm.get_state_transitions()?;

        let ret_acount_updates_list = self
            .storage
            .apply_account_updates_batch(context.parent_hash(), &account_updates)?
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
            } else {
                self.txs.remove(&tx.tx.sender());
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
        match (self.to(), other.to()) {
            (TxKind::Call(to), _) if to == ON_CHAIN_PROPOSER_ADDRESS => return Ordering::Greater,
            (_, TxKind::Call(to)) if to == ON_CHAIN_PROPOSER_ADDRESS => return Ordering::Less,
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

async fn simulate_tx(
    transaction: &GenericTransaction,
    block_header: &BlockHeader,
    storage: Store,
    blockchain: Arc<Blockchain>,
) -> Result<ExecutionResult, EvmError> {
    let vm_db = StoreVmDatabase::new(storage, block_header.clone());
    let mut vm = blockchain.new_evm(vm_db)?;

    vm.simulate_tx_from_generic(transaction, block_header)
}
