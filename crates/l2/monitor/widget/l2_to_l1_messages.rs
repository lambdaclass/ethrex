use std::fmt::Display;

use ethrex_common::{Address, H256, U256};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_sdk::{COMMON_BRIDGE_L2_ADDRESS, calldata::encode_calldata};
use ethrex_rpc::{EthClient, clients::Overrides, types::receipt::RpcLog};
use keccak_hash::keccak;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Row, StatefulWidget, Table, TableState},
};

use crate::monitor::{
    self,
    widget::{ADDRESS_LENGTH_IN_DIGITS, HASH_LENGTH_IN_DIGITS, NUMBER_LENGTH_IN_DIGITS},
};

#[derive(Debug, Clone)]
pub enum L2ToL1MessageStatus {
    WithdrawalInitiated,
    WithdrawalClaimed,
    Sent,
    Delivered,
}

impl L2ToL1MessageStatus {
    pub async fn for_tx(
        l2_tx_hash: H256,
        common_bridge_address: Address,
        eth_client: &EthClient,
    ) -> Self {
        let withdrawal_is_claimed = {
            let calldata = encode_calldata(
                "claimedWithdrawals(bytes32)",
                &[Value::FixedBytes(l2_tx_hash.as_bytes().to_vec().into())],
            )
            .expect("Failed to encode claimedWithdrawals(bytes32) calldata");

            let raw_withdrawal_is_claimed: H256 = eth_client
                .call(common_bridge_address, calldata.into(), Overrides::default())
                .await
                .expect("Failed to call claimedWithdrawals(bytes32)")
                .parse()
                .unwrap_or_default();

            U256::from_big_endian(raw_withdrawal_is_claimed.as_fixed_bytes()) == U256::one()
        };

        if withdrawal_is_claimed {
            Self::WithdrawalClaimed
        } else {
            Self::WithdrawalInitiated
        }
    }
}

impl Display for L2ToL1MessageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            L2ToL1MessageStatus::WithdrawalInitiated => write!(f, "Initiated"),
            L2ToL1MessageStatus::WithdrawalClaimed => write!(f, "Claimed"),
            L2ToL1MessageStatus::Sent => write!(f, "Sent"),
            L2ToL1MessageStatus::Delivered => write!(f, "Delivered"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum L2ToL1MessageKind {
    ETHWithdraw,
    ERC20Withdraw,
    Message,
}

impl Display for L2ToL1MessageKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            L2ToL1MessageKind::ETHWithdraw => write!(f, "Withdraw (ETH)"),
            L2ToL1MessageKind::ERC20Withdraw => write!(f, "Withdraw (ERC20)"),
            L2ToL1MessageKind::Message => write!(f, "Message"),
        }
    }
}

pub type L2ToL1MessageRow = (
    L2ToL1MessageKind,
    L2ToL1MessageStatus,
    Address, // receiver in L1
    U256,    // value
    Address, // token (L2)
    Address, // token (L1)
    H256,    // L2 tx hash
);

pub struct L2ToL1MessagesTable {
    pub state: TableState,
    pub items: Vec<L2ToL1MessageRow>,
    last_l2_block_fetched: U256,
    common_bridge_address: Address,
}

impl L2ToL1MessagesTable {
    pub async fn new(
        common_bridge_address: Address,
        eth_client: &EthClient,
        rollup_client: &EthClient,
    ) -> Self {
        let mut last_l2_block_fetched = U256::zero();
        let items = Self::fetch_new_items(
            &mut last_l2_block_fetched,
            common_bridge_address,
            eth_client,
            rollup_client,
        )
        .await;
        Self {
            state: TableState::default(),
            items,
            last_l2_block_fetched,
            common_bridge_address,
        }
    }

    pub async fn on_tick(&mut self, eth_client: &EthClient, rollup_client: &EthClient) {
        let mut new_l1_to_l2_messages = Self::fetch_new_items(
            &mut self.last_l2_block_fetched,
            self.common_bridge_address,
            eth_client,
            rollup_client,
        )
        .await;
        new_l1_to_l2_messages.truncate(50);

        let n_new_latest_batches = new_l1_to_l2_messages.len();
        self.items.truncate(50 - n_new_latest_batches);
        self.refresh_items(eth_client).await;
        self.items.extend_from_slice(&new_l1_to_l2_messages);
        self.items.rotate_right(n_new_latest_batches);
    }

