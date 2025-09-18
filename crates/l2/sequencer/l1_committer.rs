use crate::{
    CommitterConfig, EthConfig, SequencerConfig,
    based::sequencer_state::{SequencerState, SequencerStatus},
    sequencer::{
        errors::CommitterError,
        utils::{self, system_now_ms},
    },
};

use bytes::Bytes;
use ethrex_blockchain::{Blockchain, vm::StoreVmDatabase};
use ethrex_common::{
    Address, H256, U256,
    types::{
        AccountUpdate, BLOB_BASE_FEE_UPDATE_FRACTION, BlobsBundle, Block, BlockNumber,
        MIN_BASE_FEE_PER_BLOB_GAS, TxType, batch::Batch, blobs_bundle, fake_exponential_checked,
    },
};
use ethrex_l2_common::{
    calldata::Value,
    l1_messages::{get_block_l1_messages, get_l1_message_hash},
    merkle_tree::compute_merkle_root,
    privileged_transactions::{
        PRIVILEGED_TX_BUDGET, compute_privileged_transactions_hash,
        get_block_privileged_transactions,
    },
    state_diff::{StateDiff, prepare_state_diff},
};
use ethrex_l2_rpc::signer::{Signer, SignerHealth};
use ethrex_l2_sdk::{
    build_generic_tx, calldata::encode_calldata, get_last_committed_batch,
    send_tx_bump_gas_exponential_backoff,
};
#[cfg(feature = "metrics")]
use ethrex_metrics::l2::metrics::{METRICS, MetricsBlockType};
use ethrex_metrics::metrics;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::{
    clients::eth::{EthClient, Overrides},
    types::block_identifier::{BlockIdentifier, BlockTag},
};
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use super::{errors::BlobEstimationError, utils::random_duration};
use spawned_concurrency::tasks::{
    CallResponse, CastResponse, GenServer, GenServerHandle, send_after,
};

const COMMIT_FUNCTION_SIGNATURE_BASED: &str =
    "commitBatch(uint256,bytes32,bytes32,bytes32,bytes32,bytes[])";
const COMMIT_FUNCTION_SIGNATURE: &str = "commitBatch(uint256,bytes32,bytes32,bytes32,bytes32)";
/// Default wake up time for the committer to check if it should send a commit tx
const COMMITTER_DEFAULT_WAKE_TIME_MS: u64 = 60_000;

#[derive(Clone)]
pub enum CallMessage {
    Stop,
    /// time to wait in ms before sending commit
    Start(u64),
    Health,
}

#[derive(Clone)]
pub enum InMessage {
    Commit,
}

#[allow(dead_code)]
#[derive(Clone)]
pub enum OutMessage {
    Done,
    Error(String),
    Stopped,
    Started,
    Health(Box<L1CommitterHealth>),
}

pub struct L1Committer {
    eth_client: EthClient,
    blockchain: Arc<Blockchain>,
    on_chain_proposer_address: Address,
    store: Store,
    rollup_store: StoreRollup,
    commit_time_ms: u64,
    batch_gas_limit: Option<u64>,
    arbitrary_base_blob_gas_price: u64,
    validium: bool,
    signer: Signer,
    based: bool,
    sequencer_state: SequencerState,
    /// Time to wait before checking if it should send a new batch
    committer_wake_up_ms: u64,
    /// Timestamp of last successful committed batch
    last_committed_batch_timestamp: u128,
    /// Last succesful committed batch number
    last_committed_batch: u64,
    /// Cancellation token for the next inbound InMessage::Commit
    cancellation_token: Option<CancellationToken>,
}

#[derive(Clone, Serialize)]
pub struct L1CommitterHealth {
    rpc_healthcheck: BTreeMap<String, serde_json::Value>,
    commit_time_ms: u64,
    arbitrary_base_blob_gas_price: u64,
    validium: bool,
    based: bool,
    sequencer_state: String,
    committer_wake_up_ms: u64,
    last_committed_batch_timestamp: u128,
    last_committed_batch: u64,
    signer_status: SignerHealth,
    running: bool,
    on_chain_proposer_address: Address,
}

