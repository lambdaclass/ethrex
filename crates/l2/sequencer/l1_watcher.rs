use super::utils::random_duration;
use crate::based::sequencer_state::{SequencerState, SequencerStatus};
use crate::{EthConfig, L1WatcherConfig, SequencerConfig};
use crate::{sequencer::errors::L1WatcherError, utils::parse::hash_to_address};
use bytes::Bytes;
use ethereum_types::{Address, H256};
use ethrex_blockchain::Blockchain;
use ethrex_common::U256;
use ethrex_common::types::PrivilegedL2Transaction;
use ethrex_common::{H160, types::Transaction};
use ethrex_rpc::clients::EthClientError;
use ethrex_rpc::types::receipt::RpcLog;
use ethrex_rpc::{
    clients::eth::{EthClient, eth_sender::Overrides},
    types::receipt::RpcLogInfo,
};
use ethrex_storage::Store;
use keccak_hash::keccak;
use spawned_concurrency::messages::Unused;
use spawned_concurrency::tasks::{CastResponse, GenServer, GenServerHandle, send_after};
use std::{cmp::min, sync::Arc};
use tracing::{debug, error, info, warn};

#[derive(Clone)]
pub struct L1WatcherState {
    pub store: Store,
    pub blockchain: Arc<Blockchain>,
    pub eth_client: EthClient,
    pub l2_client: EthClient,
    pub address: Address,
    pub max_block_step: U256,
    pub last_block_fetched: U256,
    pub check_interval: u64,
    pub l1_block_delay: u64,
    pub sequencer_state: SequencerState,
}

impl L1WatcherState {
    pub fn new(
        store: Store,
        blockchain: Arc<Blockchain>,
        eth_config: &EthConfig,
        watcher_config: &L1WatcherConfig,
        sequencer_state: SequencerState,
    ) -> Result<Self, L1WatcherError> {
        let eth_client = EthClient::new_with_multiple_urls(eth_config.rpc_url.clone())?;
        let l2_client = EthClient::new("http://localhost:1729")?;
        let last_block_fetched = U256::ZERO;
        Ok(Self {
            store,
            blockchain,
            eth_client,
            l2_client,
            address: watcher_config.bridge_address,
            max_block_step: watcher_config.max_block_step,
            last_block_fetched,
            check_interval: watcher_config.check_interval_ms,
            l1_block_delay: watcher_config.watcher_block_delay,
            sequencer_state,
        })
    }
}

#[derive(Clone)]
pub enum InMessage {
    Watch,
}

#[allow(dead_code)]
#[derive(Clone, PartialEq)]
pub enum OutMessage {
    Done,
    Error,
}

pub struct L1Watcher;

impl L1Watcher {
    pub async fn spawn(
        store: Store,
        blockchain: Arc<Blockchain>,
        cfg: SequencerConfig,
        sequencer_state: SequencerState,
    ) -> Result<(), L1WatcherError> {
        let state = L1WatcherState::new(
            store,
            blockchain,
            &cfg.eth,
            &cfg.l1_watcher,
            sequencer_state,
        )?;
        L1Watcher::start(state);
        Ok(())
    }
}

impl GenServer for L1Watcher {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type State = L1WatcherState;
    type Error = L1WatcherError;

    fn new() -> Self {
        Self {}
    }

    async fn init(
        &mut self,
        handle: &GenServerHandle<Self>,
        state: Self::State,
    ) -> Result<Self::State, Self::Error> {
        // Perform the check and suscribe a periodic Watch.
        handle
            .clone()
            .cast(Self::CastMsg::Watch)
            .await
            .map_err(Self::Error::GenServerError)?;
        Ok(state)
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
        mut state: Self::State,
    ) -> CastResponse<Self> {
        match message {
            Self::CastMsg::Watch => {
                if let SequencerStatus::Sequencing = state.sequencer_state.status().await {
                    watch(&mut state).await;
                }
                let check_interval = random_duration(state.check_interval);
                send_after(check_interval, handle.clone(), Self::CastMsg::Watch);
                CastResponse::NoReply(state)
            }
        }
    }
}

async fn watch(state: &mut L1WatcherState) {
    let Ok(logs) = get_privileged_transactions(state)
        .await
        .inspect_err(|err| error!("L1 Watcher Error: {err}"))
    else {
        return;
    };

    // We may not have a privileged transaction nor a withdrawal, that means no events -> no logs.
    if !logs.is_empty() {
        let _ = process_privileged_transactions(state, logs)
            .await
            .inspect_err(|err| error!("L1 Watcher Error: {}", err));
    };
}

