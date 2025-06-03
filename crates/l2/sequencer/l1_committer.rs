use crate::{
    sequencer::{blobs_bundle_cache::BlobsBundleCache, errors::CommitterError},
    CommitterConfig, EthConfig, SequencerConfig,
};

use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    types::{
        batch::Batch, blobs_bundle, fake_exponential_checked, AccountUpdate, BlobsBundle,
        BlobsBundleError, Block, BlockHeader, BlockNumber, PrivilegedL2Transaction, Receipt,
        Transaction, TxKind, BLOB_BASE_FEE_UPDATE_FRACTION, MIN_BASE_FEE_PER_BLOB_GAS,
    },
    Address, H256, U256,
};
use ethrex_l2_common::{
    compute_deposit_logs_hash, compute_withdrawals_merkle_root, get_block_deposits,
    get_block_withdrawals, prepare_state_diff, StateDiff,
};
use ethrex_l2_sdk::calldata::{encode_calldata, Value};
use ethrex_metrics::metrics;
#[cfg(feature = "metrics")]
use ethrex_metrics::metrics_l2::{MetricsL2BlockType, METRICS_L2};
use ethrex_rpc::{
    clients::eth::{eth_sender::Overrides, BlockByNumber, EthClient, WrappedTransaction},
    utils::get_withdrawal_hash,
};
use ethrex_storage::Store;
use ethrex_storage_rollup::StoreRollup;
use ethrex_vm::{Evm, EvmEngine};
use secp256k1::SecretKey;
use std::{collections::HashMap, sync::Arc};
use tracing::{debug, error, info, warn};

use super::{errors::BlobEstimationError, execution_cache::ExecutionCache, utils::random_duration};
use spawned_concurrency::{send_after, CallResponse, CastResponse, GenServer, GenServerInMsg};
use spawned_rt::mpsc::Sender;

const COMMIT_FUNCTION_SIGNATURE: &str = "commitBatch(uint256,bytes32,bytes32,bytes32)";

#[derive(Clone)]
pub struct CommitterState {
    eth_client: EthClient,
    on_chain_proposer_address: Address,
    store: Store,
    rollup_store: StoreRollup,
    l1_address: Address,
    l1_private_key: SecretKey,
    commit_time_ms: u64,
    arbitrary_base_blob_gas_price: u64,
    execution_cache: Arc<ExecutionCache>,
    validium: bool,
    blobs_bundle_cache: Arc<BlobsBundleCache>,
}

impl CommitterState {
    pub fn new(
        committer_config: &CommitterConfig,
        eth_config: &EthConfig,
        store: Store,
        rollup_store: StoreRollup,
        execution_cache: Arc<ExecutionCache>,
        blobs_bundle_cache: Arc<BlobsBundleCache>,
    ) -> Result<Self, CommitterError> {
        Ok(Self {
            eth_client: EthClient::new_with_config(
                eth_config.rpc_url.iter().map(AsRef::as_ref).collect(),
                eth_config.max_number_of_retries,
                eth_config.backoff_factor,
                eth_config.min_retry_delay,
                eth_config.max_retry_delay,
                Some(eth_config.maximum_allowed_max_fee_per_gas),
                Some(eth_config.maximum_allowed_max_fee_per_blob_gas),
            )?,
            on_chain_proposer_address: committer_config.on_chain_proposer_address,
            store,
            rollup_store,
            l1_address: committer_config.l1_address,
            l1_private_key: committer_config.l1_private_key,
            commit_time_ms: committer_config.commit_time_ms,
            arbitrary_base_blob_gas_price: committer_config.arbitrary_base_blob_gas_price,
            execution_cache,
            validium: committer_config.validium,
            blobs_bundle_cache,
        })
    }
}

#[derive(Clone)]
pub enum InMessage {
    Commit,
}

#[allow(dead_code)]
#[derive(Clone, PartialEq)]
pub enum OutMessage {
    Done,
    Error,
}

pub struct L1Committer;

