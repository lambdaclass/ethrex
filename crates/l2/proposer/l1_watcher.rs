use crate::{
    proposer::errors::L1WatcherError,
    utils::{
        config::{eth::EthConfig, l1_watcher::L1WatcherConfig},
        eth_client::{errors::EthClientError, eth_sender::Overrides, EthClient},
    },
};
use bytes::Bytes;
use ethereum_types::{Address, BigEndianHash, H256, U256};
use ethrex_blockchain::{constants::TX_GAS_COST, mempool};
use ethrex_core::types::PrivilegedTxType;
use ethrex_core::types::{Signable, Transaction};
use ethrex_rpc::types::receipt::RpcLog;
use ethrex_storage::Store;
use keccak_hash::keccak;
use secp256k1::SecretKey;
use std::{cmp::min, ops::Mul, time::Duration};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

pub async fn start_l1_watcher(store: Store) {
    let eth_config = EthConfig::from_env().expect("EthConfig::from_env()");
    let watcher_config = L1WatcherConfig::from_env().expect("L1WatcherConfig::from_env()");
    let mut l1_watcher = L1Watcher::new_from_config(watcher_config, eth_config)
        .await
        .expect("new");
    l1_watcher.run(&store).await;
}

pub struct L1Watcher {
    eth_client: EthClient,
    address: Address,
    topics: Vec<H256>,
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
        let eth_client = EthClient::new_from_config(eth_config);
        let last_block_fetched =
            EthClient::get_last_fetched_l1_block(&eth_client, watcher_config.bridge_address)
                .await?
                .into();
        Ok(Self {
            eth_client,
            address: watcher_config.bridge_address,
            topics: watcher_config.topics,
            max_block_step: watcher_config.max_block_step,
            last_block_fetched,
            l2_proposer_pk: watcher_config.l2_proposer_private_key,
            check_interval: Duration::from_millis(watcher_config.check_interval_ms),
        })
    }

    pub async fn run(&mut self, store: &Store) {
        loop {
            if let Err(err) = self.main_logic(store.clone()).await {
                error!("L1 Watcher Error: {}", err);
            }

            sleep(self.check_interval).await;
        }
    }

    async fn main_logic(&mut self, store: Store) -> Result<(), L1WatcherError> {
        loop {
            sleep(self.check_interval).await;

            let (logs, new_block_to_fetch) = self.get_logs().await?;

            // We may not have a deposit nor a withdrawal, that means no events -> no logs.
            if logs.is_empty() {
                continue;
            }

            let pending_deposits_logs = self.get_pending_deposit_logs().await?;
            let _deposit_txs = self
                .process_logs(logs, &pending_deposits_logs, new_block_to_fetch, &store)
                .await?;
        }
    }

    pub async fn get_pending_deposit_logs(&self) -> Result<Vec<H256>, L1WatcherError> {
        Ok(hex::decode(
            &self
                .eth_client
                .call(
                    self.address,
                    Bytes::copy_from_slice(&[0x35, 0x6d, 0xa2, 0x49]),
                    Overrides::default(),
                )
                .await?[2..],
        )
        .map_err(|_| L1WatcherError::FailedToDeserializeLog("Not a valid hex string".to_string()))?
        .chunks(32)
        .map(H256::from_slice)
        .collect::<Vec<H256>>()
        .split_at(2) // Two first words are index and length abi encode
        .1
        .to_vec())
    }

    pub async fn get_logs(&self) -> Result<(Vec<RpcLog>, U256), L1WatcherError> {
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

        // We may get an error if the RPC doesn't has the logs for the requested
        // block interval. For example, Light Nodes.
        let logs = match self
            .eth_client
            .get_logs(
                self.last_block_fetched + 1,
                new_last_block,
                self.address,
                self.topics[0],
            )
            .await
        {
            Ok(logs) => logs,
            Err(error) => {
                warn!("Error when getting logs from L1: {}", error);
                vec![]
            }
        };

        debug!("Logs: {:#?}", logs);

        Ok((logs, new_last_block))
    }

    pub async fn process_logs(
        &mut self,
        logs: Vec<RpcLog>,
        l1_deposit_logs: &[H256],
        new_block_to_fetch: U256,
        store: &Store,
    ) -> Result<Vec<H256>, L1WatcherError> {
        let mut deposit_txs = Vec::new();
        let mut operator_nonce = store
            .get_account_info(
                store
                    .get_latest_block_number()
                    .map_err(|e| L1WatcherError::FailedToRetrieveChainConfig(e.to_string()))?
                    .ok_or(L1WatcherError::FailedToRetrieveChainConfig(
                        "Last block is None".to_string(),
                    ))?,
                Address::zero(),
            )
            .map_err(|e| L1WatcherError::FailedToRetrieveDepositorAccountInfo(e.to_string()))?
            .map(|info| info.nonce)
            .unwrap_or_default();

        for log in logs {
            let mint_value = format!("{:#x}", log.log.topics[1])
                .parse::<U256>()
                .map_err(|e| {
                    L1WatcherError::FailedToDeserializeLog(format!(
                        "Failed to parse mint value from log: {e:#?}"
                    ))
                })?;
            let beneficiary = format!("{:#x}", log.log.topics[2].into_uint())
                .parse::<Address>()
                .map_err(|e| {
                    L1WatcherError::FailedToDeserializeLog(format!(
                        "Failed to parse beneficiary from log: {e:#?}"
                    ))
                })?;

            let mut value_bytes = [0u8; 32];
            mint_value.to_big_endian(&mut value_bytes);
            if !l1_deposit_logs.contains(&keccak([beneficiary.as_bytes(), &value_bytes].concat())) {
                warn!("Deposit already processed (to: {beneficiary:#x}, value: {mint_value}), skipping.");
                continue;
            }

            info!("Initiating mint transaction for {beneficiary:#x} with value {mint_value:#x}",);

            let mut mint_transaction = self
                .eth_client
                .build_privileged_transaction(
                    PrivilegedTxType::Deposit,
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
                        nonce: Some(operator_nonce),
                        value: Some(mint_value),
                        // TODO(IMPORTANT): gas_limit should come in the log and must
                        // not be calculated in here. The reason for this is that the
                        // gas_limit for this transaction is payed by the caller in
                        // the L1 as part of the deposited funds.
                        gas_limit: Some(TX_GAS_COST.mul(2)),
                        ..Default::default()
                    },
                    10,
                )
                .await?;
            mint_transaction.sign_inplace(&self.l2_proposer_pk);

            operator_nonce += 1;

            match mempool::add_transaction(
                Transaction::PrivilegedL2Transaction(mint_transaction),
                store.clone(),
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

        // If we have an error adding the tx to the mempool we may assign it to the next
        // block to fetch, but we may lose a deposit tx.
        self.last_block_fetched = new_block_to_fetch;
        Ok(deposit_txs)
    }
}