impl L1Committer {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        committer_config: &CommitterConfig,
        eth_config: &EthConfig,
        blockchain: Arc<Blockchain>,
        store: Store,
        rollup_store: StoreRollup,
        based: bool,
        sequencer_state: SequencerState,
    ) -> Result<Self, CommitterError> {
        let eth_client = EthClient::new_with_config(
            eth_config.rpc_url.iter().map(AsRef::as_ref).collect(),
            eth_config.max_number_of_retries,
            eth_config.backoff_factor,
            eth_config.min_retry_delay,
            eth_config.max_retry_delay,
            Some(eth_config.maximum_allowed_max_fee_per_gas),
            Some(eth_config.maximum_allowed_max_fee_per_blob_gas),
        )?;
        let last_committed_batch =
            get_last_committed_batch(&eth_client, committer_config.on_chain_proposer_address)
                .await?;
        Ok(Self {
            eth_client,
            blockchain,
            on_chain_proposer_address: committer_config.on_chain_proposer_address,
            store,
            rollup_store,
            commit_time_ms: committer_config.commit_time_ms,
            batch_gas_limit: committer_config.batch_gas_limit,
            arbitrary_base_blob_gas_price: committer_config.arbitrary_base_blob_gas_price,
            validium: committer_config.validium,
            signer: committer_config.signer.clone(),
            based,
            sequencer_state,
            committer_wake_up_ms: committer_config
                .commit_time_ms
                .min(COMMITTER_DEFAULT_WAKE_TIME_MS),
            last_committed_batch_timestamp: 0,
            last_committed_batch,
            cancellation_token: None,
        })
    }

    pub async fn spawn(
        store: Store,
        blockchain: Arc<Blockchain>,
        rollup_store: StoreRollup,
        cfg: SequencerConfig,
        sequencer_state: SequencerState,
    ) -> Result<GenServerHandle<L1Committer>, CommitterError> {
        let state = Self::new(
            &cfg.l1_committer,
            &cfg.eth,
            blockchain,
            store.clone(),
            rollup_store.clone(),
            cfg.based.enabled,
            sequencer_state,
        )
        .await?;
        // NOTE: we spawn as blocking due to `generate_blobs_bundle` and
        // `send_tx_bump_gas_exponential_backoff` blocking for more than 40ms
        let l1_committer = state.start_blocking();
        if let OutMessage::Error(reason) = l1_committer
            .clone()
            .call(CallMessage::Start(cfg.l1_committer.first_wake_up_time_ms))
            .await?
        {
            Err(CommitterError::UnexpectedError(format!(
                "Failed to send first wake up message to committer {reason}"
            )))
        } else {
            Ok(l1_committer)
        }
    }

    async fn commit_next_batch_to_l1(&mut self) -> Result<(), CommitterError> {
        info!("Running committer main loop");
        // Get the batch to commit
        let last_committed_batch_number =
            get_last_committed_batch(&self.eth_client, self.on_chain_proposer_address).await?;
        let batch_to_commit = last_committed_batch_number + 1;

        let batch = match self.rollup_store.get_batch(batch_to_commit).await? {
            Some(batch) => batch,
            None => {
                let last_committed_blocks = self
                    .rollup_store
                    .get_block_numbers_by_batch(last_committed_batch_number)
                    .await?
                    .ok_or(
                        CommitterError::RetrievalError(format!("Failed to get batch with batch number {last_committed_batch_number}. Batch is missing when it should be present. This is a bug"))
                    )?;
                let last_block = last_committed_blocks
                    .last()
                    .ok_or(
                        CommitterError::RetrievalError(format!("Last committed batch ({last_committed_batch_number}) doesn't have any blocks. This is probably a bug."))
                    )?;
                let first_block_to_commit = last_block + 1;

                // Try to prepare batch
                let (
                    blobs_bundle,
                    new_state_root,
                    message_hashes,
                    privileged_transactions_hash,
                    last_block_of_batch,
                ) = self
                    .prepare_batch_from_block(*last_block, batch_to_commit)
                    .await?;

                if *last_block == last_block_of_batch {
                    debug!("No new blocks to commit, skipping");
                    return Ok(());
                }

                let batch = Batch {
                    number: batch_to_commit,
                    first_block: first_block_to_commit,
                    last_block: last_block_of_batch,
                    state_root: new_state_root,
                    privileged_transactions_hash,
                    message_hashes,
                    blobs_bundle,
                    commit_tx: None,
                    verify_tx: None,
                };

                self.rollup_store.seal_batch(batch.clone()).await?;

                debug!(
                    first_block = batch.first_block,
                    last_block = batch.last_block,
                    "Batch {} stored in database",
                    batch.number
                );

                batch
            }
        };

        info!(
            first_block = batch.first_block,
            last_block = batch.last_block,
            "Sending commitment for batch {}",
            batch.number,
        );

        match self.send_commitment(&batch).await {
            Ok(commit_tx_hash) => {
                metrics!(
                let _ = METRICS
                    .set_block_type_and_block_number(
                        MetricsBlockType::LastCommittedBlock,
                        batch.last_block,
                    )
                    .inspect_err(|e| {
                        tracing::error!(
                            "Failed to set metric: last committed block {}",
                            e.to_string()
                        )
                    });
                );

                self.rollup_store
                    .store_commit_tx_by_batch(batch.number, commit_tx_hash)
                    .await?;

                info!(
                    "Commitment sent for batch {}, with tx hash {commit_tx_hash:#x}.",
                    batch.number
                );
                Ok(())
            }
            Err(error) => Err(CommitterError::FailedToSendCommitment(format!(
                "Failed to send commitment for batch {}. first_block: {} last_block: {}: {error}",
                batch.number, batch.first_block, batch.last_block
            ))),
        }
    }

    async fn prepare_batch_from_block(
        &mut self,
        mut last_added_block_number: BlockNumber,
        batch_number: u64,
    ) -> Result<(BlobsBundle, H256, Vec<H256>, H256, BlockNumber), CommitterError> {
        let first_block_of_batch = last_added_block_number + 1;
        let mut blobs_bundle = BlobsBundle::default();

        let mut acc_messages = vec![];
        let mut acc_privileged_txs = vec![];
        let mut acc_account_updates: HashMap<Address, AccountUpdate> = HashMap::new();
        let mut message_hashes = vec![];
        let mut privileged_transactions_hashes = vec![];
        let mut new_state_root = H256::default();
        let mut acc_gas_used = 0_u64;

        #[cfg(feature = "metrics")]
        let mut tx_count = 0_u64;
        #[cfg(feature = "metrics")]
        let mut blob_size = 0_usize;
        #[cfg(feature = "metrics")]
        let mut batch_gas_used = 0_u64;

        info!("Preparing state diff from block {first_block_of_batch}");

        loop {
            let block_to_commit_number = last_added_block_number + 1;
            // Get a block to add to the batch
            let Some(block_to_commit_body) = self
                .store
                .get_block_body(block_to_commit_number)
                .await
                .map_err(CommitterError::from)?
            else {
                debug!("No new block to commit, skipping..");
                break;
            };
            let block_to_commit_header = self
                .store
                .get_block_header(block_to_commit_number)
                .map_err(CommitterError::from)?
                .ok_or(CommitterError::FailedToGetInformationFromStorage(
                    "Failed to get_block_header() after get_block_body()".to_owned(),
                ))?;

            let current_block_gas_used = block_to_commit_header.gas_used;

            // Check if adding this block would exceed the batch gas limit
            if let Some(batch_gas_limit) = self.batch_gas_limit {
                if acc_gas_used + current_block_gas_used > batch_gas_limit {
                    debug!(
                        "Batch gas limit reached. Any remaining blocks will be processed in the next batch"
                    );
                    break;
                }
            }

            // Get block transactions and receipts
            let mut txs = vec![];
            let mut receipts = vec![];
            for (index, tx) in block_to_commit_body.transactions.iter().enumerate() {
                let receipt = self
                    .store
                    .get_receipt(block_to_commit_number, index.try_into()?)
                    .await?
                    .ok_or(CommitterError::RetrievalError(
                        "Transactions in a block should have a receipt".to_owned(),
                    ))?;
                txs.push(tx.clone());
                receipts.push(receipt);
            }

            metrics!(
                tx_count += txs
                    .len()
                    .try_into()
                    .inspect_err(|_| tracing::error!("Failed to collect metric tx count"))
                    .unwrap_or(0);
                batch_gas_used += block_to_commit_header.gas_used;
            );
            // Get block messages and privileged transactions
            let messages = get_block_l1_messages(&receipts);
            let privileged_transactions = get_block_privileged_transactions(&txs);

            // Get block account updates.
            let block_to_commit = Block::new(block_to_commit_header.clone(), block_to_commit_body);
            let account_updates = if let Some(account_updates) = self
                .rollup_store
                .get_account_updates_by_block_number(block_to_commit_number)
                .await?
            {
                account_updates
            } else {
                warn!(
                    "Could not find execution cache result for block {}, falling back to re-execution",
                    last_added_block_number + 1
                );

                let vm_db =
                    StoreVmDatabase::new(self.store.clone(), block_to_commit.header.parent_hash);
                let mut vm = self.blockchain.new_evm(vm_db)?;
                vm.execute_block(&block_to_commit)?;
                vm.get_state_transitions()?
            };

            // Accumulate block data with the rest of the batch.
            acc_messages.extend(messages.clone());
            acc_privileged_txs.extend(privileged_transactions.clone());
            for account in account_updates {
                let address = account.address;
                if let Some(existing) = acc_account_updates.get_mut(&address) {
                    existing.merge(account);
                } else {
                    acc_account_updates.insert(address, account);
                }
            }

            let parent_block_hash = self
                .store
                .get_block_header(first_block_of_batch)?
                .ok_or(CommitterError::FailedToGetInformationFromStorage(
                    "Failed to get_block_header() of the last added block".to_owned(),
                ))?
                .parent_hash;
            let parent_db = StoreVmDatabase::new(self.store.clone(), parent_block_hash);

            let acc_privileged_txs_len: u64 = acc_privileged_txs.len().try_into()?;
            if acc_privileged_txs_len > PRIVILEGED_TX_BUDGET {
                warn!(
                    "Privileged transactions budget exceeded. Any remaining blocks will be processed in the next batch."
                );
                // Break loop. Use the previous generated blobs_bundle.
                break;
            }

            let result = if !self.validium {
                // Prepare current state diff.
                let state_diff = prepare_state_diff(
                    block_to_commit_header,
                    &parent_db,
                    &acc_messages,
                    &acc_privileged_txs,
                    acc_account_updates.clone().into_values().collect(),
                )?;
                generate_blobs_bundle(&state_diff)
            } else {
                Ok((BlobsBundle::default(), 0_usize))
            };

            let Ok((bundle, latest_blob_size)) = result else {
                if block_to_commit_number == first_block_of_batch {
                    return Err(CommitterError::Unreachable(
                        "Not enough blob space for a single block batch. This means a block was incorrectly produced.".to_string(),
                    ));
                }
                warn!(
                    "Batch size limit reached. Any remaining blocks will be processed in the next batch."
                );
                // Break loop. Use the previous generated blobs_bundle.
                break;
            };

            // Save current blobs_bundle and continue to add more blocks.
            blobs_bundle = bundle;

            metrics!(
                blob_size = latest_blob_size;
            );

            privileged_transactions_hashes.extend(
                privileged_transactions
                    .iter()
                    .filter_map(|tx| tx.get_privileged_hash())
                    .collect::<Vec<H256>>(),
            );

            message_hashes.extend(acc_messages.iter().map(|msg| get_l1_message_hash(msg)));

            new_state_root = self
                .store
                .state_trie(block_to_commit.hash())?
                .ok_or(CommitterError::FailedToGetInformationFromStorage(
                    "Failed to get state root from storage".to_owned(),
                ))?
                .hash_no_commit();

            last_added_block_number += 1;
            acc_gas_used += current_block_gas_used;
        } // end loop

        metrics!(if let (Ok(privileged_transaction_count), Ok(messages_count)) = (
                privileged_transactions_hashes.len().try_into(),
                message_hashes.len().try_into()
            ) {
                let _ = self
                    .rollup_store
                    .update_operations_count(tx_count, privileged_transaction_count, messages_count)
                    .await
                    .inspect_err(|e| {
                        tracing::error!("Failed to update operations metric: {}", e.to_string())
                    });
            }
            #[allow(clippy::as_conversions)]
            let blob_usage_percentage = blob_size as f64 * 100_f64 / ethrex_common::types::BYTES_PER_BLOB_F64;
            let batch_gas_used = batch_gas_used.try_into()?;
            let batch_size = (last_added_block_number - first_block_of_batch).try_into()?;
            let tx_count = tx_count.try_into()?;
            METRICS.set_blob_usage_percentage(blob_usage_percentage);
            METRICS.set_batch_gas_used(batch_number, batch_gas_used)?;
            METRICS.set_batch_size(batch_number, batch_size)?;
            METRICS.set_batch_tx_count(batch_number, tx_count)?;
        );

        info!(
            "Added {} privileged transactions to the batch",
            privileged_transactions_hashes.len()
        );

        let privileged_transactions_hash =
            compute_privileged_transactions_hash(privileged_transactions_hashes)?;

        Ok((
            blobs_bundle,
            new_state_root,
            message_hashes,
            privileged_transactions_hash,
            last_added_block_number,
        ))
    }

    async fn send_commitment(&mut self, batch: &Batch) -> Result<H256, CommitterError> {
        let messages_merkle_root = compute_merkle_root(&batch.message_hashes);
        let last_block_hash = get_last_block_hash(&self.store, batch.last_block)?;

        let mut calldata_values = vec![
            Value::Uint(U256::from(batch.number)),
            Value::FixedBytes(batch.state_root.0.to_vec().into()),
            Value::FixedBytes(messages_merkle_root.0.to_vec().into()),
            Value::FixedBytes(batch.privileged_transactions_hash.0.to_vec().into()),
            Value::FixedBytes(last_block_hash.0.to_vec().into()),
        ];

        let (commit_function_signature, values) = if self.based {
            let mut encoded_blocks: Vec<Bytes> = Vec::new();

            for i in batch.first_block..=batch.last_block {
                let block_header = self
                    .store
                    .get_block_header(i)
                    .map_err(CommitterError::from)?
                    .ok_or(CommitterError::FailedToRetrieveDataFromStorage)?;

                let block_body = self
                    .store
                    .get_block_body(i)
                    .await
                    .map_err(CommitterError::from)?
                    .ok_or(CommitterError::FailedToRetrieveDataFromStorage)?;

                let block = Block::new(block_header, block_body);

                encoded_blocks.push(block.encode_to_vec().into());
            }

            calldata_values.push(Value::Array(
                encoded_blocks.into_iter().map(Value::Bytes).collect(),
            ));

            (COMMIT_FUNCTION_SIGNATURE_BASED, calldata_values)
        } else {
            (COMMIT_FUNCTION_SIGNATURE, calldata_values)
        };

        let calldata = encode_calldata(commit_function_signature, &values)?;

        let gas_price = self
            .eth_client
            .get_gas_price_with_extra(20)
            .await?
            .try_into()
            .map_err(|_| {
                CommitterError::ConversionError("Failed to convert gas_price to a u64".to_owned())
            })?;

        // Validium: EIP1559 Transaction.
        // Rollup: EIP4844 Transaction -> For on-chain Data Availability.
        let tx = if !self.validium {
            info!("L2 is in rollup mode, sending EIP-4844 (including blob) tx to commit block");
            let le_bytes = estimate_blob_gas(
                &self.eth_client,
                self.arbitrary_base_blob_gas_price,
                20, // 20% of headroom
            )
            .await?
            .to_le_bytes();

            let gas_price_per_blob = U256::from_little_endian(&le_bytes);

            build_generic_tx(
                &self.eth_client,
                TxType::EIP4844,
                self.on_chain_proposer_address,
                self.signer.address(),
                calldata.into(),
                Overrides {
                    from: Some(self.signer.address()),
                    gas_price_per_blob: Some(gas_price_per_blob),
                    max_fee_per_gas: Some(gas_price),
                    max_priority_fee_per_gas: Some(gas_price),
                    blobs_bundle: Some(batch.blobs_bundle.clone()),
                    ..Default::default()
                },
            )
            .await
            .map_err(CommitterError::from)?
        } else {
            info!("L2 is in validium mode, sending EIP-1559 (no blob) tx to commit block");
            build_generic_tx(
                &self.eth_client,
                TxType::EIP1559,
                self.on_chain_proposer_address,
                self.signer.address(),
                calldata.into(),
                Overrides {
                    from: Some(self.signer.address()),
                    max_fee_per_gas: Some(gas_price),
                    max_priority_fee_per_gas: Some(gas_price),
                    ..Default::default()
                },
            )
            .await
            .map_err(CommitterError::from)?
        };

        let commit_tx_hash =
            send_tx_bump_gas_exponential_backoff(&self.eth_client, tx, &self.signer).await?;

        metrics!(
            let commit_tx_receipt = self
                .eth_client
                .get_transaction_receipt(commit_tx_hash)
                .await?
                .ok_or(CommitterError::UnexpectedError("no commit tx receipt".to_string()))?;
            let commit_gas_used = commit_tx_receipt.tx_info.gas_used.try_into()?;
            METRICS.set_batch_commitment_gas(batch.number, commit_gas_used)?;
            if !self.validium {
                let blob_gas_used = commit_tx_receipt.tx_info.blob_gas_used
                    .ok_or(CommitterError::UnexpectedError("no blob in rollup mode".to_string()))?
                    .try_into()?;
                METRICS.set_batch_commitment_blob_gas(batch.number, blob_gas_used)?;
            }
        );

        info!("Commitment sent: {commit_tx_hash:#x}");

        Ok(commit_tx_hash)
    }

    fn stop_committer(&mut self) -> CallResponse<Self> {
        if let Some(token) = self.cancellation_token.take() {
            token.cancel();
            info!("L1 committer stopped");
            CallResponse::Reply(OutMessage::Stopped)
        } else {
            warn!("L1 committer received stop command but it is already stopped");
            CallResponse::Reply(OutMessage::Error("Already stopped".to_string()))
        }
    }

    fn start_committer(&mut self, handle: GenServerHandle<Self>, delay: u64) -> CallResponse<Self> {
        if self.cancellation_token.is_none() {
            self.schedule_commit(delay, handle);
            info!("L1 committer restarted next commit will be sent in {delay}ms");
            CallResponse::Reply(OutMessage::Started)
        } else {
            warn!("L1 committer received start command but it is already running");
            CallResponse::Reply(OutMessage::Error("Already started".to_string()))
        }
    }

    fn schedule_commit(&mut self, delay: u64, handle: GenServerHandle<Self>) {
        let check_interval = random_duration(delay);
        let handle = send_after(check_interval, handle, InMessage::Commit);
        self.cancellation_token = Some(handle.cancellation_token);
    }

    async fn health(&mut self) -> CallResponse<Self> {
        let rpc_urls = self.eth_client.test_urls().await;
        let signer_status = self.signer.health().await;

        CallResponse::Reply(OutMessage::Health(Box::new(L1CommitterHealth {
            rpc_healthcheck: rpc_urls,
            commit_time_ms: self.commit_time_ms,
            arbitrary_base_blob_gas_price: self.arbitrary_base_blob_gas_price,
            validium: self.validium,
            based: self.based,
            sequencer_state: format!("{:?}", self.sequencer_state.status().await),
            committer_wake_up_ms: self.committer_wake_up_ms,
            last_committed_batch_timestamp: self.last_committed_batch_timestamp,
            last_committed_batch: self.last_committed_batch,
            signer_status,
            running: self.cancellation_token.is_some(),
            on_chain_proposer_address: self.on_chain_proposer_address,
        })))
    }
}