impl L1Committer {
    pub async fn spawn(
        store: Store,
        rollup_store: StoreRollup,
        execution_cache: Arc<ExecutionCache>,
        blobs_bundle_cache: Arc<BlobsBundleCache>,
        cfg: SequencerConfig,
    ) -> Result<(), CommitterError> {
        let state = CommitterState::new(
            &cfg.l1_committer,
            &cfg.eth,
            store.clone(),
            rollup_store.clone(),
            execution_cache.clone(),
            blobs_bundle_cache.clone(),
        )?;
        let mut l1_committer = L1Committer::start(state);
        let _ = l1_committer.cast(InMessage::Commit).await;
        Ok(())
    }
}

impl GenServer for L1Committer {
    type InMsg = InMessage;
    type OutMsg = OutMessage;
    type State = CommitterState;

    type Error = CommitterError;

    fn new() -> Self {
        Self {}
    }

    async fn handle_call(
        &mut self,
        _message: Self::InMsg,
        _tx: &Sender<GenServerInMsg<Self>>,
        _state: &mut Self::State,
    ) -> CallResponse<Self::OutMsg> {
        CallResponse::Reply(OutMessage::Done)
    }

    async fn handle_cast(
        &mut self,
        _message: Self::InMsg,
        tx: &Sender<GenServerInMsg<Self>>,
        state: &mut Self::State,
    ) -> CastResponse {
        // Right now we only have the Commit message, so we ignore the message
        let check_interval = random_duration(state.commit_time_ms);
        send_after(check_interval, tx.clone(), Self::InMsg::Commit);
        let _ = commit_next_batch_to_l1(state)
            .await
            .inspect_err(|err| error!("L1 Committer Error: {err}"));
        CastResponse::NoReply
    }
}