pub async fn get_privileged_transactions(
    state: &mut L1WatcherState,
) -> Result<Vec<RpcLog>, L1WatcherError> {
    if state.last_block_fetched == U256::ZERO {
        state.last_block_fetched = state
            .eth_client
            .get_last_fetched_l1_block(state.address)
            .await?
            .into();
    }

    let Some(latest_block_to_check) = state
        .eth_client
        .get_block_number()
        .await?
        .checked_sub(state.l1_block_delay.into())
    else {
        warn!("Too close to genesis to request privileged transactions");
        return Ok(vec![]);
    };

    debug!(
        "Latest possible block number with {} blocks of delay: {latest_block_to_check} ({latest_block_to_check:#x})",
        state.l1_block_delay,
    );

    // last_block_fetched could be greater than latest_block_to_check:
    // - Right after deploying the contract as latest_block_fetched is set to the block where the contract is deployed
    // - If the node is stopped and l1_block_delay is changed
    if state.last_block_fetched > latest_block_to_check {
        warn!("Last block fetched is greater than latest safe block");
        return Ok(vec![]);
    }

    let new_last_block = min(
        state.last_block_fetched + state.max_block_step,
        latest_block_to_check,
    );

    debug!(
        "Looking logs from block {:#x} to {:#x}",
        state.last_block_fetched, new_last_block
    );

    // Matches the event PrivilegedTxSent from ICommonBridge.sol
    let topic = keccak(b"PrivilegedTxSent(address,address,uint256,uint256,uint256,bytes)");

    let logs = state
        .eth_client
        .get_logs(
            state.last_block_fetched + 1,
            new_last_block,
            state.address,
            topic,
        )
        .await
        .inspect_err(|error| {
            // We may get an error if the RPC doesn't has the logs for the requested
            // block interval. For example, Light Nodes.
            warn!("Error when getting logs from L1: {}", error);
        })
        .unwrap_or_default();

    debug!("Logs: {:#?}", logs);

    // If we have an error adding the tx to the mempool we may assign it to the next
    // block to fetch, but we may lose a privileged tx.
    state.last_block_fetched = new_last_block;

    Ok(logs)
}

pub async fn process_privileged_transactions(
    state: &L1WatcherState,
    logs: Vec<RpcLog>,
) -> Result<Vec<H256>, L1WatcherError> {
    let mut privileged_txs = Vec::new();

    for log in logs {
        let privileged_transaction_data = PrivilegedTransactionData::from_log(log.log)?;

        let gas_price = state.l2_client.get_gas_price().await?;
        // Avoid panicking when using as_u64()
        let gas_price: u64 = gas_price
            .try_into()
            .map_err(|_| L1WatcherError::Custom("Failed at gas_price.try_into()".to_owned()))?;

        let chain_id = state
            .store
            .get_chain_config()
            .map_err(|e| L1WatcherError::FailedToRetrieveChainConfig(e.to_string()))?
            .chain_id;

        let mint_transaction = privileged_transaction_data
            .into_tx(&state.eth_client, chain_id, gas_price)
            .await?;

        let tx = Transaction::PrivilegedL2Transaction(mint_transaction);

        if privileged_transaction_already_processed(state, tx.compute_hash()).await? {
            warn!(
                "Privileged transaction already processed (to: {:x}, value: {:x}, transactionId: {:#}), skipping.",
                privileged_transaction_data.to_address,
                privileged_transaction_data.value,
                privileged_transaction_data.transaction_id
            );
            continue;
        }

        info!(
            "Initiating mint transaction for {:x} with value {:x} and transactionId: {:#}",
            privileged_transaction_data.to_address,
            privileged_transaction_data.value,
            privileged_transaction_data.transaction_id
        );

        let Ok(hash) = state
            .blockchain
            .add_transaction_to_pool(tx)
            .await
            .inspect_err(|e| warn!("Failed to add mint transaction to the mempool: {e:#?}"))
        else {
            // TODO: Figure out if we want to continue or not
            continue;
        };

        info!("Mint transaction added to mempool {hash:#x}",);
        privileged_txs.push(hash);
    }

    Ok(privileged_txs)
}

async fn privileged_transaction_already_processed(
    state: &L1WatcherState,
    tx_hash: H256,
) -> Result<bool, L1WatcherError> {
    if state
        .store
        .get_transaction_by_hash(tx_hash)
        .await
        .map_err(L1WatcherError::FailedAccessingStore)?
        .is_some()
    {
        return Ok(true);
    }

    // If we have a reconstructed state, we don't have the transaction in our store.
    // Check if the transaction is marked as pending in the contract.
    let pending_privileged_transactions = state
        .eth_client
        .get_pending_privileged_transactions(state.address)
        .await?;
    Ok(!pending_privileged_transactions.contains(&tx_hash))
}

