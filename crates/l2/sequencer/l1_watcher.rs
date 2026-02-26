use super::utils::random_duration;
use crate::sequencer::errors::L1WatcherError;
use crate::{EthConfig, L1WatcherConfig, SequencerConfig};
use ethereum_types::{Address, H256, U256};
use ethrex_blockchain::{Blockchain, BlockchainType};
use ethrex_common::types::{Log, PrivilegedL2Transaction, Transaction, TxKind};
use ethrex_common::utils::keccak;
use ethrex_l2_common::messages::{
    L2MESSAGE_EVENT_SELECTOR, L2Message, MESSENGER_ADDRESS, get_l2_message_hash,
};
use ethrex_l2_common::sequencer_state::{SequencerState, SequencerStatus};
use ethrex_l2_sdk::privileged_data::PrivilegedTransactionData;
use ethrex_l2_sdk::{get_last_fetched_l1_block, get_pending_l1_messages, get_pending_l2_messages};
use ethrex_rpc::clients::eth::EthClient;
use ethrex_rpc::types::block_identifier::{BlockIdentifier, BlockTag};
use ethrex_rpc::types::receipt::RpcLog;
use ethrex_storage::Store;
use reqwest::Url;
use serde::Serialize;
use spawned_concurrency::tasks::{
    CallResponse, CastResponse, GenServer, GenServerHandle, InitResult, Success, send_after,
};
use std::collections::BTreeMap;
use std::time::Duration;
use std::{cmp::min, sync::Arc};
use tracing::{debug, error, info, warn};

#[derive(Clone)]
pub enum CallMessage {
    Health,
}

#[derive(Clone)]
pub enum InMessage {
    WatchLogsL1,
    WatchLogsL2,
    UpdateL1BlobBaseFee,
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
    pub this_l2_client: EthClient,
    pub l2_clients: Vec<L2Client>,
    pub bridge_address: Address,
    pub router_address: Address,
    pub max_block_step: U256,
    pub last_block_fetched_l1: U256,
    pub check_interval: u64,
    pub l1_block_delay: u64,
    pub sequencer_state: SequencerState,
    pub l1_blob_base_fee_update_interval: u64,
    pub chain_id_topic: H256,
}

pub struct L2Client {
    pub eth_client: EthClient,
    pub last_block_fetched_l2: U256,
    pub chain_id: u64,
}

