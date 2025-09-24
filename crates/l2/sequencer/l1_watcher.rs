use super::utils::random_duration;
use crate::based::sequencer_state::{SequencerState, SequencerStatus};
use crate::{EthConfig, L1WatcherConfig, SequencerConfig};
use crate::{sequencer::errors::L1WatcherError, utils::parse::hash_to_address};
use bytes::Bytes;
use ethereum_types::{Address, H256, U256};
use ethrex_blockchain::Blockchain;
use ethrex_common::types::{PrivilegedL2Transaction, TxType};
use ethrex_common::{H160, types::Transaction};
use ethrex_l2_sdk::{
    build_generic_tx, get_last_fetched_l1_block, get_pending_privileged_transactions,
};
use ethrex_rpc::clients::EthClientError;
use ethrex_rpc::types::receipt::RpcLog;
use ethrex_rpc::{
    clients::eth::{EthClient, Overrides},
    types::receipt::RpcLogInfo,
};
use ethrex_storage::Store;
use keccak_hash::keccak;
use serde::Serialize;
use spawned_concurrency::tasks::{
    CallResponse, CastResponse, GenServer, GenServerHandle, InitResult, Success, send_after,
};
use std::collections::BTreeMap;
use std::{cmp::min, sync::Arc};
use tracing::{debug, error, info, warn};

#[derive(Clone)]
pub enum CallMessage {
    Health,
}

#[derive(Clone)]
pub enum InMessage {
    Watch,
}

#[derive(Clone)]
pub enum OutMessage {
    Done,
    Error,
    Health(L1WatcherHealth),
}

pub struct L1Watcher {
    pub store: Store,
    pub blockchain: Arc<Blockchain>,
    pub eth_client: EthClient,
    pub l2_client: EthClient,
    pub address: Address,
    pub max_block_step: U256,
    pub last_block_fetched: U256,
    pub check_interval: u64,
    pub sequencer_state: SequencerState,
}

#[derive(Clone, Serialize)]
pub struct L1WatcherHealth {
    pub l1_rpc_healthcheck: BTreeMap<String, serde_json::Value>,
    pub l2_rpc_healthcheck: BTreeMap<String, serde_json::Value>,
    pub max_block_step: String,
    pub last_block_fetched: String,
    pub check_interval: u64,
    pub sequencer_state: String,
    pub bridge_address: Address,
}

impl L1Watcher {
    pub fn new(
        store: Store,
        blockchain: Arc<Blockchain>,
        eth_config: &EthConfig,
        watcher_config: &L1WatcherConfig,
        sequencer_state: SequencerState,
    ) -> Result<Self, L1WatcherError> {
        let eth_client = EthClient::new_with_multiple_urls(eth_config.rpc_url.clone())?;
        let l2_client = EthClient::new("http://localhost:1729")?;
        let last_block_fetched = U256::zero();
        Ok(Self {
            store,
            blockchain,
            eth_client,
            l2_client,
            address: watcher_config.bridge_address,
            max_block_step: watcher_config.max_block_step,
            last_block_fetched,
            check_interval: watcher_config.check_interval_ms,
            sequencer_state,
        })
    }

    pub async fn spawn(
        store: Store,
        blockchain: Arc<Blockchain>,
        cfg: SequencerConfig,
        sequencer_state: SequencerState,
    ) -> Result<GenServerHandle<Self>, L1WatcherError> {
        let state = Self::new(
            store,
            blockchain,
            &cfg.eth,
            &cfg.l1_watcher,
            sequencer_state,
        )?;
        Ok(state.start())
    }

    async fn watch(&mut self) {
        let Ok(logs) = self
            .get_privileged_transactions()
            .await
            .inspect_err(|err| error!("L1 Watcher Error: {err}"))
        else {
            return;
        };

        // We may not have a privileged transaction nor a withdrawal, that means no events -> no logs.
        if !logs.is_empty() {
            let _ = self
                .process_privileged_transactions(logs)
                .await
                .inspect_err(|err| error!("L1 Watcher Error: {}", err));
        };
    }

