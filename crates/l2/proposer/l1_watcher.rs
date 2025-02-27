use crate::{
    proposer::errors::L1WatcherError,
    utils::config::{errors::ConfigError, eth::EthConfig, l1_watcher::L1WatcherConfig},
};
use bytes::Bytes;
use ethereum_types::{Address, BigEndianHash, H256, U256};
use ethrex_blockchain::{constants::TX_GAS_COST, mempool};
use ethrex_common::types::{Signable, Transaction};
use ethrex_rpc::clients::eth::{errors::EthClientError, eth_sender::Overrides, EthClient};
use ethrex_rpc::types::receipt::RpcLog;
use ethrex_storage::Store;
use keccak_hash::keccak;
use secp256k1::SecretKey;
use std::{cmp::min, ops::Mul, time::Duration};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

pub async fn start_l1_watcher(store: Store) -> Result<(), ConfigError> {
    dbg!("Starting L1 Watcher");
    let eth_config = EthConfig::from_env()?;
    let watcher_config = L1WatcherConfig::from_env()?;
    dbg!(watcher_config.bridge_address);
    let mut l1_watcher = L1Watcher::new_from_config(watcher_config, eth_config).await?;
    l1_watcher.run(&store).await;
    Ok(())
}

pub struct L1Watcher {
    eth_client: EthClient,
    l2_client: EthClient,
    address: Address,
    max_block_step: U256,
    last_block_fetched: U256,
    l2_proposer_pk: SecretKey,
    check_interval: Duration,
}

impl L1Watcher {
    pub async fn new_from_config(
        watcher_config: L1WatcherConfig,
        eth_config: EthConfig,
    ) -> Result<Self, EthClientError> {
        let eth_client = EthClient::new(&eth_config.rpc_url);
        let l2_client = EthClient::new("http://localhost:1729");
        let last_block_fetched: U256 =
            EthClient::get_last_fetched_l1_block(&eth_client, watcher_config.bridge_address)
                .await?
                .into();
        Ok(Self {
            eth_client,
            l2_client,
            address: watcher_config.bridge_address,
            max_block_step: watcher_config.max_block_step,
            last_block_fetched: U256::zero(), // last_block_fetched,
            l2_proposer_pk: watcher_config.l2_proposer_private_key,
            check_interval: Duration::from_millis(watcher_config.check_interval_ms),
        })
    }

    pub async fn run(&mut self, store: &Store) {
        loop {
            if let Err(err) = self.main_logic(store).await {
                error!("L1 Watcher Error: {}", err);
            }

            sleep(self.check_interval).await;
        }
    }

    async fn main_logic(&mut self, store: &Store) -> Result<(), L1WatcherError> {
        loop {
            sleep(self.check_interval).await;

            dbg!("Loop L1 Watcher", &self.address);
            let logs = self.get_logs().await?;

            // We may not have a deposit nor a withdrawal, that means no events -> no logs.
            if logs.is_empty() {
                continue;
            }

            dbg!("HAY LOGS", &logs);
            let pending_deposit_logs = self.get_pending_deposit_logs().await?;
            let _deposit_txs = self
                .process_logs(logs, &pending_deposit_logs, store)
                .await?;
        }
    }

    pub async fn get_pending_deposit_logs(&self) -> Result<Vec<H256>, L1WatcherError> {
        let selector = keccak(b"getDepositLogs()")
            .as_bytes()
            .get(..4)
            .ok_or(EthClientError::Custom("Failed to get selector.".to_owned()))?
            .to_vec();

        Ok(hex::decode(
            self.eth_client
                .call(
                    self.address,
                    Bytes::copy_from_slice(&selector),
                    Overrides::default(),
                )
                .await?
                .get(2..)
                .ok_or(L1WatcherError::FailedToDeserializeLog(
                    "Not a valid hex string".to_string(),
                ))?,
        )
        .map_err(|_| L1WatcherError::FailedToDeserializeLog("Not a valid hex string".to_string()))?
        .chunks(32)
        .map(H256::from_slice)
        .collect::<Vec<H256>>()
        .split_at(2) // Two first words are index and length abi encode
        .1
        .to_vec())
    }