impl GenServer for L1Committer {
    type CallMsg = CallMessage;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;

    type Error = CommitterError;

    // Right now we only have the `Commit` message, so we ignore the `message` parameter
    async fn handle_cast(
        &mut self,
        _message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        if let SequencerStatus::Sequencing = self.sequencer_state.status().await {
            let current_last_committed_batch =
                get_last_committed_batch(&self.eth_client, self.on_chain_proposer_address)
                    .await
                    .unwrap_or(self.last_committed_batch);
            let Some(current_time) = utils::system_now_ms() else {
                self.schedule_commit(self.committer_wake_up_ms, handle.clone());
                return CastResponse::NoReply;
            };

            // In the event that the current batch in L1 is greater than the one we have recorded we shouldn't send a new batch
            if current_last_committed_batch > self.last_committed_batch {
                self.last_committed_batch = current_last_committed_batch;
                self.last_committed_batch_timestamp = current_time;
                self.schedule_commit(self.committer_wake_up_ms, handle.clone());
                return CastResponse::NoReply;
            }

            let commit_time: u128 = self.commit_time_ms.into();
            let should_send_commitment =
                current_time - self.last_committed_batch_timestamp > commit_time;
            #[allow(clippy::collapsible_if)]
            if should_send_commitment {
                if self
                    .commit_next_batch_to_l1()
                    .await
                    .inspect_err(|e| error!("L1 Committer Error: {e}"))
                    .is_ok()
                {
                    self.last_committed_batch_timestamp = system_now_ms().unwrap_or(current_time);
                    self.last_committed_batch = current_last_committed_batch + 1;
                }
            }
        }
        self.schedule_commit(self.commit_time_ms, handle.clone());
        CastResponse::NoReply
    }