    pub async fn get_privileged_transactions(&mut self) -> Result<Vec<RpcLog>, L1WatcherError> {
        if self.last_block_fetched.is_zero() {
            self.last_block_fetched = get_last_fetched_l1_block(&self.eth_client, self.address)
                .await?
                .into();
        }

        let Some(latest_block_to_check) = self
            .eth_client
            .get_block_number()
            .await?
            .checked_sub(self.eth_client.config.safe_block_delay.into())
        else {
            warn!("Too close to genesis to request privileged transactions");
            return Ok(vec![]);
        };

        debug!(
            "Latest possible block number with {} blocks of delay: {latest_block_to_check} ({latest_block_to_check:#x})",
            self.eth_client.config.safe_block_delay,
        );

        // last_block_fetched could be greater than latest_block_to_check:
        // - Right after deploying the contract as latest_block_fetched is set to the block where the contract is deployed
        // - If the node is stopped and safe_block_delay is changed
        if self.last_block_fetched > latest_block_to_check {
            warn!("Last block fetched is greater than latest safe block");
            return Ok(vec![]);
        }

        let new_last_block = min(
            self.last_block_fetched + self.max_block_step,
            latest_block_to_check,
        );

        if self.last_block_fetched == latest_block_to_check {
            debug!("{:#x} ==  {:#x}", self.last_block_fetched, new_last_block);
            return Ok(vec![]);
        }

        debug!(
            "Looking logs from block {:#x} to {:#x}",
            self.last_block_fetched, new_last_block
        );

        // Matches the event PrivilegedTxSent from ICommonBridge.sol
        let topic =
            keccak(b"PrivilegedTxSent(address,address,address,uint256,uint256,uint256,bytes)");

        let logs = self
            .eth_client
            .get_logs(
                self.last_block_fetched + 1,
                new_last_block,
                self.address,
                vec![topic],
            )
            .await?;

        debug!("Logs: {:#?}", logs);

        // If we have an error adding the tx to the mempool we may assign it to the next
        // block to fetch, but we may lose a privileged tx.
        self.last_block_fetched = new_last_block;

        Ok(logs)
    }