    pub async fn get_logs(&mut self) -> Result<Vec<RpcLog>, L1WatcherError> {
        let current_block = self.eth_client.get_block_number().await?;

        debug!(
            "Current block number: {} ({:#x})",
            current_block, current_block
        );

        let new_last_block = min(self.last_block_fetched + self.max_block_step, current_block);

        debug!(
            "Looking logs from block {:#x} to {:#x}",
            self.last_block_fetched, new_last_block
        );

        // Matches the event DepositInitiated from ICommonBridge.sol
        let topic = keccak(b"DepositInitiated(uint256,address,uint256,bytes32)");
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
        pending_deposit_logs: &[H256],
        store: &Store,
    ) -> Result<Vec<H256>, L1WatcherError> {
        let mut deposit_txs = Vec::new();

        for log in logs {
            let mint_value = format!(
                "{:#x}",
                log.log
                    .topics
                    .get(1)
                    .ok_or(L1WatcherError::FailedToDeserializeLog(
                        "Failed to parse mint value from log: log.topics[1] out of bounds"
                            .to_owned()
                    ))?
            )
            .parse::<U256>()
            .map_err(|e| {
                L1WatcherError::FailedToDeserializeLog(format!(
                    "Failed to parse mint value from log: {e:#?}"
                ))
            })?;
            let beneficiary_uint = log
                .log
                .topics
                .get(2)
                .ok_or(L1WatcherError::FailedToDeserializeLog(
                    "Failed to parse beneficiary from log: log.topics[2] out of bounds".to_owned(),
                ))?
                .into_uint();
            let beneficiary = format!("{beneficiary_uint:#x}")
                .parse::<Address>()
                .map_err(|e| {
                    L1WatcherError::FailedToDeserializeLog(format!(
                        "Failed to parse beneficiary from log: {e:#?}"
                    ))
                })?;

            let deposit_id =
                log.log
                    .topics
                    .get(3)
                    .ok_or(L1WatcherError::FailedToDeserializeLog(
                        "Failed to parse beneficiary from log: log.topics[3] out of bounds"
                            .to_owned(),
                    ))?;

            let deposit_id = format!("{deposit_id:#x}").parse::<U256>().map_err(|e| {
                L1WatcherError::FailedToDeserializeLog(format!(
                    "Failed to parse depositId value from log: {e:#?}"
                ))
            })?;

            let value_bytes = mint_value.to_big_endian();
            let id_bytes = deposit_id.to_big_endian();
            if !pending_deposit_logs.contains(&keccak(
                [beneficiary.as_bytes(), &value_bytes, &id_bytes].concat(),
            )) {
                warn!("Deposit already processed (to: {beneficiary:#x}, value: {mint_value}, depositId: {deposit_id}), skipping.");
                continue;
            }

            info!("Initiating mint transaction for {beneficiary:#x} with value {mint_value:#x} and depositId: {deposit_id:#}",);

            let gas_price = self.l2_client.get_gas_price().await?;
            // Avoid panicking when using as_u64()
            let gas_price: u64 = gas_price
                .try_into()
                .map_err(|_| L1WatcherError::Custom("Failed at gas_price.try_into()".to_owned()))?;

            let mut mint_transaction = self
                .eth_client
                .build_privileged_transaction(
                    beneficiary,
                    beneficiary,
                    Bytes::new(),
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
                        nonce: Some(deposit_id.as_u64()),
                        value: Some(mint_value),
                        // TODO(IMPORTANT): gas_limit should come in the log and must
                        // not be calculated in here. The reason for this is that the
                        // gas_limit for this transaction is payed by the caller in
                        // the L1 as part of the deposited funds.
                        gas_limit: Some(TX_GAS_COST.mul(2)),
                        // TODO(CHECK): Seems that when we start the L2, we need to set the gas.
                        // Otherwise, the transaction is not included in the mempool.
                        // We should override the blockchain to always include the transaction.
                        max_fee_per_gas: Some(gas_price),
                        max_priority_fee_per_gas: Some(gas_price),
                        ..Default::default()
                    },
                    10,
                )
                .await?;
            mint_transaction.sign_inplace(&self.l2_proposer_pk);

            match mempool::add_transaction(
                Transaction::PrivilegedL2Transaction(mint_transaction),
                store,
            ) {
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
}