#[derive(Clone, Serialize)]
pub struct L1WatcherHealth {
    pub l1_rpc_healthcheck: BTreeMap<String, serde_json::Value>,
    pub max_block_step: String,
    pub last_block_fetched: String,
    pub check_interval: u64,
    pub l1_block_delay: u64,
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
        l2_url: Url,
    ) -> Result<Self, L1WatcherError> {
        let eth_client = EthClient::new_with_multiple_urls(eth_config.rpc_url.clone())?;
        let this_l2_client = EthClient::new(l2_url)?;
        let mut l2_clients: Vec<L2Client> = vec![];
        info!(
            "Configuring L1 Watcher L2 clients {:?} {:?}",
            watcher_config.l2_rpc_urls, watcher_config.l2_chain_ids
        );
        for (url, chain_id) in watcher_config
            .l2_rpc_urls
            .iter()
            .zip(watcher_config.l2_chain_ids.clone())
        {
            info!(
                "Adding L2 client with URL: {} and chain ID: {}",
                url, chain_id
            );
            let l2_client = EthClient::new(url.clone())?;
            l2_clients.push(L2Client {
                eth_client: l2_client,
                last_block_fetched_l2: U256::zero(),
                chain_id,
            });
        }
        let last_block_fetched = U256::zero();
        let chain_id_topic = {
            let u256 = U256::from(store.get_chain_config().chain_id);
            let bytes = u256.to_big_endian();
            H256(bytes)
        };

        let router_address = watcher_config.router_address;

        Ok(Self {
            store,
            blockchain,
            eth_client,
            this_l2_client,
            l2_clients,
            bridge_address: watcher_config.bridge_address,
            router_address,
            max_block_step: watcher_config.max_block_step,
            last_block_fetched_l1: last_block_fetched,
            check_interval: watcher_config.check_interval_ms,
            l1_block_delay: watcher_config.watcher_block_delay,
            sequencer_state,
            l1_blob_base_fee_update_interval: watcher_config.l1_blob_base_fee_update_interval,
            chain_id_topic,
        })
    }

    pub fn spawn(
        store: Store,
        blockchain: Arc<Blockchain>,
        cfg: SequencerConfig,
        sequencer_state: SequencerState,
        l2_url: Url,
    ) -> Result<GenServerHandle<Self>, L1WatcherError> {
        let state = Self::new(
            store,
            blockchain,
            &cfg.eth,
            &cfg.l1_watcher,
            sequencer_state,
            l2_url,
        )?;
        Ok(state.start())
    }

    async fn watch_l1(&mut self) {
        let Ok(logs) = self
            .get_logs_l1()
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

    async fn get_logs_l1(&mut self) -> Result<Vec<RpcLog>, L1WatcherError> {
        // Matches the event PrivilegedTxSent from ICommonBridge.sol
        let topic =
            keccak(b"PrivilegedTxSent(address,address,address,uint256,uint256,uint256,bytes)");
        if self.last_block_fetched_l1.is_zero() {
            self.last_block_fetched_l1 =
                get_last_fetched_l1_block(&self.eth_client, self.bridge_address)
                    .await?
                    .into();
        }
        let (last_block_fetched, logs) = Self::get_privileged_transactions(
            self.last_block_fetched_l1,
            self.l1_block_delay,
            &self.eth_client,
            vec![topic],
            self.bridge_address,
            self.max_block_step,
        )
        .await?;
        self.last_block_fetched_l1 = last_block_fetched;
        Ok(logs)
    }

    pub async fn get_privileged_transactions(
        last_block_fetched: U256,
        block_delay: u64,
        client: &EthClient,
        topics: Vec<H256>,
        address: Address,
        max_block_step: U256,
    ) -> Result<(U256, Vec<RpcLog>), L1WatcherError> {
        info!("Getting privileged transactions");
        let Some(latest_block_to_check) = client
            .get_block_number()
            .await?
            .checked_sub(block_delay.into())
        else {
            warn!("Too close to genesis to request privileged transactions");
            return Ok((last_block_fetched, vec![]));
        };

        debug!(
            "Latest possible block number with {} blocks of delay: {latest_block_to_check} ({latest_block_to_check:#x})",
            block_delay,
        );

        // last_block_fetched could be greater than latest_block_to_check:
        // - Right after deploying the contract as latest_block_fetched is set to the block where the contract is deployed
        // - If the node is stopped and l1_block_delay is changed
        if last_block_fetched > latest_block_to_check {
            warn!("Last block fetched is greater than latest safe block");
            return Ok((last_block_fetched, vec![]));
        }

        let new_last_block = min(last_block_fetched + max_block_step, latest_block_to_check);

        if last_block_fetched == latest_block_to_check {
            debug!("{:#x} ==  {:#x}", last_block_fetched, new_last_block);
            return Ok((last_block_fetched, vec![]));
        }

        debug!(
            "Looking logs from block {:#x} to {:#x}",
            last_block_fetched, new_last_block
        );

        let logs = client
            .get_logs(last_block_fetched + 1, new_last_block, address, topics)
            .await?;

        // If we have an error adding the tx to the mempool we may assign it to the next
        // block to fetch, but we may lose a privileged tx.
        Ok((new_last_block, logs))
    }

    pub async fn process_privileged_transactions(
        &mut self,
        logs: Vec<RpcLog>,
    ) -> Result<Vec<H256>, L1WatcherError> {
        let mut privileged_txs = Vec::new();

        let pending_privileged_transactions =
            get_pending_l1_messages(&self.eth_client, self.bridge_address).await?;

        for log in logs {
            let privileged_transaction_data = PrivilegedTransactionData::from_log(log.log)
                .map_err(L1WatcherError::FailedToDeserializeLog)?;

            let chain_id = self.this_l2_client.get_chain_id().await?.as_u64();

            let gas_price = self.this_l2_client.get_gas_price().await?;
            // Avoid panicking when using as_u64()
            let gas_price: u64 = gas_price
                .try_into()
                .map_err(|_| L1WatcherError::Custom("Failed at gas_price.try_into()".to_owned()))?;

            // We should actually delete the gas price field from privileged transactions.
            let mint_transaction = privileged_transaction_data
                .into_tx(&self.eth_client, chain_id, gas_price)
                .await?;

            let tx = Transaction::PrivilegedL2Transaction(mint_transaction);

            if self
                .privileged_transaction_already_processed(
                    tx.hash(),
                    &pending_privileged_transactions,
                )
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

    async fn process_l2_transactions(
        &mut self,
        l2_txs: Vec<(L2Message, u64)>,
    ) -> Result<(), L1WatcherError> {
        let mut privileged_txs = Vec::new();

        let gas_price = self.this_l2_client.get_gas_price().await?;
        // Avoid panicking when using as_u64()
        let gas_price: u64 = gas_price
            .try_into()
            .map_err(|_| L1WatcherError::Custom("Failed at gas_price.try_into()".to_owned()))?;

        for (tx, source_chain_id) in l2_txs {
            info!("Add mint tx with nonce: {}", tx.tx_id.as_u64());

            let mint_transaction = PrivilegedL2Transaction {
                chain_id: source_chain_id,
                nonce: tx.tx_id.as_u64(),
                max_priority_fee_per_gas: gas_price,
                max_fee_per_gas: gas_price,
                gas_limit: tx.gas_limit.as_u64(),
                to: TxKind::Call(tx.to),
                value: tx.value,
                data: tx.data.clone(),
                access_list: vec![],
                from: tx.from,
                inner_hash: Default::default(),
                sender_cache: Default::default(),
            };

            let privileged_tx = Transaction::PrivilegedL2Transaction(mint_transaction);

            if self
                .store
                .get_transaction_by_hash(privileged_tx.hash())
                .await
                .map_err(L1WatcherError::FailedAccessingStore)?
                .is_some()
            {
                warn!(
                    "L2 transaction already processed (to: {:x}, value: {:x}, transactionId: {:#}), skipping.",
                    tx.to, tx.value, tx.tx_id
                );
                continue;
            }

            let Ok(hash) = self
                .blockchain
                .add_transaction_to_pool(privileged_tx)
                .await
                .inspect_err(|e| warn!("Failed to add mint transaction to the mempool: {e:#?}"))
            else {
                // TODO: Figure out if we want to continue or not
                continue;
            };

            info!("L2 Mint transaction added to mempool {hash:#x}",);
            privileged_txs.push(hash);
        }
        Ok(())
    }

    async fn privileged_transaction_already_processed(
        &self,
        tx_hash: H256,
        pending_privileged_transactions: &[H256],
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
        Ok(!pending_privileged_transactions.contains(&tx_hash))
    }

    async fn health(&self) -> CallResponse<Self> {
        let l1_rpc_healthcheck = self.eth_client.test_urls().await;

        CallResponse::Reply(OutMessage::Health(L1WatcherHealth {
            l1_rpc_healthcheck,
            max_block_step: self.max_block_step.to_string(),
            last_block_fetched: self.last_block_fetched_l1.to_string(),
            check_interval: self.check_interval,
            l1_block_delay: self.l1_block_delay,
            sequencer_state: format!("{:?}", self.sequencer_state.status()),
            bridge_address: self.bridge_address,
        }))
    }

    async fn watch_l2s(&mut self) {
        let Ok(l2_txs) = self
            .get_logs_l2()
            .await
            .inspect_err(|err| error!("L1 Watcher Error: {err}"))
        else {
            return;
        };

        info!("Fetched {} L2 logs", l2_txs.len());

        // We may not have a privileged transaction nor a withdrawal, that means no events -> no logs.
        if !l2_txs.is_empty() {
            let _ = self
                .process_l2_transactions(l2_txs)
                .await
                .inspect_err(|err| error!("L1 Watcher Error: {}", err));
        };
    }

    async fn get_logs_l2(&mut self) -> Result<Vec<(L2Message, u64)>, L1WatcherError> {
        info!("Getting L2 logs");
        let topics = vec![*L2MESSAGE_EVENT_SELECTOR, self.chain_id_topic];
        // We don't need to delay L2 logs
        let block_delay = 0;
        let mut acc_logs = Vec::new();

        // TODO: On errors, we may want to try updating the rest of L2 clients.
        for l2_client in &mut self.l2_clients {
            debug!(
                "Fetching logs from block {}",
                l2_client.last_block_fetched_l2
            );
            let (new_last_block, logs) = Self::get_privileged_transactions(
                l2_client.last_block_fetched_l2,
                block_delay,
                &l2_client.eth_client,
                topics.clone(),
                MESSENGER_ADDRESS,
                self.max_block_step,
            )
            .await?;

            info!("Fetched {} L2 logs from L2 client", logs.len());

            if logs.is_empty() {
                // No logs, just update the last block fetched.
                l2_client.last_block_fetched_l2 = new_last_block;
                continue;
            }

            let verified_logs =
                filter_verified_messages(self.bridge_address, &self.eth_client, l2_client, logs)
                    .await?;

            info!("Verified {} L2 logs from L2 client", verified_logs.len());

            // We need to update the last block fetched only if the logs were verified.
            if let Some((_, block_number, _)) = verified_logs.last() {
                l2_client.last_block_fetched_l2 = (*block_number).into();
            }

            acc_logs.extend(verified_logs);
        }
        Ok(acc_logs
            .iter()
            .map(|(msg, _, chain_id)| (msg.clone(), *chain_id))
            .collect())
    }
}

pub async fn filter_verified_messages(
    bridge_address: Address,
    l1_client: &EthClient,
    l2_client: &L2Client,
    logs: Vec<RpcLog>,
) -> Result<Vec<(L2Message, u64, u64)>, L1WatcherError> {
    let mut verified_logs = Vec::new();
    debug!("Filtering L2 messages");

    // Check if the transaction is marked as pending in the contract.
    let pending_l2_messages =
        get_pending_l2_messages(l1_client, bridge_address, l2_client.chain_id).await?;

    debug!("Pending l2 messages {:?}", pending_l2_messages);

    for rpc_log in logs {
        let log = Log {
            address: rpc_log.log.address,
            topics: rpc_log.log.topics.clone(),
            data: rpc_log.log.data.clone(),
        };

        let Some(l2_message) = L2Message::from_log(&log, l2_client.chain_id) else {
            return Err(L1WatcherError::FailedToDeserializeLog(
                "Failed to parse L2Message from log".to_owned(),
            ));
        };

        debug!("l2 message parsed from log: {:?}", l2_message);

        let message_hash = get_l2_message_hash(&l2_message);
        if !pending_l2_messages.contains(&message_hash) {
            info!("L2 message not found in pending messages: {message_hash:#x}");
            // Message not verified.
            // Given that logs are fetched in block order, we can stop here.
            break;
        }
        info!("L2 message verified: {message_hash:#x}");

        verified_logs.push((l2_message, rpc_log.block_number, l2_client.chain_id));
    }

    Ok(verified_logs)
}

impl GenServer for L1Watcher {
    type CallMsg = CallMessage;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type Error = L1WatcherError;

    async fn init(self, handle: &GenServerHandle<Self>) -> Result<InitResult<Self>, Self::Error> {
        // Perform the first log watch and schedule periodic checks.
        handle
            .clone()
            .cast(Self::CastMsg::WatchLogsL1)
            .await
            .map_err(Self::Error::InternalError)?;

        // Perform the first l2 log watch and schedule periodic checks.
        handle
            .clone()
            .cast(Self::CastMsg::WatchLogsL2)
            .await
            .map_err(Self::Error::InternalError)?;

        // Perform the first L1 blob base fee update and schedule periodic updates.
        handle
            .clone()
            .cast(InMessage::UpdateL1BlobBaseFee)
            .await
            .map_err(L1WatcherError::InternalError)?;
        Ok(Success(self))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            Self::CastMsg::WatchLogsL1 => {
                if let SequencerStatus::Sequencing = self.sequencer_state.status() {
                    self.watch_l1().await;
                }
                let check_interval = random_duration(self.check_interval);
                send_after(check_interval, handle.clone(), Self::CastMsg::WatchLogsL1);
                CastResponse::NoReply
            }
            Self::CastMsg::WatchLogsL2 => {
                info!("Watching L2 logs");
                if let SequencerStatus::Sequencing = self.sequencer_state.status() {
                    self.watch_l2s().await;
                }
                let check_interval = random_duration(self.check_interval);
                send_after(check_interval, handle.clone(), Self::CastMsg::WatchLogsL2);
                CastResponse::NoReply
            }
            Self::CastMsg::UpdateL1BlobBaseFee => {
                info!("Updating L1 blob base fee");
                let Ok(blob_base_fee) = self
                    .eth_client
                    .get_blob_base_fee(BlockIdentifier::Tag(BlockTag::Latest))
                    .await
                    .inspect_err(|e| {
                        error!("Failed to fetch L1 blob base fee: {e}");
                    })
                else {
                    return CastResponse::NoReply;
                };

                info!("Fetched L1 blob base fee: {blob_base_fee}");

                let BlockchainType::L2(l2_config) = &self.blockchain.options.r#type else {
                    error!("Invalid blockchain type. Expected L2.");
                    return CastResponse::NoReply;
                };

                let Ok(mut fee_config_guard) = l2_config.fee_config.write() else {
                    error!("Fee config lock was poisoned when updating L1 blob base fee");
                    return CastResponse::NoReply;
                };

                let Some(l1_fee_config) = fee_config_guard.l1_fee_config.as_mut() else {
                    warn!("L1 fee config is not set. Skipping L1 blob base fee update.");
                    return CastResponse::NoReply;
                };

                info!(
                    "Updating L1 blob base fee from {} to {}",
                    l1_fee_config.l1_fee_per_blob_gas, blob_base_fee
                );

                l1_fee_config.l1_fee_per_blob_gas = blob_base_fee;

                let interval = Duration::from_millis(self.l1_blob_base_fee_update_interval);
                send_after(interval, handle.clone(), Self::CastMsg::UpdateL1BlobBaseFee);
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