pub struct PrivilegedTransactionData {
    pub value: U256,
    pub to_address: H160,
    pub transaction_id: U256,
    pub from: H160,
    pub gas_limit: U256,
    pub calldata: Vec<u8>,
}

impl PrivilegedTransactionData {
    pub fn from_log(log: RpcLogInfo) -> Result<PrivilegedTransactionData, L1WatcherError> {
        let from = log
            .topics
            .get(1)
            .ok_or(L1WatcherError::FailedToDeserializeLog(
                "Failed to parse mint value from log: log.topics[1] out of bounds".to_owned(),
            ))?;

        let from_address = hash_to_address(*from);

        let to_address_hash = log
            .topics
            .get(2)
            .ok_or(L1WatcherError::FailedToDeserializeLog(
                "Failed to parse beneficiary from log: log.topics[2] out of bounds".to_owned(),
            ))?;
        let to_address = hash_to_address(*to_address_hash);

        let transaction_id = log
            .topics
            .get(3)
            .ok_or(L1WatcherError::FailedToDeserializeLog(
                "Failed to parse beneficiary from log: log.topics[3] out of bounds".to_owned(),
            ))?;

        let transaction_id = format!("{transaction_id:#x}")
            .parse::<U256>()
            .map_err(|e| {
                L1WatcherError::FailedToDeserializeLog(format!(
                    "Failed to parse transactionId value from log: {e:#?}"
                ))
            })?;

        // The previous values are indexed in the topic of the log. Data contains the rest.
        // DATA = value: uint256 || gas_limit: uint256 || offset_calldata: uint256 || length_calldata: uint256 || calldata: bytes
        // DATA = 0..32          || 32..64             || 64..96                   || 96..128                  || 128..(128+calldata_len)
        // Any value that is not 32 bytes is padded with zeros.

        let value = U256::from_be_bytes(
            log.data
                .get(0..32)
                .ok_or(L1WatcherError::FailedToDeserializeLog(
                    "Failed to parse gas_limit from log: log.data[32..64] out of bounds".to_owned(),
                ))?
                .try_into()
                .map_err(|_| L1WatcherError::Custom("Conversion error".to_string()))?,
        );

        let gas_limit = U256::from_be_bytes(
            log.data
                .get(32..64)
                .ok_or(L1WatcherError::FailedToDeserializeLog(
                    "Failed to parse gas_limit from log: log.data[32..64] out of bounds".to_owned(),
                ))?
                .try_into()
                .map_err(|_| L1WatcherError::Custom("Conversion error".to_string()))?,
        );

        let calldata_len = U256::from_be_bytes(
            log.data
                .get(96..128)
                .ok_or(L1WatcherError::FailedToDeserializeLog(
                    "Failed to parse calldata_len from log: log.data[96..128] out of bounds"
                        .to_owned(),
                ))?
                .try_into()
                .map_err(|_| L1WatcherError::Custom("Conversion error".to_string()))?,
        );
        let calldata = log
            .data
            .get(128..128 + calldata_len.as_usize())
            .ok_or(L1WatcherError::FailedToDeserializeLog(
            "Failed to parse calldata from log: log.data[128..128 + calldata_len] out of bounds"
                .to_owned(),
        ))?;

        Ok(Self {
            value,
            to_address,
            transaction_id,
            from: from_address,
            gas_limit,
            calldata: calldata.to_vec(),
        })
    }
    pub async fn into_tx(
        &self,
        eth_client: &EthClient,
        chain_id: u64,
        gas_price: u64,
    ) -> Result<PrivilegedL2Transaction, EthClientError> {
        eth_client
            .build_privileged_transaction(
                self.to_address,
                self.from,
                Bytes::copy_from_slice(&self.calldata),
                Overrides {
                    chain_id: Some(chain_id),
                    // Using the transaction_id as nonce.
                    // If we make a transaction on the L2 with this address, we may break the
                    // privileged transaction workflow.
                    nonce: Some(self.transaction_id.as_u64()),
                    value: Some(self.value),
                    gas_limit: Some(self.gas_limit.as_u64()),
                    // TODO(CHECK): Seems that when we start the L2, we need to set the gas.
                    // Otherwise, the transaction is not included in the mempool.
                    // We should override the blockchain to always include the transaction.
                    max_fee_per_gas: Some(gas_price),
                    max_priority_fee_per_gas: Some(gas_price),
                    ..Default::default()
                },
            )
            .await
    }
}