    async fn handle_call(
        &mut self,
        message: Self::CallMsg,
        handle: &GenServerHandle<Self>,
    ) -> CallResponse<Self> {
        match message {
            CallMessage::Stop => self.stop_committer(),
            CallMessage::Start(delay) => self.start_committer(handle.clone(), delay),
            CallMessage::Health => self.health().await,
        }
    }
}

/// Generate the blob bundle necessary for the EIP-4844 transaction.
pub fn generate_blobs_bundle(
    state_diff: &StateDiff,
) -> Result<(BlobsBundle, usize), CommitterError> {
    let blob_data = state_diff.encode().map_err(CommitterError::from)?;

    let blob_size = blob_data.len();

    let blob = blobs_bundle::blob_from_bytes(blob_data).map_err(CommitterError::from)?;

    Ok((
        BlobsBundle::create_from_blobs(&vec![blob]).map_err(CommitterError::from)?,
        blob_size,
    ))
}

fn get_last_block_hash(
    store: &Store,
    last_block_number: BlockNumber,
) -> Result<H256, CommitterError> {
    store
        .get_block_header(last_block_number)?
        .map(|header| header.hash())
        .ok_or(CommitterError::RetrievalError(
            "Failed to get last block hash from storage".to_owned(),
        ))
}

/// Estimates the gas price for blob transactions based on the current state of the blockchain.
///
/// # Parameters:
/// - `eth_client`: The Ethereum client used to fetch the latest block.
/// - `arbitrary_base_blob_gas_price`: The base gas price that serves as the minimum price for blob transactions.
/// - `headroom`: Percentage applied to the estimated gas price to provide a buffer against fluctuations.
///
/// # Formula:
/// The gas price is estimated using an exponential function based on the blob gas used in the latest block and the
/// excess blob gas from the block header, following the formula from EIP-4844:
/// ```txt
///    blob_gas = arbitrary_base_blob_gas_price + (excess_blob_gas + blob_gas_used) * headroom
/// ```
async fn estimate_blob_gas(
    eth_client: &EthClient,
    arbitrary_base_blob_gas_price: u64,
    headroom: u64,
) -> Result<u64, CommitterError> {
    let latest_block = eth_client
        .get_block_by_number(BlockIdentifier::Tag(BlockTag::Latest), false)
        .await?;

    let blob_gas_used = latest_block.header.blob_gas_used.unwrap_or(0);
    let excess_blob_gas = latest_block.header.excess_blob_gas.unwrap_or(0);

    // Using the formula from the EIP-4844
    // https://eips.ethereum.org/EIPS/eip-4844
    // def get_base_fee_per_blob_gas(header: Header) -> int:
    // return fake_exponential(
    //     MIN_BASE_FEE_PER_BLOB_GAS,
    //     header.excess_blob_gas,
    //     BLOB_BASE_FEE_UPDATE_FRACTION
    // )
    //
    // factor * e ** (numerator / denominator)
    // def fake_exponential(factor: int, numerator: int, denominator: int) -> int:

    // Check if adding the blob gas used and excess blob gas would overflow
    let total_blob_gas = excess_blob_gas
        .checked_add(blob_gas_used)
        .ok_or(BlobEstimationError::OverflowError)?;

    // If the blob's market is in high demand, the equation may give a really big number.
    // This function doesn't panic, it performs checked/saturating operations.
    let blob_gas = fake_exponential_checked(
        MIN_BASE_FEE_PER_BLOB_GAS,
        total_blob_gas,
        BLOB_BASE_FEE_UPDATE_FRACTION,
    )
    .map_err(BlobEstimationError::FakeExponentialError)?;

    let gas_with_headroom = (blob_gas * (100 + headroom)) / 100;

    // Check if we have an overflow when we take the headroom into account.
    let blob_gas = arbitrary_base_blob_gas_price
        .checked_add(gas_with_headroom)
        .ok_or(BlobEstimationError::OverflowError)?;

    Ok(blob_gas)
}