    async fn refresh_items(&mut self, eth_client: &EthClient) {
        for (_kind, status, .., l2_tx_hash) in self.items.iter_mut() {
            *status =
                L2ToL1MessageStatus::for_tx(*l2_tx_hash, self.common_bridge_address, eth_client)
                    .await;
        }
    }

    async fn fetch_new_items(
        last_l2_block_fetched: &mut U256,
        common_bridge_address: Address,
        eth_client: &EthClient,
        rollup_client: &EthClient,
    ) -> Vec<L2ToL1MessageRow> {
        let logs = Self::get_logs(last_l2_block_fetched, rollup_client).await;
        Self::process_logs(&logs, common_bridge_address, eth_client).await
    }

    async fn get_logs(last_l1_block_fetched: &mut U256, rollup_client: &EthClient) -> Vec<RpcLog> {
        monitor::utils::get_logs(
            last_l1_block_fetched,
            COMMON_BRIDGE_L2_ADDRESS,
            vec![],
            rollup_client,
        )
        .await
    }

    async fn process_logs(
        logs: &[RpcLog],
        common_bridge_address: Address,
        eth_client: &EthClient,
    ) -> Vec<L2ToL1MessageRow> {
        let mut processed_logs = Vec::new();

        let eth_withdrawal_topic = keccak(b"WithdrawalInitiated(address,address,uint256)");
        let erc20_withdrawal_topic =
            keccak(b"ERC20WithdrawalInitiated(address,address,address,uint256)");

        for log in logs {
            let withdrawal_status = L2ToL1MessageStatus::for_tx(
                log.transaction_hash,
                common_bridge_address,
                eth_client,
            )
            .await;
            match log.log.topics[0] {
                topic if topic == eth_withdrawal_topic => {
                    processed_logs.push((
                        L2ToL1MessageKind::ETHWithdraw,
                        withdrawal_status,
                        Address::from_slice(&log.log.topics[1].as_fixed_bytes()[12..]),
                        U256::from_big_endian(log.log.topics[2].as_fixed_bytes()),
                        Address::default(),
                        Address::default(),
                        log.transaction_hash,
                    ));
                }
                topic if topic == erc20_withdrawal_topic => {
                    processed_logs.push((
                        L2ToL1MessageKind::ERC20Withdraw,
                        withdrawal_status,
                        Address::from_slice(&log.log.topics[3].as_fixed_bytes()[12..]),
                        U256::from_big_endian(&log.log.data[log.log.data.len() - 32..]),
                        Address::from_slice(&log.log.topics[1].as_fixed_bytes()[12..]),
                        Address::from_slice(&log.log.topics[2].as_fixed_bytes()[12..]),
                        log.transaction_hash,
                    ));
                }
                _ => {
                    continue;
                }
            }
        }

        processed_logs
    }
}

impl StatefulWidget for &mut L2ToL1MessagesTable {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State)
    where
        Self: Sized,
    {
        let constraints = vec![
            Constraint::Length(9),
            Constraint::Length(16),
            Constraint::Length(ADDRESS_LENGTH_IN_DIGITS),
            Constraint::Length(NUMBER_LENGTH_IN_DIGITS),
            Constraint::Length(ADDRESS_LENGTH_IN_DIGITS),
            Constraint::Length(ADDRESS_LENGTH_IN_DIGITS),
            Constraint::Length(HASH_LENGTH_IN_DIGITS),
        ];

        let rows = self.items.iter().map(
            |(kind, status, receiver_on_l1, value, token_l1, token_l2, l2_tx_hash)| {
                Row::new(vec![
                    Span::styled(format!("{kind}"), Style::default()),
                    Span::styled(format!("{status}"), Style::default()),
                    Span::styled(format!("{receiver_on_l1:#x}"), Style::default()),
                    Span::styled(value.to_string(), Style::default()),
                    Span::styled(format!("{token_l1:#x}"), Style::default()),
                    Span::styled(format!("{token_l2:#x}"), Style::default()),
                    Span::styled(format!("{l2_tx_hash:#x}"), Style::default()),
                ])
            },
        );

        let l1_to_l2_messages_table = Table::new(rows, constraints)
            .header(
                Row::new(vec![
                    "Kind",
                    "Status",
                    "Receiver on L1",
                    "Value",
                    "Token L1",
                    "Token L2",
                    "L2 Tx Hash",
                ])
                .style(Style::default()),
            )
            .block(
                Block::bordered()
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(Span::styled(
                        "L2 to L1 Messages",
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            );

        l1_to_l2_messages_table.render(area, buf, state);
    }
}
