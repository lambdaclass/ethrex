use crate::{sequencer::errors::L1WatcherError, utils::parse::hash_to_address};
use crate::{EthConfig, L1WatcherConfig, SequencerConfig};
use bytes::Bytes;
use ethereum_types::{Address, H256, U256};
use ethrex_blockchain::Blockchain;
use ethrex_common::{types::Transaction, H160};
use ethrex_rpc::types::receipt::RpcLog;
use ethrex_rpc::{
    clients::eth::{eth_sender::Overrides, EthClient},
    types::receipt::RpcLogInfo,
};
use ethrex_storage::Store;
use keccak_hash::keccak;
use std::{cmp::min, sync::Arc};
use tracing::{debug, error, info, warn};

use super::errors::SequencerError;
use super::utils::sleep_random;

pub async fn start_l1_watcher(
    store: Store,
    blockchain: Arc<Blockchain>,
    cfg: SequencerConfig,
) -> Result<(), SequencerError> {
    let mut l1_watcher = L1Watcher::new_from_config(&cfg.l1_watcher, &cfg.eth).await?;
    l1_watcher.run(&store, &blockchain).await;
    Ok(())
}

pub struct L1Watcher {
    eth_client: EthClient,
    l2_client: EthClient,
    address: Address,
    max_block_step: U256,
    last_block_fetched: U256,
    check_interval: u64,
    l1_block_delay: u64,
}

impl L1Watcher {
    pub async fn new_from_config(
        watcher_config: &L1WatcherConfig,
        eth_config: &EthConfig,
    ) -> Result<Self, L1WatcherError> {
        let eth_client = EthClient::new_with_multiple_urls(eth_config.rpc_url.clone())?;
        let l2_client = EthClient::new("http://localhost:1729")?;

        let last_block_fetched = U256::zero();
        Ok(Self {
            eth_client,
            l2_client,
            address: watcher_config.bridge_address,
            max_block_step: watcher_config.max_block_step,
            last_block_fetched,
            check_interval: watcher_config.check_interval_ms,
            l1_block_delay: watcher_config.watcher_block_delay,
        })
    }

    pub async fn run(&mut self, store: &Store, blockchain: &Blockchain) {
        loop {
            if let Err(err) = self.main_logic(store, blockchain).await {
                error!("L1 Watcher Error: {}", err);
            }
        }
    }

    async fn main_logic(
        &mut self,
        store: &Store,
        blockchain: &Blockchain,
    ) -> Result<(), L1WatcherError> {
        loop {
            sleep_random(self.check_interval).await;

            let logs = self.get_logs().await?;

            // We may not have a deposit nor a withdrawal, that means no events -> no logs.
            if logs.is_empty() {
                continue;
            }

            let _deposit_txs = self.process_logs(logs, store, blockchain).await?;
        }
    }

    pub async fn get_logs(&mut self) -> Result<Vec<RpcLog>, L1WatcherError> {
        if self.last_block_fetched.is_zero() {
            self.last_block_fetched = self
                .eth_client
                .get_last_fetched_l1_block(self.address)
                .await?
                .into();
        }

        let Some(latest_block_to_check) = self
            .eth_client
            .get_block_number()
            .await?
            .checked_sub(self.l1_block_delay.into())
        else {
            warn!("Too close to genesis to request deposits");
            return Ok(vec![]);
        };

        debug!(
            "Latest possible block number with {} blocks of delay: {latest_block_to_check} ({latest_block_to_check:#x})",
            self.l1_block_delay,
        );

        // last_block_fetched could be greater than latest_block_to_check:
        // - Right after deploying the contract as latest_block_fetched is set to the block where the contract is deployed
        // - If the node is stopped and l1_block_delay is changed
        if self.last_block_fetched > latest_block_to_check {
            warn!("Last block fetched is greater than latest safe block");
            return Ok(vec![]);
        }

        let new_last_block = min(
            self.last_block_fetched + self.max_block_step,
            latest_block_to_check,
        );

        debug!(
            "Looking logs from block {:#x} to {:#x}",
            self.last_block_fetched, new_last_block
        );

        // Matches the event DepositInitiated from ICommonBridge.sol
        let topic = keccak(
            b"DepositInitiated(uint256,address,uint256,address,address,uint256,bytes,bytes32)",
        );
        let logs = match self
            .eth_client
            .get_logs(
                self.last_block_fetched + 1,
                new_last_block,
                self.address,
                topic,
            )
            .await
        {
            Ok(logs) => logs,
            Err(error) => {
                // We may get an error if the RPC doesn't has the logs for the requested
                // block interval. For example, Light Nodes.
                warn!("Error when getting logs from L1: {}", error);
                vec![]
            }
        };

        debug!("Logs: {:#?}", logs);

        // If we have an error adding the tx to the mempool we may assign it to the next
        // block to fetch, but we may lose a deposit tx.
        self.last_block_fetched = new_last_block;

        Ok(logs)
    }

