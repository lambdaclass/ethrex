use std::fmt::Display;

use ethrex_common::{Address, H256, U256};
use ethrex_l2_sdk::COMMON_BRIDGE_L2_ADDRESS;
use ethrex_rpc::{EthClient, types::receipt::RpcLog};
use keccak_hash::keccak;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Row, StatefulWidget, Table, TableState},
};

use crate::{
    monitor::{self, widget::HASH_LENGTH_IN_DIGITS},
    sequencer::l1_watcher::PrivilegedTransactionData,
};

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

impl From<&PrivilegedTransactionData> for L1ToL2MessageKind {
    fn from(data: &PrivilegedTransactionData) -> Self {
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
            vec!["PrivilegedTxSent(address,address,uint256,uint256,uint256,bytes)"],
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
            .get_pending_privileged_transactions(common_bridge_address)
            .await
            .expect("Failed to get pending L1 to L2 messages");

        for log in logs {
            let l1_to_l2_message = PrivilegedTransactionData::from_log(log.log.clone())
                .expect("Failed to parse PrivilegedTxSent log");

            let l1_to_l2_message_hash = keccak(
                [
                    l1_to_l2_message.from.as_bytes(),
                    l1_to_l2_message.to_address.as_bytes(),
                    &l1_to_l2_message.transaction_id.to_big_endian(),
                    &l1_to_l2_message.value.to_big_endian(),
                    &l1_to_l2_message.gas_limit.to_big_endian(),
                    keccak(&l1_to_l2_message.calldata).as_bytes(),
                ]
                .concat(),
            );

            processed_logs.push((
                if pending_l1_to_l2_messages.contains(&log.transaction_hash) {
                    L1ToL2MessageStatus::Pending
                } else {
                    L1ToL2MessageStatus::Processed
                },
                L1ToL2MessageKind::from(&l1_to_l2_message),
                log.transaction_hash,
                l1_to_l2_message_hash,
                l1_to_l2_message.value,
            ));
        }

        processed_logs
    }
}

impl StatefulWidget for &mut L1ToL2MessagesTable {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State)
    where
        Self: Sized,
    {
        let constraints = vec![
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Length(HASH_LENGTH_IN_DIGITS),
            Constraint::Length(HASH_LENGTH_IN_DIGITS),
            Constraint::Fill(1),
        ];

        let rows = self
            .items
            .iter()
            .map(|(status, kind, l1_tx_hash, l2_tx_hash, amount)| {
                Row::new(vec![
                    Span::styled(format!("{status}"), Style::default()),
                    Span::styled(format!("{kind}"), Style::default()),
                    Span::styled(format!("{l1_tx_hash:#x}"), Style::default()),
                    Span::styled(format!("{l2_tx_hash:#x}"), Style::default()),
                    Span::styled(amount.to_string(), Style::default()),
                ])
            });

        let l1_to_l2_messages_table = Table::new(rows, constraints)
            .header(
                Row::new(vec!["Status", "Kind", "L1 Tx Hash", "L2 Tx Hash", "Value"])
                    .style(Style::default()),
            )
            .block(
                Block::bordered()
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(Span::styled(
                        "L1 to L2 Messages",
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            );

        l1_to_l2_messages_table.render(area, buf, state);
    }
}
