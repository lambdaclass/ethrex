use std::{
    cmp::min,
    collections::{BTreeMap, HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};

use ethrex_blockchain::{Blockchain, fork_choice::apply_fork_choice, vm::StoreVmDatabase};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        AccountUpdate, Block, BlockNumber, PrivilegedL2Transaction, Transaction, batch::Batch,
    },
};
use ethrex_l2_common::{
    calldata::Value,
    l1_messages::{L1Message, get_block_l1_messages, get_l1_message_hash},
    privileged_transactions::compute_privileged_transactions_hash,
    state_diff::prepare_state_diff,
};
use ethrex_l2_sdk::calldata::encode_calldata;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rpc::clients::{Overrides, eth::errors::CalldataEncodeError};
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
    sequencer::l1_committer::generate_blobs_bundle,
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
    #[error("Failed to encode calldata: {0}")]
    CalldataDecodeError(#[from] CalldataEncodeError),
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
    latest_safe_batch: u64,
    pending_commit_logs: BTreeMap<u64, RpcLog>,
    pending_verify_logs: BTreeMap<u64, RpcLog>,
    pending_batches: VecDeque<PendingBatch>,
}

#[derive(Clone, Debug)]
struct PendingBatch {
    number: u64,
    last_block_hash: H256,
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
            latest_safe_batch: 0,
            pending_commit_logs: BTreeMap::new(),
            pending_verify_logs: BTreeMap::new(),
            pending_batches: VecDeque::new(),
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
    let last_safe_batch_number = state
        .eth_client
        .get_last_verified_batch(state.on_chain_proposer_address)
        .await?;
    if state.latest_safe_batch < last_safe_batch_number {
        info!("Node is not up to date. Syncing via L1");
        while state.latest_safe_batch < last_safe_batch_number {
            fetch_pending_batches(state).await?;

            store_safe_batches(state).await?;
        }
    } else {
        info!("Node is up to date");
    }

    Ok(())
}

/// Fetch logs from the L1 chain for the `BatchCommitted`` event.
/// This function fetches logs, starting from the last fetched block number (aka the last block that was processed)
/// and going up to the current L1 block number.
/// Given the logs from the event `BatchCommitted`,
/// this function gets the committed batches that are missing in the local store.
/// It does that by comparing if the batch number is greater than the last known batch number.
async fn fetch_logs(state: &mut BlockFetcherState) -> Result<(), BlockFetcherError> {
    let last_l1_block_number = state.eth_client.get_block_number().await?;

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
        let commit_logs = state
            .eth_client
            .get_logs(
                state.last_l1_block_fetched + 1,
                new_last_l1_fetched_block,
                state.on_chain_proposer_address,
                keccak(b"BatchCommitted(uint256,bytes32)"),
            )
            .await?;

        let verify_logs = state
            .eth_client
            .get_logs(
                state.last_l1_block_fetched + 1,
                new_last_l1_fetched_block,
                state.on_chain_proposer_address,
                keccak(b"BatchesVerified(uint256,uint256)"),
            )
            .await?;

        // Update the last L1 block fetched.
        state.last_l1_block_fetched = new_last_l1_fetched_block;

        // get the batch number for every commit log
        for log in commit_logs {
            let bytes = log
                .log
                .topics
                .get(1)
                .ok_or(BlockFetcherError::InternalError(
                    "Failed to get committed batch number from BatchCommitted log".to_string(),
                ))?
                .as_bytes();

            let commit_batch_number = u64::from_be_bytes(
                bytes
                    .get(bytes.len() - 8..)
                    .ok_or(BlockFetcherError::InternalError(
                        "Invalid byte length for u64 conversion".to_string(),
                    ))?
                    .try_into()
                    .map_err(|_| {
                        BlockFetcherError::InternalError(
                            "Invalid conversion from be bytes to u64".to_string(),
                        )
                    })?,
            );

            if commit_batch_number > state.latest_safe_batch {
                state.pending_commit_logs.insert(commit_batch_number, log);
            }
        }

        // get the batch number for every verify log
        for log in verify_logs {
            let bytes = log
                .log
                .topics
                .get(1)
                .ok_or(BlockFetcherError::InternalError(
                    "Failed to get committed batch number from BatchCommitted log".to_string(),
                ))?
                .as_bytes();

            let initial_batch_number = u64::from_be_bytes(
                bytes
                    .get(bytes.len() - 8..)
                    .ok_or(BlockFetcherError::InternalError(
                        "Invalid byte length for u64 conversion".to_string(),
                    ))?
                    .try_into()
                    .map_err(|_| {
                        BlockFetcherError::InternalError(
                            "Invalid conversion from be bytes to u64".to_string(),
                        )
                    })?,
            );

            let bytes = log
                .log
                .topics
                .get(2)
                .ok_or(BlockFetcherError::InternalError(
                    "Failed to get committed batch number from BatchCommitted log".to_string(),
                ))?
                .as_bytes();

            let final_batch_number = u64::from_be_bytes(
                bytes
                    .get(bytes.len() - 8..)
                    .ok_or(BlockFetcherError::InternalError(
                        "Invalid byte length for u64 conversion".to_string(),
                    ))?
                    .try_into()
                    .map_err(|_| {
                        BlockFetcherError::InternalError(
                            "Invalid conversion from be bytes to u64".to_string(),
                        )
                    })?,
            );

            if initial_batch_number > state.latest_safe_batch
                && initial_batch_number <= final_batch_number
            {
                for batch_number in initial_batch_number..=final_batch_number {
                    state.pending_verify_logs.insert(batch_number, log.clone());
                }
            }
        }
    }

    Ok(())
}