    pub async fn process_logs(
        &self,
        logs: Vec<RpcLog>,
        store: &Store,
        blockchain: &Blockchain,
    ) -> Result<Vec<H256>, L1WatcherError> {
        let mut deposit_txs = Vec::new();

        for log in logs {
            let deposit_data = DepositData::from_log(log.log)?;

            if self
                .deposit_already_processed(deposit_data.deposit_tx_hash, store)
                .await?
            {
                warn!(
                    "Deposit already processed (to: {:x}, value: {:x}, depositId: {:#}), skipping.",
                    deposit_data.recipient, deposit_data.mint_value, deposit_data.deposit_id
                );
                continue;
            }

            info!(
                "Initiating mint transaction for {:x} with value {:x} and depositId: {:#}",
                deposit_data.recipient, deposit_data.mint_value, deposit_data.deposit_id
            );

            let gas_price = self.l2_client.get_gas_price().await?;
            // Avoid panicking when using as_u64()
            let gas_price: u64 = gas_price
                .try_into()
                .map_err(|_| L1WatcherError::Custom("Failed at gas_price.try_into()".to_owned()))?;

            let mint_transaction = self
                .eth_client
                .build_privileged_transaction(
                    deposit_data.to_address,
                    deposit_data.recipient,
                    deposit_data.from,
                    Bytes::copy_from_slice(&deposit_data.calldata),
                    Overrides {
                        chain_id: Some(
                            store
                                .get_chain_config()
                                .map_err(|e| {
                                    L1WatcherError::FailedToRetrieveChainConfig(e.to_string())
                                })?
                                .chain_id,
                        ),
                        // Using the deposit_id as nonce.
                        // If we make a transaction on the L2 with this address, we may break the
                        // deposit workflow.
                        nonce: Some(deposit_data.deposit_id.as_u64()),
                        value: Some(deposit_data.mint_value),
                        gas_limit: Some(deposit_data.gas_limit.as_u64()),
                        // TODO(CHECK): Seems that when we start the L2, we need to set the gas.
                        // Otherwise, the transaction is not included in the mempool.
                        // We should override the blockchain to always include the transaction.
                        max_fee_per_gas: Some(gas_price),
                        max_priority_fee_per_gas: Some(gas_price),
                        ..Default::default()
                    },
                )
                .await?;

            match blockchain
                .add_transaction_to_pool(Transaction::PrivilegedL2Transaction(mint_transaction))
                .await
            {
                Ok(hash) => {
                    info!("Mint transaction added to mempool {hash:#x}",);
                    deposit_txs.push(hash);
                }
                Err(e) => {
                    warn!("Failed to add mint transaction to the mempool: {e:#?}");
                    // TODO: Figure out if we want to continue or not
                    continue;
                }
            }
        }

        Ok(deposit_txs)
    }

    async fn deposit_already_processed(
        &self,
        deposit_hash: H256,
        store: &Store,
    ) -> Result<bool, L1WatcherError> {
        if store
            .get_transaction_by_hash(deposit_hash)
            .await
            .map_err(L1WatcherError::FailedAccessingStore)?
            .is_some()
        {
            return Ok(true);
        }

        // If we have a reconstructed state, we don't have the transaction in our store.
        // Check if the deposit is marked as pending in the contract.
        let pending_deposits = self
            .eth_client
            .get_pending_deposit_logs(self.address)
            .await?;
        Ok(!pending_deposits.contains(&deposit_hash))
    }
}