    pub async fn process_privileged_transactions(
        &mut self,
        logs: Vec<RpcLog>,
    ) -> Result<Vec<H256>, L1WatcherError> {
        let mut privileged_txs = Vec::new();

        for log in logs {
            let privileged_transaction_data = PrivilegedTransactionData::from_log(log.log)?;

            let gas_price = self.l2_client.get_gas_price().await?;
            // Avoid panicking when using as_u64()
            let gas_price: u64 = gas_price
                .try_into()
                .map_err(|_| L1WatcherError::Custom("Failed at gas_price.try_into()".to_owned()))?;

            let chain_id = self
                .store
                .get_chain_config()
                .map_err(|e| L1WatcherError::FailedToRetrieveChainConfig(e.to_string()))?
                .chain_id;

            let mint_transaction = privileged_transaction_data
                .into_tx(&self.eth_client, chain_id, gas_price)
                .await?;

            let tx = Transaction::PrivilegedL2Transaction(mint_transaction);

            if self
                .privileged_transaction_already_processed(tx.hash())
                .await?
            {
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

            let Ok(hash) = self
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
        &mut self,
        tx_hash: H256,
    ) -> Result<bool, L1WatcherError> {
        if self
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
        let pending_privileged_transactions =
            get_pending_privileged_transactions(&self.eth_client, self.address).await?;
        Ok(!pending_privileged_transactions.contains(&tx_hash))
    }

    async fn health(&mut self) -> CallResponse<Self> {
        let l1_rpc_healthcheck = self.eth_client.test_urls().await;
        let l2_rpc_healthcheck = self.l2_client.test_urls().await;

        CallResponse::Reply(OutMessage::Health(L1WatcherHealth {
            l1_rpc_healthcheck,
            l2_rpc_healthcheck,
            max_block_step: self.max_block_step.to_string(),
            last_block_fetched: self.last_block_fetched.to_string(),
            check_interval: self.check_interval,
            sequencer_state: format!("{:?}", self.sequencer_state.status().await),
            bridge_address: self.address,
        }))
    }
}

impl GenServer for L1Watcher {
    type CallMsg = CallMessage;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = L1WatcherError;

    async fn init(self, handle: &GenServerHandle<Self>) -> Result<InitResult<Self>, Self::Error> {
        // Perform the check and suscribe a periodic Watch.
        handle
            .clone()
            .cast(Self::CastMsg::Watch)
            .await
            .map_err(Self::Error::InternalError)?;
        Ok(Success(self))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            Self::CastMsg::Watch => {
                if let SequencerStatus::Sequencing = self.sequencer_state.status().await {
                    self.watch().await;
                }
                let check_interval = random_duration(self.check_interval);
                send_after(check_interval, handle.clone(), Self::CastMsg::Watch);
                CastResponse::NoReply
            }
        }
    }

    async fn handle_call(
        &mut self,
        message: Self::CallMsg,
        _handle: &GenServerHandle<Self>,
    ) -> spawned_concurrency::tasks::CallResponse<Self> {
        match message {
            CallMessage::Health => self.health().await,
        }
    }
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
        /*
            event PrivilegedTxSent (
                address indexed L1from, => part of topics, not data
                address from, => 0..32
                address to, => 32..64
                uint256 transactionId, => 64..96
                uint256 value, => 96..128
                uint256 gasLimit, => 128..160
                bytes data
                    => offset_data => 160..192
                    => length_data => 192..224
                    => data => 224..
            );
            Any value that is not 32 bytes is padded with zeros.
        */

        let from = H256::from_slice(log.data.get(0..32).ok_or(
            L1WatcherError::FailedToDeserializeLog(
                "Failed to parse gas_limit from log: log.data[0..32] out of bounds".to_owned(),
            ),
        )?);
        let from_address = hash_to_address(from);

        let to = H256::from_slice(log.data.get(32..64).ok_or(
            L1WatcherError::FailedToDeserializeLog(
                "Failed to parse gas_limit from log: log.data[32..64] out of bounds".to_owned(),
            ),
        )?);
        let to_address = hash_to_address(to);

        let transaction_id = U256::from_big_endian(log.data.get(64..96).ok_or(
            L1WatcherError::FailedToDeserializeLog(
                "Failed to parse gas_limit from log: log.data[64..96] out of bounds".to_owned(),
            ),
        )?);

        let value = U256::from_big_endian(log.data.get(96..128).ok_or(
            L1WatcherError::FailedToDeserializeLog(
                "Failed to parse gas_limit from log: log.data[96..128] out of bounds".to_owned(),
            ),
        )?);

        let gas_limit = U256::from_big_endian(log.data.get(128..160).ok_or(
            L1WatcherError::FailedToDeserializeLog(
                "Failed to parse gas_limit from log: log.data[128..160] out of bounds".to_owned(),
            ),
        )?);

        // 160..192 is taken by offset_data, which we do not need

        let calldata_len = U256::from_big_endian(
            log.data
                .get(192..224)
                .ok_or(L1WatcherError::FailedToDeserializeLog(
                    "Failed to parse calldata_len from log: log.data[192..224] out of bounds"
                        .to_owned(),
                ))?,
        );

        let calldata = log
            .data
            .get(224..224 + calldata_len.as_usize())
            .ok_or(L1WatcherError::FailedToDeserializeLog(
            "Failed to parse calldata from log: log.data[224..224 + calldata_len] out of bounds"
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
        let generic_tx = build_generic_tx(
            eth_client,
            TxType::Privileged,
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
        .await?;
        Ok(generic_tx.try_into()?)
    }
}