/// Fetch the logs from the L1 (commit & verify).
/// build a new batch with its batch number, which is stored
/// in a queue to wait for validation
pub async fn fetch_pending_batches(state: &mut BlockFetcherState) -> Result<(), BlockFetcherError> {
    fetch_logs(state).await?;

    for (batch_number, batch_committed_log) in &state.pending_commit_logs.clone() {
        if state
            .pending_batches
            .iter()
            .any(|batch| batch.number == *batch_number)
        {
            // check if the batch has already been added
            continue;
        }
        let batch_commit_tx_calldata = state
            .eth_client
            .get_transaction_by_hash(batch_committed_log.transaction_hash)
            .await?
            .ok_or(BlockFetcherError::InternalError(format!(
                "Failed to get the receipt for transaction {:x}",
                batch_committed_log.transaction_hash
            )))?
            .data;

        let batch_blocks = decode_batch_from_calldata(&batch_commit_tx_calldata)?;

        let Some(last_block) = batch_blocks.last() else {
            return Err(BlockFetcherError::InternalError(
                "Batch block shouldn't be empty.".into(),
            ));
        };
        state.pending_batches.push_back(PendingBatch {
            number: *batch_number,
            last_block_hash: last_block.header.hash(),
        });
    }
    Ok(())
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

/// Traverse the pending batches queue and build and stores the ones that are safe (verified).
pub async fn store_safe_batches(state: &mut BlockFetcherState) -> Result<(), BlockFetcherError> {
    while let Some(pending_batch) = state.pending_batches.pop_front() {
        if batch_is_safe(state, &pending_batch.last_block_hash).await?
            && state
                .pending_commit_logs
                .contains_key(&pending_batch.number)
            && state
                .pending_verify_logs
                .contains_key(&pending_batch.number)
        {
            info!("Safe batch sealed {}.", pending_batch.number);
            let commit_log = state
                .pending_commit_logs
                .remove(&pending_batch.number)
                .ok_or(BlockFetcherError::InternalError(
                    "Commit log should be in the list.".into(),
                ))?;

            let verify_log = state
                .pending_verify_logs
                .remove(&pending_batch.number)
                .ok_or(BlockFetcherError::InternalError(
                    "Verify log should be in the list.".into(),
                ))?;

            let batch_commit_tx_calldata = state
                .eth_client
                .get_transaction_by_hash(commit_log.transaction_hash)
                .await?
                .ok_or(BlockFetcherError::InternalError(format!(
                    "Failed to get the receipt for transaction {:x}",
                    commit_log.transaction_hash
                )))?
                .data;

            let batch_blocks = decode_batch_from_calldata(&batch_commit_tx_calldata)?;
            for block in batch_blocks.iter() {
                state.blockchain.add_block(block).await?;

                let block_hash = block.hash();

                apply_fork_choice(&state.store, block_hash, block_hash, block_hash).await?;

                info!(
                    "Added fetched block {} with hash {block_hash:#x}",
                    block.header.number,
                );
            }

            let mut batch =
                build_batch_from_blocks(state, &batch_blocks, pending_batch.number).await?;
            batch.commit_tx = Some(commit_log.transaction_hash);
            batch.verify_tx = Some(verify_log.transaction_hash);
            state.latest_safe_batch = batch.number;
            state.rollup_store.seal_batch(batch).await?;
        } else {
            // if the batch isn't verified yet, add it again to the queue
            state.pending_batches.push_front(pending_batch);
            break;
        }
    }
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

/// checks if a given batch is safe by accessing a mapping in the `OnChainProposer` contract.
/// If it returns true for the current batch, it means that it have been verified.
async fn batch_is_safe(
    state: &mut BlockFetcherState,
    last_block_hash: &H256,
) -> Result<bool, BlockFetcherError> {
    let values = vec![Value::FixedBytes(last_block_hash.0.to_vec().into())];

    let calldata = encode_calldata("verifiedBatches(bytes32)", &values)?;

    let result = state
        .eth_client
        .call(
            state.on_chain_proposer_address,
            calldata.into(),
            Overrides::default(),
        )
        .await?;

    let decoded_response = hex::decode(result.trim_start_matches("0x"))
        .map_err(|e| BlockFetcherError::InternalError(e.to_string()))?;

    let last_byte = decoded_response
        .last()
        .ok_or(BlockFetcherError::InternalError(
            "Response should have at least one byte.".to_string(),
        ))?;

    Ok(*last_byte > 0)
}

async fn build_batch_from_blocks(
    state: &mut BlockFetcherState,
    batch: &[Block],
    batch_number: u64,
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
        number: batch_number,
        first_block: first_block.header.number,
        last_block: last_block.header.number,
        state_root: new_state_root,
        privileged_transactions_hash,
        message_hashes: get_batch_message_hashes(state, batch).await?,
        blobs_bundle,
        commit_tx: None,
        verify_tx: None,
    })
}
