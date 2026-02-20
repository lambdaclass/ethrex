use std::fmt::Display;

use ethrex_common::utils::keccak;
use ethrex_common::{Address, H256, U256};
use ethrex_l2_common::{calldata::Value, messages::MESSENGER_ADDRESS};
use ethrex_l2_sdk::{COMMON_BRIDGE_L2_ADDRESS, calldata::encode_calldata};
use ethrex_rpc::{EthClient, clients::Overrides, types::receipt::RpcLog};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Row, StatefulWidget, Table, TableState},
};

use crate::{
    error::MonitorError,
    utils::SelectableScroller,
    widget::{ADDRESS_LENGTH_IN_DIGITS, HASH_LENGTH_IN_DIGITS, NUMBER_LENGTH_IN_DIGITS},
};

/*
event WithdrawalInitiated(
    address indexed senderOnL2,   => topic 1
    address indexed receiverOnL1, => topic 2
    uint256 indexed amount        => topic 3
);
*/
const WITHDRAWAL_ETH_RECEIVER_TOPIC_IDX: usize = 2;
const WITHDRAWAL_ETH_AMOUNT_TOPIC_IDX: usize = 3;
/*
event ERC20WithdrawalInitiated(
    address indexed tokenL1,      => topic 1
    address indexed tokenL2,      => topic 2
    address indexed receiverOnL1, => topic 3
    uint256 amount                => data 0..32
);
*/
const WITHDRAWAL_ERC20_TOKEN_L1_TOPIC_IDX: usize = 1;
const WITHDRAWAL_ERC20_TOKEN_L2_TOPIC_IDX: usize = 2;
const WITHDRAWAL_ERC20_RECEIVER_TOPIC_IDX: usize = 3;
/*
event L1Message(
    address indexed senderOnL2,   => topic 1
    bytes32 indexed data,         => topic 2
    uint256 indexed messageId     => topic 3
);
*/
const L1MESSAGE_MESSAGE_ID_TOPIC_IDX: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
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
        l1_client: &EthClient,
        l2_client: &EthClient,
    ) -> Result<Self, MonitorError> {
        let tx_receipt = l2_client
            .get_transaction_receipt(l2_tx_hash)
            .await?
            .ok_or(MonitorError::ReceiptError)?;
        let l1message_log = tx_receipt
            .logs
            .iter()
            .find(|log| log.log.address == MESSENGER_ADDRESS)
            .ok_or(MonitorError::NoLogs)?;
        let msg_id = l1message_log
            .log
            .topics
            .get(L1MESSAGE_MESSAGE_ID_TOPIC_IDX)
            .ok_or(MonitorError::LogsTopics(L1MESSAGE_MESSAGE_ID_TOPIC_IDX))?;
        let withdrawal_is_claimed = {
            let calldata = encode_calldata(
                "claimedWithdrawalIDs(uint256)",
                &[Value::FixedBytes(msg_id.as_bytes().to_vec().into())],
            )
            .map_err(MonitorError::CalldataEncodeError)?;

            let raw_withdrawal_is_claimed: H256 = l1_client
                .call(common_bridge_address, calldata.into(), Overrides::default())
                .await
                .map_err(MonitorError::EthClientError)?
                .parse()
                .unwrap_or_default();

            U256::from_big_endian(raw_withdrawal_is_claimed.as_fixed_bytes()) == U256::one()
        };

        if withdrawal_is_claimed {
            Ok(Self::WithdrawalClaimed)
        } else {
            Ok(Self::WithdrawalInitiated)
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

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L2ToL1MessageRow {
    pub kind: L2ToL1MessageKind,
    pub status: L2ToL1MessageStatus,
    pub receiver: Address,
    pub value: U256,
    pub token_l1: Address,
    pub token_l2: Address,
    pub l2_tx_hash: H256,
}

#[derive(Clone, Default)]
pub struct L2ToL1MessagesTable {
    pub state: TableState,
    pub items: Vec<L2ToL1MessageRow>,
    last_l2_block_fetched: U256,
    common_bridge_address: Address,
    selected: bool,
}

impl L2ToL1MessagesTable {
    pub fn new(common_bridge_address: Address) -> Self {
        Self {
            common_bridge_address,
            ..Default::default()
        }
    }

    pub async fn on_tick(
        &mut self,
        eth_client: &EthClient,
        rollup_client: &EthClient,
    ) -> Result<(), MonitorError> {
        let mut new_l2_to_l1_messages = Self::fetch_new_items(
            &mut self.last_l2_block_fetched,
            self.common_bridge_address,
            eth_client,
            rollup_client,
        )
        .await?;
        new_l2_to_l1_messages.drain(..new_l2_to_l1_messages.len().saturating_sub(50));

        let n_new = new_l2_to_l1_messages.len();
        let items_to_keep = 50usize.saturating_sub(n_new);
        self.items.drain(..self.items.len().saturating_sub(items_to_keep));
        self.refresh_items(eth_client, rollup_client).await?;
        self.items.extend_from_slice(&new_l2_to_l1_messages);
        self.items.rotate_right(n_new);

        Ok(())
    }

    async fn refresh_items(
        &mut self,
        l1_client: &EthClient,
        l2_client: &EthClient,
    ) -> Result<(), MonitorError> {
        for row in self.items.iter_mut() {
            row.status = L2ToL1MessageStatus::for_tx(
                row.l2_tx_hash,
                self.common_bridge_address,
                l1_client,
                l2_client,
            )
            .await?;
        }
        Ok(())
    }

    async fn fetch_new_items(
        last_l2_block_fetched: &mut U256,
        common_bridge_address: Address,
        eth_client: &EthClient,
        rollup_client: &EthClient,
    ) -> Result<Vec<L2ToL1MessageRow>, MonitorError> {
        let logs = crate::utils::get_logs(
            last_l2_block_fetched,
            COMMON_BRIDGE_L2_ADDRESS,
            vec![],
            rollup_client,
        )
        .await?;
        Self::process_logs(&logs, common_bridge_address, eth_client, rollup_client).await
    }

    async fn process_logs(
        logs: &[RpcLog],
        common_bridge_address: Address,
        l1_client: &EthClient,
        l2_client: &EthClient,
    ) -> Result<Vec<L2ToL1MessageRow>, MonitorError> {
        let mut processed_logs = Vec::new();

        let eth_withdrawal_topic = keccak(b"WithdrawalInitiated(address,address,uint256)");
        let erc20_withdrawal_topic =
            keccak(b"ERC20WithdrawalInitiated(address,address,address,uint256)");

        for log in logs {
            let withdrawal_status = match L2ToL1MessageStatus::for_tx(
                log.transaction_hash,
                common_bridge_address,
                l1_client,
                l2_client,
            )
            .await
            {
                Ok(status) => status,
                Err(MonitorError::NoLogs) => continue,
                Err(e) => return Err(e),
            };
            match *log.log.topics.first().ok_or(MonitorError::LogsTopics(0))? {
                topic if topic == eth_withdrawal_topic => {
                    processed_logs.push(L2ToL1MessageRow {
                        kind: L2ToL1MessageKind::ETHWithdraw,
                        status: withdrawal_status,
                        receiver: Address::from_slice(
                            &log.log
                                .topics
                                .get(WITHDRAWAL_ETH_RECEIVER_TOPIC_IDX)
                                .ok_or(MonitorError::LogsTopics(WITHDRAWAL_ETH_RECEIVER_TOPIC_IDX))?
                                .as_fixed_bytes()[12..],
                        ),
                        value: U256::from_big_endian(
                            log.log
                                .topics
                                .get(WITHDRAWAL_ETH_AMOUNT_TOPIC_IDX)
                                .ok_or(MonitorError::LogsTopics(WITHDRAWAL_ETH_AMOUNT_TOPIC_IDX))?
                                .as_fixed_bytes(),
                        ),
                        token_l1: Address::default(),
                        token_l2: Address::default(),
                        l2_tx_hash: log.transaction_hash,
                    });
                }
                topic if topic == erc20_withdrawal_topic => {
                    processed_logs.push(L2ToL1MessageRow {
                        kind: L2ToL1MessageKind::ERC20Withdraw,
                        status: withdrawal_status,
                        receiver: Address::from_slice(
                            &log.log
                                .topics
                                .get(WITHDRAWAL_ERC20_RECEIVER_TOPIC_IDX)
                                .ok_or(MonitorError::LogsTopics(
                                    WITHDRAWAL_ERC20_RECEIVER_TOPIC_IDX,
                                ))?
                                .as_fixed_bytes()[12..],
                        ),
                        value: U256::from_big_endian(
                            log.log.data.get(0..32).ok_or(MonitorError::LogsData(32))?,
                        ),
                        token_l1: Address::from_slice(
                            &log.log
                                .topics
                                .get(WITHDRAWAL_ERC20_TOKEN_L1_TOPIC_IDX)
                                .ok_or(MonitorError::LogsTopics(
                                    WITHDRAWAL_ERC20_TOKEN_L1_TOPIC_IDX,
                                ))?
                                .as_fixed_bytes()[12..],
                        ),
                        token_l2: Address::from_slice(
                            &log.log
                                .topics
                                .get(WITHDRAWAL_ERC20_TOKEN_L2_TOPIC_IDX)
                                .ok_or(MonitorError::LogsTopics(
                                    WITHDRAWAL_ERC20_TOKEN_L2_TOPIC_IDX,
                                ))?
                                .as_fixed_bytes()[12..],
                        ),
                        l2_tx_hash: log.transaction_hash,
                    });
                }
                _ => {
                    continue;
                }
            }
        }

        Ok(processed_logs)
    }
}

impl StatefulWidget for &mut L2ToL1MessagesTable {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State)
    where
        Self: Sized,
    {
        let constraints = vec![
            Constraint::Length(16),
            Constraint::Length(9),
            Constraint::Length(ADDRESS_LENGTH_IN_DIGITS),
            Constraint::Length(NUMBER_LENGTH_IN_DIGITS),
            Constraint::Length(ADDRESS_LENGTH_IN_DIGITS),
            Constraint::Length(ADDRESS_LENGTH_IN_DIGITS),
            Constraint::Length(HASH_LENGTH_IN_DIGITS),
        ];

        let rows = self.items.iter().map(|row| {
            Row::new(vec![
                Span::styled(format!("{}", row.kind), Style::default()),
                Span::styled(format!("{}", row.status), Style::default()),
                Span::styled(format!("{:#x}", row.receiver), Style::default()),
                Span::styled(row.value.to_string(), Style::default()),
                Span::styled(format!("{:#x}", row.token_l1), Style::default()),
                Span::styled(format!("{:#x}", row.token_l2), Style::default()),
                Span::styled(format!("{:#x}", row.l2_tx_hash), Style::default()),
            ])
        });

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
                    .border_style(Style::default().fg(if self.selected {
                        Color::Magenta
                    } else {
                        Color::Cyan
                    }))
                    .title(Span::styled(
                        "L2 to L1 Messages",
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            );

        l1_to_l2_messages_table.render(area, buf, state);
    }
}

impl SelectableScroller for L2ToL1MessagesTable {
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
