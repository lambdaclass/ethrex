//! NativeL1Watcher GenServer — polls L1 for `L1MessageRecorded` events
//! from the NativeRollup.sol contract and pushes them into the shared
//! `PendingL1Messages` queue.

use std::time::Duration;

use ethrex_common::utils::keccak;
use ethrex_common::{Address, H256, U256};
use ethrex_rpc::clients::eth::EthClient;
use ethrex_rpc::types::receipt::RpcLog;
use spawned_concurrency::tasks::{
    CastResponse, GenServer, GenServerHandle, InitResult, Success, send_after,
};
use tracing::{debug, error, info, warn};

use super::types::{L1Message, PendingL1Messages};

/// Event topic0: keccak256("L1MessageRecorded(address,address,uint256,uint256,bytes32,uint256)")
fn l1_message_recorded_topic() -> H256 {
    keccak(b"L1MessageRecorded(address,address,uint256,uint256,bytes32,uint256)")
}

#[derive(Clone)]
pub enum CastMsg {
    Poll,
}

pub struct NativeL1Watcher {
    eth_client: EthClient,
    contract_address: Address,
    pending_messages: PendingL1Messages,
    last_block_fetched: U256,
    check_interval_ms: u64,
    max_block_step: U256,
}

#[derive(Debug, thiserror::Error)]
pub enum NativeL1WatcherError {
    #[error("EthClient error: {0}")]
    EthClient(#[from] ethrex_rpc::clients::eth::errors::EthClientError),
    #[error("Internal error: {0}")]
    Internal(#[from] spawned_concurrency::error::GenServerError),
    #[error("Parse error: {0}")]
    Parse(String),
}

impl NativeL1Watcher {
    pub fn new(
        eth_client: EthClient,
        contract_address: Address,
        pending_messages: PendingL1Messages,
        check_interval_ms: u64,
        max_block_step: u64,
    ) -> Self {
        Self {
            eth_client,
            contract_address,
            pending_messages,
            last_block_fetched: U256::zero(),
            check_interval_ms,
            max_block_step: U256::from(max_block_step),
        }
    }

    async fn poll_l1_messages(&mut self) {
        let topic = l1_message_recorded_topic();

        let latest_block = match self.eth_client.get_block_number().await {
            Ok(n) => n,
            Err(e) => {
                error!("NativeL1Watcher: failed to get block number: {e}");
                return;
            }
        };

        // Don't go past the latest block
        if self.last_block_fetched >= latest_block {
            debug!("NativeL1Watcher: no new blocks to scan");
            return;
        }

        let from_block = self.last_block_fetched + 1;
        let to_block = std::cmp::min(self.last_block_fetched + self.max_block_step, latest_block);

        debug!(
            "NativeL1Watcher: scanning blocks {:#x} to {:#x}",
            from_block, to_block
        );

        let logs = match self
            .eth_client
            .get_logs(from_block, to_block, self.contract_address, vec![topic])
            .await
        {
            Ok(logs) => logs,
            Err(e) => {
                error!("NativeL1Watcher: failed to get logs: {e}");
                return;
            }
        };

        if !logs.is_empty() {
            info!(
                "NativeL1Watcher: found {} L1MessageRecorded events",
                logs.len()
            );
        }

        let mut parsed = Vec::new();
        for log in logs {
            match Self::parse_l1_message_recorded(&log) {
                Ok(msg) => parsed.push(msg),
                Err(e) => {
                    warn!("NativeL1Watcher: failed to parse log: {e}");
                    continue;
                }
            }
        }

        if !parsed.is_empty() {
            match self.pending_messages.lock() {
                Ok(mut queue) => {
                    for msg in &parsed {
                        info!(
                            "NativeL1Watcher: queued L1 message nonce={} sender={:?} to={:?} value={}",
                            msg.nonce, msg.sender, msg.to, msg.value
                        );
                    }
                    queue.extend(parsed);
                }
                Err(e) => {
                    error!("NativeL1Watcher: failed to lock pending_messages: {e}");
                }
            }
        }

        self.last_block_fetched = to_block;
    }

    /// Parse an `L1MessageRecorded` event log.
    ///
    /// Event signature:
    /// ```text
    /// L1MessageRecorded(
    ///     address indexed sender,   // topic[1]
    ///     address indexed to,       // topic[2]
    ///     uint256 value,            // data[0..32]
    ///     uint256 gasLimit,         // data[32..64]
    ///     bytes32 dataHash,         // data[64..96]
    ///     uint256 indexed nonce     // topic[3]
    /// )
    /// ```
    fn parse_l1_message_recorded(log: &RpcLog) -> Result<L1Message, NativeL1WatcherError> {
        let topics = &log.log.topics;
        let data = &log.log.data;

        if topics.len() < 4 {
            return Err(NativeL1WatcherError::Parse(format!(
                "Expected 4 topics, got {}",
                topics.len()
            )));
        }
        if data.len() < 96 {
            return Err(NativeL1WatcherError::Parse(format!(
                "Expected at least 96 bytes of data, got {}",
                data.len()
            )));
        }

        let parse_err = |msg: &str| NativeL1WatcherError::Parse(msg.to_string());

        // topic[1] = sender (address, left-padded to 32 bytes)
        let sender_topic = topics.get(1).ok_or(parse_err("missing topic[1]"))?;
        let sender_bytes = sender_topic
            .as_bytes()
            .get(12..)
            .ok_or(parse_err("topic[1] too short"))?;
        let sender = Address::from_slice(sender_bytes);

        // topic[2] = to
        let to_topic = topics.get(2).ok_or(parse_err("missing topic[2]"))?;
        let to_bytes = to_topic
            .as_bytes()
            .get(12..)
            .ok_or(parse_err("topic[2] too short"))?;
        let to = Address::from_slice(to_bytes);

        // topic[3] = nonce
        let nonce_topic = topics.get(3).ok_or(parse_err("missing topic[3]"))?;
        let nonce = U256::from_big_endian(nonce_topic.as_bytes());

        // data[0..32] = value
        let value_bytes = data
            .get(..32)
            .ok_or(parse_err("data too short for value"))?;
        let value = U256::from_big_endian(value_bytes);

        // data[32..64] = gasLimit
        let gas_limit_bytes = data
            .get(32..64)
            .ok_or(parse_err("data too short for gasLimit"))?;
        let gas_limit_u256 = U256::from_big_endian(gas_limit_bytes);
        let gas_limit: u64 = gas_limit_u256
            .try_into()
            .map_err(|_| NativeL1WatcherError::Parse("gasLimit exceeds u64".into()))?;

        // data[64..96] = dataHash
        let data_hash_bytes = data
            .get(64..96)
            .ok_or(parse_err("data too short for dataHash"))?;
        let data_hash = H256::from_slice(data_hash_bytes);

        Ok(L1Message {
            sender,
            to,
            nonce,
            value,
            gas_limit,
            data_hash,
        })
    }
}

impl GenServer for NativeL1Watcher {
    type CallMsg = ();
    type CastMsg = CastMsg;
    type OutMsg = ();
    type Error = NativeL1WatcherError;

    async fn init(self, handle: &GenServerHandle<Self>) -> Result<InitResult<Self>, Self::Error> {
        // Schedule the first poll immediately
        handle
            .clone()
            .cast(CastMsg::Poll)
            .await
            .map_err(NativeL1WatcherError::Internal)?;
        Ok(Success(self))
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &GenServerHandle<Self>,
    ) -> CastResponse {
        match message {
            CastMsg::Poll => {
                self.poll_l1_messages().await;
                send_after(
                    Duration::from_millis(self.check_interval_ms),
                    handle.clone(),
                    CastMsg::Poll,
                );
                CastResponse::NoReply
            }
        }
    }

    async fn handle_call(
        &mut self,
        _message: Self::CallMsg,
        _handle: &GenServerHandle<Self>,
    ) -> spawned_concurrency::tasks::CallResponse<Self> {
        spawned_concurrency::tasks::CallResponse::Reply(())
    }
}
