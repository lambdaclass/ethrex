use std::fmt::Display;

use ethrex_common::{Address, H256, U256};
use ethrex_l2_sdk::COMMON_BRIDGE_L2_ADDRESS;
use ethrex_rpc::{EthClient, types::receipt::RpcLog};
use ratatui::widgets::TableState;

use crate::{DepositData, monitor};

pub struct L1ToL2MessagesTable {
    pub state: TableState,
    // Status | Kind | L1 tx hash | L2 tx hash | amount
    pub items: Vec<(L1ToL2MessageStatus, L1ToL2MessageKind, H256, H256, U256)>,
    last_l1_block_fetched: U256,
    common_bridge_address: Address,
}

#[derive(Debug, Clone)]
pub enum L1ToL2MessageStatus {
    Pending,
    Processed,
}

impl Display for L1ToL2MessageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            L1ToL2MessageStatus::Pending => write!(f, "Pending"),
            L1ToL2MessageStatus::Processed => write!(f, "Processed"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum L1ToL2MessageKind {
    Deposit,
    Message,
}

impl From<&DepositData> for L1ToL2MessageKind {
    fn from(data: &DepositData) -> Self {
        if data.from == COMMON_BRIDGE_L2_ADDRESS && data.to_address == COMMON_BRIDGE_L2_ADDRESS {
            Self::Deposit
        } else {
            Self::Message
        }
    }
}

impl Display for L1ToL2MessageKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            L1ToL2MessageKind::Deposit => write!(f, "Deposit"),
            L1ToL2MessageKind::Message => write!(f, "Message"),
        }
    }
}

impl L1ToL2MessagesTable {
    pub async fn new(common_bridge_address: Address, eth_client: &EthClient) -> Self {
        let mut last_l1_block_fetched = eth_client
            .get_last_fetched_l1_block(common_bridge_address)
            .await
            .expect("Failed to get last fetched L1 block")
            .into();
        let items = Self::refresh_items(
            &mut last_l1_block_fetched,
            common_bridge_address,
            eth_client,
        )
        .await;
        Self {
            state: TableState::default(),
            items,
            last_l1_block_fetched,
            common_bridge_address,
        }
    }

    pub async fn on_tick(&mut self, eth_client: &EthClient) {
        let mut new_l1_to_l2_messages = Self::refresh_items(
            &mut self.last_l1_block_fetched,
            self.common_bridge_address,
            eth_client,
        )
        .await;

        let n_new_latest_batches = new_l1_to_l2_messages.len();

        if n_new_latest_batches > 50 {
            new_l1_to_l2_messages.truncate(50);
            self.items.extend_from_slice(&new_l1_to_l2_messages);
        } else {
            self.items.truncate(50 - n_new_latest_batches);
            self.items.extend_from_slice(&new_l1_to_l2_messages);
            self.items.rotate_right(n_new_latest_batches);
        }
    }

    async fn refresh_items(
        last_l1_block_fetched: &mut U256,
        common_bridge_address: Address,
        eth_client: &EthClient,
    ) -> Vec<(L1ToL2MessageStatus, L1ToL2MessageKind, H256, H256, U256)> {
        let logs = Self::get_logs(last_l1_block_fetched, common_bridge_address, eth_client).await;
        Self::process_logs(&logs, common_bridge_address, eth_client).await
    }

    async fn get_logs(
        last_l1_block_fetched: &mut U256,
        common_bridge_address: Address,
        eth_client: &EthClient,
    ) -> Vec<RpcLog> {
        monitor::utils::get_logs(
            last_l1_block_fetched,
            common_bridge_address,
            "L1ToL2Message(uint256,address,uint256,address,uint256,bytes,bytes32)",
            eth_client,
        )
        .await
    }

    async fn process_logs(
        logs: &[RpcLog],
        common_bridge_address: Address,
        eth_client: &EthClient,
    ) -> Vec<(L1ToL2MessageStatus, L1ToL2MessageKind, H256, H256, U256)> {
        let mut processed_logs = Vec::new();

        let pending_l1_to_l2_messages = eth_client
            .get_pending_deposit_logs(common_bridge_address)
            .await
            .expect("Failed to get pending L1 to L2 messages");

        for log in logs {
            let l1_to_l2_message =
                DepositData::from_log(log.log.clone()).expect("Failed to parse L1ToL2Message log");

            processed_logs.push((
                if pending_l1_to_l2_messages.contains(&log.transaction_hash) {
                    L1ToL2MessageStatus::Pending
                } else {
                    L1ToL2MessageStatus::Processed
                },
                L1ToL2MessageKind::from(&l1_to_l2_message),
                log.transaction_hash,
                l1_to_l2_message.deposit_tx_hash,
                l1_to_l2_message.value,
            ));
        }

        processed_logs
    }
}