async fn commit_next_batch_to_l1(state: &mut CommitterState) -> Result<(), CommitterError> {
    // Get the batch to commit
    let last_committed_batch_number = state
        .eth_client
        .get_last_committed_batch(state.on_chain_proposer_address)
        .await?;
    let batch_to_commit = last_committed_batch_number + 1;

    let batch = match state.rollup_store.get_batch(batch_to_commit).await? {
        Some(batch) => batch,
        None => {
            let last_committed_batch = state
                .rollup_store
                .get_batch(last_committed_batch_number)
                .await?.ok_or(CommitterError::InternalError(format!("Failed to get batch with batch number {last_committed_batch_number}. Batch is missing when it should be present. This is a bug")))?;
            let first_block_to_commit = last_committed_batch.last_block + 1;
            // Try to prepare batch
            let (
                blobs_bundle,
                new_state_root,
                withdrawal_hashes,
                deposit_logs_hash,
                last_block_of_batch,
            ) = prepare_batch_from_block(state, last_committed_batch.last_block, batch_to_commit)
                .await?;

            if last_committed_batch.last_block == last_block_of_batch {
                debug!("No new blocks to commit, skipping");
                return Ok(());
            }

            let batch = Batch {
                number: batch_to_commit,
                first_block: first_block_to_commit,
                last_block: last_block_of_batch,
                state_root: new_state_root,
                deposit_logs_hash,
                withdrawal_hashes,
                blobs_bundle,
            };

            state.rollup_store.store_batch(batch.clone()).await?;

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

    match send_commitment(state, &batch).await {
        Ok(commit_tx_hash) => {
            metrics!(
            let _ = METRICS_L2
                .set_block_type_and_block_number(
                    MetricsL2BlockType::LastCommittedBlock,
                    batch.last_block,
                )
                .inspect_err(|e| {
                    tracing::error!(
                        "Failed to set metric: last committed block {}",
                        e.to_string()
                    )
                });
            );

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
    state: &mut CommitterState,
    mut last_added_block_number: BlockNumber,
    batch_number: u64,
) -> Result<(BlobsBundle, H256, Vec<H256>, H256, BlockNumber), CommitterError> {
    let first_block_of_batch = last_added_block_number + 1;
    let mut blobs_bundle = BlobsBundle::default();

    let mut acc_withdrawals = vec![];
    let mut acc_deposits = vec![];
    let mut acc_account_updates: HashMap<Address, AccountUpdate> = HashMap::new();
    let mut withdrawal_hashes = vec![];
    let mut deposit_logs_hashes = vec![];
    let mut new_state_root = H256::default();

    #[cfg(feature = "metrics")]
    let mut tx_count = 0_u64;
    let mut _blob_size = 0_usize;

    info!("Preparing state diff from block {first_block_of_batch}");

    loop {
        // Get a block to add to the batch
        let Some(block_to_commit_body) = state
            .store
            .get_block_body(last_added_block_number + 1)
            .await
            .map_err(CommitterError::from)?
        else {
            debug!("No new block to commit, skipping..");
            break;
        };
        let block_to_commit_header = state
            .store
            .get_block_header(last_added_block_number + 1)
            .map_err(CommitterError::from)?
            .ok_or(CommitterError::FailedToGetInformationFromStorage(
                "Failed to get_block_header() after get_block_body()".to_owned(),
            ))?;

        // Get block transactions and receipts
        let mut txs_and_receipts = vec![];
        let mut txs = vec![];
        for (index, tx) in block_to_commit_body.transactions.iter().enumerate() {
            let receipt = state
                .store
                .get_receipt(last_added_block_number + 1, index.try_into()?)
                .await?
                .ok_or(CommitterError::InternalError(
                    "Transactions in a block should have a receipt".to_owned(),
                ))?;
            txs_and_receipts.push((tx.clone(), receipt));
            txs.push(tx.clone());
        }

        metrics!(
            tx_count += txs_and_receipts
                .len()
                .try_into()
                .inspect_err(|_| tracing::error!("Failed to collect metric tx count"))
                .unwrap_or(0)
        );
        // Get block withdrawals and deposits
        let withdrawals = get_block_withdrawals(&txs_and_receipts)?;
        let deposits = get_block_deposits(&txs);

        // Get block account updates.
        let block_to_commit = Block::new(block_to_commit_header.clone(), block_to_commit_body);
        let account_updates =
            if let Some(account_updates) = state.execution_cache.get(block_to_commit.hash())? {
                account_updates
            } else {
                warn!(
                "Could not find execution cache result for block {}, falling back to re-execution",
                last_added_block_number + 1
            );

                let vm_db =
                    StoreVmDatabase::new(state.store.clone(), block_to_commit.header.parent_hash);
                let mut vm = Evm::new(EvmEngine::default(), vm_db);
                vm.execute_block(&block_to_commit)?;
                vm.get_state_transitions()?
            };

        // Accumulate block data with the rest of the batch.
        acc_withdrawals.extend(withdrawals.clone());
        acc_deposits.extend(deposits.clone());
        for account in account_updates {
            let address = account.address;
            if let Some(existing) = acc_account_updates.get_mut(&address) {
                existing.merge(account);
            } else {
                acc_account_updates.insert(address, account);
            }
        }

        let result = if !state.validium {
            // Prepare current state diff.
            let state_diff = prepare_state_diff(
                first_block_of_batch,
                block_to_commit_header,
                state.store.clone(),
                &acc_withdrawals,
                &acc_deposits,
                acc_account_updates.clone().into_values().collect(),
            )
            .await?;
            generate_blobs_bundle(&state_diff)
        } else {
            Ok((BlobsBundle::default(), 0_usize))
        };

        let Ok((bundle, latest_blob_size)) = result else {
            warn!("Batch size limit reached. Any remaining blocks will be processed in the next batch.");
            // Break loop. Use the previous generated blobs_bundle.
            break;
        };

        // Save current blobs_bundle and continue to add more blocks.
        blobs_bundle = bundle;
        _blob_size = latest_blob_size;
        for (_, tx) in &withdrawals {
            let hash =
                get_withdrawal_hash(tx).ok_or(CommitterError::InvalidWithdrawalTransaction)?;
            withdrawal_hashes.push(hash);
        }

        deposit_logs_hashes.extend(
            deposits
                .iter()
                .filter_map(|tx| tx.get_deposit_hash())
                .collect::<Vec<H256>>(),
        );

        new_state_root = state
            .store
            .state_trie(block_to_commit.hash())?
            .ok_or(CommitterError::FailedToGetInformationFromStorage(
                "Failed to get state root from storage".to_owned(),
            ))?
            .hash_no_commit();

        last_added_block_number += 1;
    }

    if !state.validium {
        state
            .blobs_bundle_cache
            .push(batch_number, blobs_bundle.clone())?;
    }

    metrics!(if let (Ok(deposits_count), Ok(withdrawals_count)) = (
            deposit_logs_hashes.len().try_into(),
            withdrawal_hashes.len().try_into()
        ) {
            let _ = state
                .rollup_store
                .update_operations_count(tx_count, deposits_count, withdrawals_count)
                .await
                .inspect_err(|e| {
                    tracing::error!("Failed to update operations metric: {}", e.to_string())
                });
        }
        #[allow(clippy::as_conversions)]
        let blob_usage_percentage = _blob_size as f64 * 100_f64 / ethrex_common::types::BYTES_PER_BLOB_F64;
        METRICS_L2.set_blob_usage_percentage(blob_usage_percentage);
    );

    let deposit_logs_hash = compute_deposit_logs_hash(deposit_logs_hashes)?;
    Ok((
        blobs_bundle,
        new_state_root,
        withdrawal_hashes,
        deposit_logs_hash,
        last_added_block_number,
    ))
}

/// Generate the blob bundle necessary for the EIP-4844 transaction.
fn generate_blobs_bundle(state_diff: &StateDiff) -> Result<(BlobsBundle, usize), CommitterError> {
    let blob_data = state_diff.encode().map_err(CommitterError::from)?;

    let blob_size = blob_data.len();

    let blob = blobs_bundle::blob_from_bytes(blob_data).map_err(CommitterError::from)?;

    Ok((
        BlobsBundle::create_from_blobs(&vec![blob]).map_err(CommitterError::from)?,
        blob_size,
    ))
}

async fn send_commitment(
    state: &mut CommitterState,
    batch: &Batch,
) -> Result<H256, CommitterError> {
    let calldata_values = vec![
        Value::Uint(U256::from(batch_number)),
        Value::FixedBytes(new_state_root.0.to_vec().into()),
        Value::FixedBytes(withdrawal_logs_merkle_root.0.to_vec().into()),
        Value::FixedBytes(batch.deposit_logs_hash.0.to_vec().into()),
    ];

    let calldata = encode_calldata(COMMIT_FUNCTION_SIGNATURE, &calldata_values)?;

    let gas_price = state
        .eth_client
        .get_gas_price_with_extra(20)
        .await?
        .try_into()
        .map_err(|_| {
            CommitterError::InternalError("Failed to convert gas_price to a u64".to_owned())
        })?;

    // Validium: EIP1559 Transaction.
    // Rollup: EIP4844 Transaction -> For on-chain Data Availability.
    let mut tx = if !state.validium {
        let le_bytes = estimate_blob_gas(
            &state.eth_client,
            state.arbitrary_base_blob_gas_price,
            20, // 20% of headroom
        )
        .await?
        .to_le_bytes();

        let gas_price_per_blob = U256::from_little_endian(&le_bytes);

        let wrapped_tx = state
            .eth_client
            .build_eip4844_transaction(
                state.on_chain_proposer_address,
                state.l1_address,
                calldata.into(),
                Overrides {
                    from: Some(state.l1_address),
                    gas_price_per_blob: Some(gas_price_per_blob),
                    max_fee_per_gas: Some(gas_price),
                    max_priority_fee_per_gas: Some(gas_price),
                    ..Default::default()
                },
                batch.blobs_bundle.clone(),
            )
            .await
            .map_err(CommitterError::from)?;

        WrappedTransaction::EIP4844(wrapped_tx)
    } else {
        let wrapped_tx = state
            .eth_client
            .build_eip1559_transaction(
                state.on_chain_proposer_address,
                state.l1_address,
                calldata.into(),
                Overrides {
                    from: Some(state.l1_address),
                    max_fee_per_gas: Some(gas_price),
                    max_priority_fee_per_gas: Some(gas_price),
                    ..Default::default()
                },
            )
            .await
            .map_err(CommitterError::from)?;

        WrappedTransaction::EIP1559(wrapped_tx)
    };

    state
        .eth_client
        .set_gas_for_wrapped_tx(&mut tx, state.l1_address)
        .await?;

    let commit_tx_hash = state
        .eth_client
        .send_tx_bump_gas_exponential_backoff(&mut tx, &state.l1_private_key)
        .await?;

    info!("Commitment sent: {commit_tx_hash:#x}");

    Ok(commit_tx_hash)
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
        .get_block_by_number(BlockByNumber::Latest)
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
