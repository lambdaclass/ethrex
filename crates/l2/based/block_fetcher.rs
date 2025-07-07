use std::{cmp::min, collections::HashMap, sync::Arc, time::Duration};

use ethrex_blockchain::{Blockchain, fork_choice::apply_fork_choice, vm::StoreVmDatabase};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        AccountUpdate, Block, BlockNumber, PrivilegedL2Transaction, Transaction, batch::Batch,
    },
};
use ethrex_l2_common::{
    l1_messages::{L1Message, get_block_l1_messages, get_l1_message_hash},
    privileged_transactions::compute_privileged_transactions_hash,
    state_diff::prepare_state_diff,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rpc::{EthClient, types::receipt::RpcLog};
use ethrex_storage::Store;
use ethrex_storage_rollup::{RollupStoreError, StoreRollup};
use keccak_hash::keccak;
use spawned_concurrency::{
    error::GenServerError,
    messages::Unused,
    tasks::{CastResponse, GenServer, GenServerHandle, send_after},
};
use tracing::{debug, error, info};

use crate::{
    SequencerConfig,
    based::sequencer_state::{SequencerState, SequencerStatus},
    sequencer::{l1_committer::generate_blobs_bundle, utils::node_is_up_to_date},
};

#[derive(Debug, thiserror::Error)]
pub enum BlockFetcherError {
    #[error("Block Fetcher failed due to an EthClient error: {0}")]
    EthClientError(#[from] ethrex_rpc::clients::EthClientError),
    #[error("Block Fetcher failed due to a Store error: {0}")]
    StoreError(#[from] ethrex_storage::error::StoreError),
    #[error("State Updater failed due to a RollupStore error: {0}")]
    RollupStoreError(#[from] RollupStoreError),
    #[error("Internal Error: {0}")]
    InternalError(String),
    #[error("Failed to store fetched block: {0}")]
    ChainError(#[from] ethrex_blockchain::error::ChainError),
    #[error("Failed to apply fork choice for fetched block: {0}")]
    InvalidForkChoice(#[from] ethrex_blockchain::error::InvalidForkChoice),
    #[error("Failed to push fetched block to execution cache: {0}")]
    ExecutionCacheError(#[from] crate::sequencer::errors::ExecutionCacheError),
    #[error("Failed to RLP decode fetched block: {0}")]
    RLPDecodeError(#[from] ethrex_rlp::error::RLPDecodeError),
    #[error("Block Fetcher failed in a helper function: {0}")]
    UtilsError(#[from] crate::utils::error::UtilsError),
    #[error("Missing bytes from calldata: {0}")]
    WrongBatchCalldata(String),
    #[error("Failed due to an EVM error: {0}")]
    EvmError(#[from] ethrex_vm::EvmError),
    #[error("Failed to produce the blob bundle")]
    BlobBundleError,
    #[error("Failed to compute deposit logs hash: {0}")]
    PrivilegedTransactionError(
        #[from] ethrex_l2_common::privileged_transactions::PrivilegedTransactionError,
    ),
    // TODO: Avoid propagating GenServerErrors outside GenServer modules
    // See https://github.com/lambdaclass/ethrex/issues/3376
    #[error("Spawned GenServer Error")]
    GenServerError(GenServerError),
}

#[derive(Clone)]
pub struct BlockFetcherState {
    eth_client: EthClient,
    on_chain_proposer_address: Address,
    store: Store,
    rollup_store: StoreRollup,
    blockchain: Arc<Blockchain>,
    sequencer_state: SequencerState,
    fetch_interval_ms: u64,
    last_l1_block_fetched: U256,
    fetch_block_step: U256,
}

impl BlockFetcherState {
    pub async fn new(
        cfg: &SequencerConfig,
        store: Store,
        rollup_store: StoreRollup,
        blockchain: Arc<Blockchain>,
        sequencer_state: SequencerState,
    ) -> Result<Self, BlockFetcherError> {
        let eth_client = EthClient::new_with_multiple_urls(cfg.eth.rpc_url.clone())?;
        let last_l1_block_fetched = eth_client
            .get_last_fetched_l1_block(cfg.l1_watcher.bridge_address)
            .await?
            .into();
        Ok(Self {
            eth_client,
            on_chain_proposer_address: cfg.l1_committer.on_chain_proposer_address,
            store,
            rollup_store,
            blockchain,
            sequencer_state,
            fetch_interval_ms: cfg.based.block_fetcher.fetch_interval_ms,
            last_l1_block_fetched,
            fetch_block_step: cfg.based.block_fetcher.fetch_block_step.into(),
        })
    }
}

#[derive(Clone)]
pub enum InMessage {
    Fetch,
}

#[derive(Clone, PartialEq)]
pub enum OutMessage {
    Done,
}

pub struct BlockFetcher;

impl BlockFetcher {
    pub async fn spawn(
        cfg: &SequencerConfig,
        store: Store,
        rollup_store: StoreRollup,
        blockchain: Arc<Blockchain>,
        sequencer_state: SequencerState,
    ) -> Result<(), BlockFetcherError> {
        let state =
            BlockFetcherState::new(cfg, store, rollup_store, blockchain, sequencer_state).await?;
        let mut block_fetcher = BlockFetcher::start(state);
        block_fetcher
            .cast(InMessage::Fetch)
            .await
            .map_err(BlockFetcherError::GenServerError)
    }
}

impl GenServer for BlockFetcher {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type State = BlockFetcherState;
    type Error = BlockFetcherError;

    fn new() -> Self {
        Self {}
    }

    async fn handle_cast(
        &mut self,
        _message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
        mut state: Self::State,
    ) -> CastResponse<Self> {
        if let SequencerStatus::Following = state.sequencer_state.status().await {
            let _ = fetch(&mut state).await.inspect_err(|err| {
                error!("Block Fetcher Error: {err}");
            });
        }
        send_after(
            Duration::from_millis(state.fetch_interval_ms),
            handle.clone(),
            Self::CastMsg::Fetch,
        );
        CastResponse::NoReply(state)
    }
}

async fn fetch(state: &mut BlockFetcherState) -> Result<(), BlockFetcherError> {
    while !node_is_up_to_date::<BlockFetcherError>(
        &state.eth_client,
        state.on_chain_proposer_address,
        &state.rollup_store,
    )
    .await?
    {
        info!("Node is not up to date. Syncing via L1");

        let last_l2_block_number_known = state.store.get_latest_block_number().await?;

        let last_l2_batch_number_known = state
            .rollup_store
            .get_batch_number_by_block(last_l2_block_number_known)
            .await?
            .ok_or(BlockFetcherError::InternalError(format!(
                "Failed to get last batch number known for block {last_l2_block_number_known}"
            )))?;

        let last_l2_committed_batch_number = state
            .eth_client
            .get_last_committed_batch(state.on_chain_proposer_address)
            .await?;

        let l2_batches_behind = last_l2_committed_batch_number.checked_sub(last_l2_batch_number_known).ok_or(
            BlockFetcherError::InternalError(
                "Failed to calculate batches behind. Last batch number known is greater than last committed batch number.".to_string(),
            ),
        )?;

        info!(
            "Node is {l2_batches_behind} batches behind. Last batch number known: {last_l2_batch_number_known}, last committed batch number: {last_l2_committed_batch_number}"
        );

        let (batch_committed_logs, batch_verified_logs) = get_logs(state).await?;

        process_committed_logs(batch_committed_logs, state, last_l2_batch_number_known).await?;
        process_verified_logs(batch_verified_logs, state).await?;
    }

    info!("Node is up to date");

    Ok(())
}

/// Fetch logs from the L1 chain for the BatchCommitted and BatchVerified events.
/// This function fetches logs, starting from the last fetched block number (aka the last block that was processed)
/// and going up to the current block number.
async fn get_logs(
    state: &mut BlockFetcherState,
) -> Result<(Vec<RpcLog>, Vec<RpcLog>), BlockFetcherError> {
    let last_l1_block_number = state.eth_client.get_block_number().await?;

    let mut batch_committed_logs = Vec::new();
    let mut batch_verified_logs = Vec::new();
    while state.last_l1_block_fetched < last_l1_block_number {
        let new_last_l1_fetched_block = min(
            state.last_l1_block_fetched + state.fetch_block_step,
            last_l1_block_number,
        );

        debug!(
            "Fetching logs from block {} to {}",
            state.last_l1_block_fetched + 1,
            new_last_l1_fetched_block
        );

        // Fetch logs from the L1 chain for the BatchCommitted event.
        let committed_logs = state
            .eth_client
            .get_logs(
                state.last_l1_block_fetched + 1,
                new_last_l1_fetched_block,
                state.on_chain_proposer_address,
                keccak(b"BatchCommitted(uint256,bytes32)"),
            )
            .await?;

        // Fetch logs from the L1 chain for the BatchVerified event.
        let verified_logs = state
            .eth_client
            .get_logs(
                state.last_l1_block_fetched + 1,
                new_last_l1_fetched_block,
                state.on_chain_proposer_address,
                keccak(b"BatchVerified(uint256)"),
            )
            .await?;

        // Update the last L1 block fetched.
        state.last_l1_block_fetched = new_last_l1_fetched_block;

        batch_committed_logs.extend_from_slice(&committed_logs);
        batch_verified_logs.extend_from_slice(&verified_logs);
    }

    Ok((batch_committed_logs, batch_verified_logs))
}

/// Process the logs from the event `BatchCommitted`.
/// Gets the committed batches that are missing in the local store from the logs,
/// and seals the batch in the rollup store.
async fn process_committed_logs(
    batch_committed_logs: Vec<RpcLog>,
    state: &mut BlockFetcherState,
    last_l2_batch_number_known: u64,
) -> Result<(), BlockFetcherError> {
    let mut missing_batches_logs =
        filter_logs(&batch_committed_logs, last_l2_batch_number_known).await?;

    missing_batches_logs.sort_by_key(|(_log, batch_number)| *batch_number);

    for (batch_committed_log, batch_number) in missing_batches_logs {
        let batch_commit_tx_calldata = state
            .eth_client
            .get_transaction_by_hash(batch_committed_log.transaction_hash)
            .await?
            .ok_or(BlockFetcherError::InternalError(format!(
                "Failed to get the receipt for transaction {:x}",
                batch_committed_log.transaction_hash
            )))?
            .data;

        let batch = decode_batch_from_calldata(&batch_commit_tx_calldata)?;

        store_batch(state, &batch).await?;

        seal_batch(
            state,
            &batch,
            batch_number,
            batch_committed_log.transaction_hash,
        )
        .await?;
    }
    Ok(())
}

/// Given the logs from the event `BatchCommitted`,
/// this function gets the committed batches that are missing in the local store.
/// It does that by comparing if the batch number is greater than the last known batch number.
async fn filter_logs(
    logs: &[RpcLog],
    last_batch_number_known: u64,
) -> Result<Vec<(RpcLog, U256)>, BlockFetcherError> {
    let mut filtered_logs = Vec::new();

    // Filter missing batches logs
    for batch_committed_log in logs.iter().cloned() {
        let committed_batch_number = U256::from_big_endian(
            batch_committed_log
                .log
                .topics
                .get(1)
                .ok_or(BlockFetcherError::InternalError(
                    "Failed to get committed batch number from BatchCommitted log".to_string(),
                ))?
                .as_bytes(),
        );

        if committed_batch_number > last_batch_number_known.into() {
            filtered_logs.push((batch_committed_log, committed_batch_number));
        }
    }

    Ok(filtered_logs)
}

// TODO: Move to calldata module (SDK)
fn decode_batch_from_calldata(calldata: &[u8]) -> Result<Vec<Block>, BlockFetcherError> {
    // function commitBatch(
    //     uint256 batchNumber,
    //     bytes32 newStateRoot,
    //     bytes32 stateDiffKZGVersionedHash,
    //     bytes32 messagesLogsMerkleRoot,
    //     bytes32 processedPrivilegedTransactionsRollingHash,
    //     bytes[] calldata _rlpEncodedBlocks
    // ) external;

    // data =   4 bytes (function selector) 0..4
    //          || 8 bytes (batch number)   4..36
    //          || 32 bytes (new state root) 36..68
    //          || 32 bytes (state diff KZG versioned hash) 68..100
    //          || 32 bytes (messages logs merkle root) 100..132
    //          || 32 bytes (processed privileged transactions rolling hash) 132..164

    let batch_length_in_blocks = U256::from_big_endian(calldata.get(196..228).ok_or(
        BlockFetcherError::WrongBatchCalldata("Couldn't get batch length bytes".to_owned()),
    )?)
    .as_usize();

    let base = 228;

    let mut batch = Vec::new();

    for block_i in 0..batch_length_in_blocks {
        let block_length_offset = base + block_i * 32;

        let dynamic_offset = U256::from_big_endian(
            calldata
                .get(block_length_offset..block_length_offset + 32)
                .ok_or(BlockFetcherError::WrongBatchCalldata(
                    "Couldn't get dynamic offset bytes".to_owned(),
                ))?,
        )
        .as_usize();

        let block_length_in_bytes = U256::from_big_endian(
            calldata
                .get(base + dynamic_offset..base + dynamic_offset + 32)
                .ok_or(BlockFetcherError::WrongBatchCalldata(
                    "Couldn't get block length bytes".to_owned(),
                ))?,
        )
        .as_usize();

        let block_offset = base + dynamic_offset + 32;

        let block = Block::decode(
            calldata
                .get(block_offset..block_offset + block_length_in_bytes)
                .ok_or(BlockFetcherError::WrongBatchCalldata(
                    "Couldn't get block bytes".to_owned(),
                ))?,
        )?;

        batch.push(block);
    }

    Ok(batch)
}

async fn store_batch(
    state: &mut BlockFetcherState,
    batch: &[Block],
) -> Result<(), BlockFetcherError> {
    for block in batch.iter() {
        state.blockchain.add_block(block).await?;

        let block_hash = block.hash();

        apply_fork_choice(&state.store, block_hash, block_hash, block_hash).await?;

        info!(
            "Added fetched block {} with hash {block_hash:#x}",
            block.header.number,
        );
    }

    Ok(())
}

async fn seal_batch(
    state: &mut BlockFetcherState,
    batch: &[Block],
    batch_number: U256,
    commit_tx: H256,
) -> Result<(), BlockFetcherError> {
    let batch = get_batch(state, batch, batch_number, commit_tx).await?;

    state.rollup_store.seal_batch(batch).await?;

    info!("Sealed batch {batch_number}.");

    Ok(())
}

async fn get_batch_message_hashes(
    state: &mut BlockFetcherState,
    batch: &[Block],
) -> Result<Vec<H256>, BlockFetcherError> {
    let mut message_hashes = Vec::new();

    for block in batch {
        let block_messages = extract_block_messages(state, block.header.number).await?;

        for msg in &block_messages {
            message_hashes.push(get_l1_message_hash(msg));
        }
    }

    Ok(message_hashes)
}

async fn extract_block_messages(
    state: &mut BlockFetcherState,
    block_number: BlockNumber,
) -> Result<Vec<L1Message>, BlockFetcherError> {
    let Some(block_body) = state.store.get_block_body(block_number).await? else {
        return Err(BlockFetcherError::InternalError(format!(
            "Block {block_number} is supposed to be in store at this point"
        )));
    };

    let mut txs = vec![];
    let mut receipts = vec![];
    for (index, tx) in block_body.transactions.iter().enumerate() {
        let receipt = state
            .store
            .get_receipt(
                block_number,
                index.try_into().map_err(|_| {
                    BlockFetcherError::InternalError("Failed to convert index to u64".to_owned())
                })?,
            )
            .await?
            .ok_or(BlockFetcherError::InternalError(
                "Transactions in a block should have a receipt".to_owned(),
            ))?;
        txs.push(tx.clone());
        receipts.push(receipt);
    }
    Ok(get_block_l1_messages(&receipts))
}

async fn get_batch(
    state: &mut BlockFetcherState,
    batch: &[Block],
    batch_number: U256,
    commit_tx: H256,
) -> Result<Batch, BlockFetcherError> {
    let privileged_transactions: Vec<PrivilegedL2Transaction> = batch
        .iter()
        .flat_map(|block| {
            block.body.transactions.iter().filter_map(|tx| {
                if let Transaction::PrivilegedL2Transaction(tx) = tx {
                    Some(tx.clone())
                } else {
                    None
                }
            })
        })
        .collect();
    let privileged_transaction_hashes = privileged_transactions
        .iter()
        .filter_map(|tx| tx.get_privileged_hash())
        .collect();
    let mut messages = Vec::new();
    for block in batch {
        let block_messages = extract_block_messages(state, block.header.number).await?;
        messages.extend(block_messages);
    }
    let privileged_transactions_hash =
        compute_privileged_transactions_hash(privileged_transaction_hashes)?;

    let first_block = batch.first().ok_or(BlockFetcherError::InternalError(
        "Batch is empty. This shouldn't happen.".to_owned(),
    ))?;

    let last_block = batch.last().ok_or(BlockFetcherError::InternalError(
        "Batch is empty. This shouldn't happen.".to_owned(),
    ))?;

    let new_state_root = state
        .store
        .state_trie(last_block.hash())?
        .ok_or(BlockFetcherError::InternalError(
            "This block should be in the store".to_owned(),
        ))?
        .hash_no_commit();

    // This is copied from the L1Committer, this should be reviewed.
    let mut acc_account_updates: HashMap<H160, AccountUpdate> = HashMap::new();
    for block in batch {
        let vm_db = StoreVmDatabase::new(state.store.clone(), block.header.parent_hash);
        let mut vm = state.blockchain.new_evm(vm_db)?;
        vm.execute_block(block)
            .map_err(BlockFetcherError::EvmError)?;
        let account_updates = vm
            .get_state_transitions()
            .map_err(BlockFetcherError::EvmError)?;

        for account in account_updates {
            let address = account.address;
            if let Some(existing) = acc_account_updates.get_mut(&address) {
                existing.merge(account);
            } else {
                acc_account_updates.insert(address, account);
            }
        }
    }

    let parent_block_hash = first_block.header.parent_hash;

    let parent_db = StoreVmDatabase::new(state.store.clone(), parent_block_hash);

    let state_diff = prepare_state_diff(
        last_block.header.clone(),
        &parent_db,
        &messages,
        &privileged_transactions,
        acc_account_updates.into_values().collect(),
    )
    .map_err(|_| BlockFetcherError::BlobBundleError)?;

    let (blobs_bundle, _) =
        generate_blobs_bundle(&state_diff).map_err(|_| BlockFetcherError::BlobBundleError)?;

    Ok(Batch {
        number: batch_number.as_u64(),
        first_block: first_block.header.number,
        last_block: last_block.header.number,
        state_root: new_state_root,
        privileged_transactions_hash,
        message_hashes: get_batch_message_hashes(state, batch).await?,
        blobs_bundle,
        commit_tx: Some(commit_tx),
        verify_tx: None,
    })
}

/// Process the logs from the event `BatchVerified`.
/// Gets the batch number from the logs and stores the verify transaction hash in the rollup store
async fn process_verified_logs(
    batch_verified_logs: Vec<RpcLog>,
    state: &mut BlockFetcherState,
) -> Result<(), BlockFetcherError> {
    for batch_verified_log in batch_verified_logs {
        let batch_number = U256::from_big_endian(
            batch_verified_log
                .log
                .topics
                .get(1)
                .ok_or(BlockFetcherError::InternalError(
                    "Failed to get verified batch number from BatchVerified log".to_string(),
                ))?
                .as_bytes(),
        );

        let verify_tx_hash = batch_verified_log.transaction_hash;

        state
            .rollup_store
            .store_verify_tx_by_batch(batch_number.as_u64(), verify_tx_hash)
            .await?;

        info!("Stored verify transaction hash {verify_tx_hash:#x} for batch {batch_number}");
    }
    Ok(())
}
