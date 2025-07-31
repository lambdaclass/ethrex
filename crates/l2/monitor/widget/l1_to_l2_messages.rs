use std::fmt::Display;

use ethrex_common::{Address, H256, U256};
use ethrex_l2_sdk::COMMON_BRIDGE_L2_ADDRESS;
use ethrex_rpc::{EthClient, types::receipt::RpcLog};
use ethrex_storage::Store;
use keccak_hash::keccak;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Row, StatefulWidget, Table, TableState},
};

use crate::{
    monitor::{self, utils::SelectableScroller, widget::HASH_LENGTH_IN_DIGITS},
    sequencer::{errors::MonitorError, l1_watcher::PrivilegedTransactionData},
};

// kind | status | L1 tx hash | L2 tx hash | amount
pub type L1ToL2MessagesRow = (L1ToL2MessageKind, L1ToL2MessageStatus, H256, H256, U256);

#[derive(Clone, Default)]
pub struct L1ToL2MessagesTable {
    pub state: TableState,
    pub items: Vec<L1ToL2MessagesRow>,
    last_l1_block_fetched: U256,
    common_bridge_address: Address,
    selected: bool,
}

#[derive(Debug, Clone)]
pub enum L1ToL2MessageStatus {
    Unknown = 0,
    Pending = 1,
    ProcessedOnL2 = 3,
    Committed = 4,
    Verified = 5,
}

impl L1ToL2MessageStatus {
    pub async fn for_tx(
        l2_tx_hash: H256,
        common_bridge_address: Address,
        eth_client: &EthClient,
        store: &Store,
    ) -> Result<Self, MonitorError> {
        if let Ok(Some(_tx)) = store.get_transaction_by_hash(l2_tx_hash).await {
            Ok(Self::ProcessedOnL2)
        } else if eth_client
            .get_pending_privileged_transactions(common_bridge_address)
            .await
            .map_err(|_| MonitorError::GetPendingPrivilegedTx)?
            .contains(&l2_tx_hash)
        {
            Ok(Self::Pending)
        } else {
            Ok(Self::Unknown)
        }
    }
}

impl Display for L1ToL2MessageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            L1ToL2MessageStatus::Unknown => write!(f, "Unknown"),
            L1ToL2MessageStatus::Pending => write!(f, "Pending"),
            L1ToL2MessageStatus::ProcessedOnL2 => write!(f, "Processed on L2"),
            L1ToL2MessageStatus::Committed => write!(f, "Committed"),
            L1ToL2MessageStatus::Verified => write!(f, "Verified"),
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
    pub fn new(common_bridge_address: Address) -> Self {
        Self {
            common_bridge_address,
            ..Default::default()
        }
    }

    pub async fn on_tick(
        &mut self,
        eth_client: &EthClient,
        store: &Store,
    ) -> Result<(), MonitorError> {
        let mut new_l1_to_l2_messages = Self::fetch_new_items(
            &mut self.last_l1_block_fetched,
            self.common_bridge_address,
            eth_client,
            store,
        )
        .await?;
        new_l1_to_l2_messages.truncate(50);

        let n_new_latest_batches = new_l1_to_l2_messages.len();
        self.items.truncate(50 - n_new_latest_batches);
        self.refresh_items(eth_client, store).await?;
        self.items.extend_from_slice(&new_l1_to_l2_messages);
        self.items.rotate_right(n_new_latest_batches);
        Ok(())
    }

    async fn refresh_items(
        &mut self,
        eth_client: &EthClient,
        store: &Store,
    ) -> Result<(), MonitorError> {
        for (_kind, status, _l1_tx_hash, l2_tx_hash, ..) in self.items.iter_mut() {
            *status = L1ToL2MessageStatus::for_tx(
                *l2_tx_hash,
                self.common_bridge_address,
                eth_client,
                store,
            )
            .await?;
        }
        Ok(())
    }

    async fn fetch_new_items(
        last_l1_block_fetched: &mut U256,
        common_bridge_address: Address,
        eth_client: &EthClient,
        store: &Store,
    ) -> Result<Vec<L1ToL2MessagesRow>, MonitorError> {
        let logs = monitor::utils::get_logs(
            last_l1_block_fetched,
            common_bridge_address,
            vec!["PrivilegedTxSent(address,address,uint256,uint256,uint256,bytes)"],
            eth_client,
        )
        .await?;
        Self::process_logs(&logs, common_bridge_address, eth_client, store).await
    }

    async fn process_logs(
        logs: &[RpcLog],
        common_bridge_address: Address,
        eth_client: &EthClient,
        store: &Store,
    ) -> Result<Vec<L1ToL2MessagesRow>, MonitorError> {
        let mut processed_logs = Vec::new();

        for log in logs {
            let l1_to_l2_message = PrivilegedTransactionData::from_log(log.log.clone())
                .map_err(|_| MonitorError::PrivilegedTxParseError)?;

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
                L1ToL2MessageKind::from(&l1_to_l2_message),
                L1ToL2MessageStatus::for_tx(
                    l1_to_l2_message_hash,
                    common_bridge_address,
                    eth_client,
                    store,
                )
                .await?,
                log.transaction_hash,
                l1_to_l2_message_hash,
                l1_to_l2_message.value,
            ));
        }

        Ok(processed_logs)
    }
}

impl StatefulWidget for &mut L1ToL2MessagesTable {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State)
    where
        Self: Sized,
    {
        let constraints = vec![
            Constraint::Length(10),
            Constraint::Length(15),
            Constraint::Length(HASH_LENGTH_IN_DIGITS),
            Constraint::Length(HASH_LENGTH_IN_DIGITS),
            Constraint::Fill(1),
        ];

        let rows = self
            .items
            .iter()
            .map(|(kind, status, l1_tx_hash, l2_tx_hash, amount)| {
                Row::new(vec![
                    Span::styled(format!("{kind}"), Style::default()),
                    Span::styled(format!("{status}"), Style::default()),
                    Span::styled(format!("{l1_tx_hash:#x}"), Style::default()),
                    Span::styled(format!("{l2_tx_hash:#x}"), Style::default()),
                    Span::styled(amount.to_string(), Style::default()),
                ])
            });

        let l1_to_l2_messages_table = Table::new(rows, constraints)
            .header(
                Row::new(vec!["Kind", "Status", "L1 Tx Hash", "L2 Tx Hash", "Value"])
                    .style(Style::default()),
            )
            .block(
                Block::bordered()
                    .border_style(Style::default().fg(if self.selected {
                        Color::Magenta
                    } else {
                        Color::Cyan
                    }))
                    .title(Span::styled(
                        "L1 to L2 Messages",
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            );

        l1_to_l2_messages_table.render(area, buf, state);
    }
}

impl SelectableScroller for L1ToL2MessagesTable {
    fn selected(&mut self, is_selected: bool) {
        self.selected = is_selected;
    }
    fn scroll_up(&mut self) {
        let selected = self.state.selected_mut();
        *selected = Some(selected.unwrap_or(0).saturating_sub(1))
    }
    fn scroll_down(&mut self) {
        let selected = self.state.selected_mut();
        *selected = Some(
            selected
                .unwrap_or(0)
                .saturating_add(1)
                .min(self.items.len().saturating_sub(1)),
        )
    }
}
