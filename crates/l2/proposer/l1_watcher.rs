use crate::{
    proposer::errors::L1WatcherError,
    utils::{
        config::{eth::EthConfig, l1_watcher::L1WatcherConfig},
        eth_client::{transaction::PayloadRLPEncode, EthClient},
    },
};
use ethereum_rust_blockchain::{constants::TX_GAS_COST, mempool};
use ethereum_rust_core::types::{
    PrivilegedL2Transaction, PrivilegedTxType, Transaction, TxKind, TxType,
};
use ethereum_rust_rlp::encode::RLPEncode;
use ethereum_rust_rpc::types::receipt::RpcLog;
use ethereum_rust_storage::Store;
use ethereum_types::{Address, BigEndianHash, H256, U256};
use keccak_hash::keccak;
use libsecp256k1::{sign, Message, SecretKey};
use std::{cmp::min, ops::Mul, time::Duration};
use tokio::time::sleep;
use tracing::{debug, info, warn};

pub async fn start_l1_watcher(store: Store) {
    let eth_config = EthConfig::from_env().expect("EthConfig::from_env()");
    let watcher_config = L1WatcherConfig::from_env().expect("L1WatcherConfig::from_env()");
    let mut l1_watcher = L1Watcher::new_from_config(watcher_config, eth_config);
    loop {
        let logs = l1_watcher.get_logs().await.expect("l1_watcher.get_logs()");
        let _deposit_txs = l1_watcher
            .process_logs(logs, &store)
            .await
            .expect("l1_watcher.process_logs()");
    }
}

pub struct L1Watcher {
    eth_client: EthClient,
    address: Address,
    topics: Vec<H256>,
    check_interval: Duration,
    max_block_step: U256,
    last_block_fetched: U256,
    l2_proposer_pk: SecretKey,
    l2_proposer_address: Address,
}

impl L1Watcher {
    pub fn new_from_config(watcher_config: L1WatcherConfig, eth_config: EthConfig) -> Self {
        Self {
            eth_client: EthClient::new_from_config(eth_config),
            address: watcher_config.bridge_address,
            topics: watcher_config.topics,
            check_interval: Duration::from_millis(watcher_config.check_interval_ms),
            max_block_step: watcher_config.max_block_step,
            last_block_fetched: U256::zero(),
            l2_proposer_pk: watcher_config.l2_proposer_private_key,
            l2_proposer_address: watcher_config.l2_proposer_address,
        }
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

        let logs = self
            .eth_client
            .get_logs(
                self.last_block_fetched + 1,
                new_last_block,
                self.address,
                self.topics[0],
            )
            .await?;

        debug!("Logs: {:#?}", logs);

        self.last_block_fetched = new_last_block;

        sleep(self.check_interval).await;

        Ok(logs)
    }

    pub async fn process_logs(
        &self,
        logs: Vec<RpcLog>,
        store: &Store,
    ) -> Result<Vec<H256>, L1WatcherError> {
        if logs.is_empty() {
            return Ok(Vec::new());
        }

        let mut deposit_txs = Vec::new();
        let mut operator_nonce = store
            .get_account_info(
                self.eth_client.get_block_number().await?.as_u64(),
                self.l2_proposer_address,
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

            info!("Initiating mint transaction for {beneficiary:#x} with value {mint_value:#x}",);

            let mut mint_transaction = PrivilegedL2Transaction {
                tx_type: PrivilegedTxType::Deposit,
                to: TxKind::Call(beneficiary),
                chain_id: store
                    .get_chain_config()
                    .map_err(|e| L1WatcherError::FailedToRetrieveChainConfig(e.to_string()))?
                    .chain_id,
                ..Default::default()
            };

            mint_transaction.nonce = operator_nonce;
            operator_nonce += 1;

            mint_transaction.max_fee_per_gas = self.eth_client.get_gas_price().await?.as_u64();
            // TODO(IMPORTANT): gas_limit should come in the log and must
            // not be calculated in here. The reason for this is that the
            // gas_limit for this transaction is payed by the caller in
            // the L1 as part of the deposited funds.
            mint_transaction.gas_limit = TX_GAS_COST.mul(2);
            mint_transaction.value = mint_value;

            let mut payload = vec![TxType::Privileged as u8];
            payload.append(mint_transaction.encode_payload_to_vec().as_mut());

            let data = Message::parse(&keccak(payload).0);
            let signature = sign(&data, &self.l2_proposer_pk);

            mint_transaction.signature_r = U256::from(signature.0.r.b32());
            mint_transaction.signature_s = U256::from(signature.0.s.b32());
            mint_transaction.signature_y_parity = signature.1.serialize() != 0;

            let mut encoded_tx = Vec::new();
            mint_transaction.encode(&mut encoded_tx);

            let mut data = vec![TxType::Privileged as u8];
            data.append(&mut encoded_tx);

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
        Ok(deposit_txs)
    }
}