struct DepositData {
    pub mint_value: U256,
    pub to_address: H160,
    pub deposit_id: U256,
    pub recipient: H160,
    pub from: H160,
    pub gas_limit: U256,
    pub calldata: Vec<u8>,
    pub deposit_tx_hash: H256,
}

impl DepositData {
    fn from_log(log: RpcLogInfo) -> Result<DepositData, L1WatcherError> {
        let mint_value = format!(
            "{:#x}",
            log.topics
                .get(1)
                .ok_or(L1WatcherError::FailedToDeserializeLog(
                    "Failed to parse mint value from log: log.topics[1] out of bounds".to_owned()
                ))?
        )
        .parse::<U256>()
        .map_err(|e| {
            L1WatcherError::FailedToDeserializeLog(format!(
                "Failed to parse mint value from log: {e:#?}"
            ))
        })?;
        let to_address_hash = log
            .topics
            .get(2)
            .ok_or(L1WatcherError::FailedToDeserializeLog(
                "Failed to parse beneficiary from log: log.topics[2] out of bounds".to_owned(),
            ))?;
        let to_address = hash_to_address(*to_address_hash);

        let deposit_id = log
            .topics
            .get(3)
            .ok_or(L1WatcherError::FailedToDeserializeLog(
                "Failed to parse beneficiary from log: log.topics[3] out of bounds".to_owned(),
            ))?;

        let deposit_id = format!("{deposit_id:#x}").parse::<U256>().map_err(|e| {
            L1WatcherError::FailedToDeserializeLog(format!(
                "Failed to parse depositId value from log: {e:#?}"
            ))
        })?;

        // The previous values are indexed in the topic of the log. Data contains the rest.
        // DATA = recipient: Address || from: Address || gas_limit: uint256 || offset_calldata: uint256 || tx_hash: H256 || length_calldata: uint256 || calldata: bytes
        // DATA = 0..32              || 32..64        || 64..96             || 96..128                  || 128..160      || 160..192                 || 192..(192+calldata_len)
        // Any value that is not 32 bytes is padded with zeros.

        let recipient = log
            .data
            .get(12..32)
            .ok_or(L1WatcherError::FailedToDeserializeLog(
                "Failed to parse recipient from log: log.data[0..32] out of bounds".to_owned(),
            ))?;
        let recipient = Address::from_slice(recipient);

        let from = log
            .data
            .get(44..64)
            .ok_or(L1WatcherError::FailedToDeserializeLog(
                "Failed to parse from from log: log.data[44..64] out of bounds".to_owned(),
            ))?;
        let from = Address::from_slice(from);

        let gas_limit = U256::from_big_endian(log.data.get(64..96).ok_or(
            L1WatcherError::FailedToDeserializeLog(
                "Failed to parse gas_limit from log: log.data[64..96] out of bounds".to_owned(),
            ),
        )?);

        let deposit_tx_hash = H256::from_slice(
            log.data
                .get(128..160)
                .ok_or(L1WatcherError::FailedToDeserializeLog(
                    "Failed to parse deposit_tx_hash from log: log.data[64..96] out of bounds"
                        .to_owned(),
                ))?,
        );

        let calldata_len = U256::from_big_endian(log.data.get(160..192).ok_or(
            L1WatcherError::FailedToDeserializeLog(
                "Failed to parse calldata_len from log: log.data[96..128] out of bounds".to_owned(),
            ),
        )?);
        let calldata = log
            .data
            .get(192..192 + calldata_len.as_usize())
            .ok_or(L1WatcherError::FailedToDeserializeLog(
            "Failed to parse calldata from log: log.data[128..128 + calldata_len] out of bounds"
                .to_owned(),
        ))?;

        Ok(Self {
            mint_value,
            to_address,
            deposit_id,
            recipient,
            from,
            gas_limit,
            calldata: calldata.to_vec(),
            deposit_tx_hash,
        })
    }
}
