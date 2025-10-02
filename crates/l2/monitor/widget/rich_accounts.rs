use bytes::Bytes;
use ethrex_common::{Address, U256};
use ethrex_config::networks::LOCAL_DEVNET_PRIVATE_KEYS;
use ethrex_l2_common::utils::get_address_from_secret_key;
use ethrex_rpc::{EthClient, types::block_identifier::BlockIdentifier};
use hex::FromHexError;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Row, StatefulWidget, Table, TableState},
};
use secp256k1::SecretKey;

use crate::{
    monitor::{utils::SelectableScroller, widget::HASH_LENGTH_IN_DIGITS},
    sequencer::errors::MonitorError,
};

// address | private key | balance
pub type RichAccountRow = (Address, SecretKey, U256);

#[derive(Clone, Default)]
pub struct RichAccountsTable {
    pub state: TableState,
    pub items: Vec<RichAccountRow>,
    last_block_fetched: U256,

    selected: bool,
}

impl RichAccountsTable {
    pub async fn new(rollup_client: &EthClient) -> Result<Self, MonitorError> {
        let last_block_fetched = rollup_client
            .get_block_number()
            .await
            .map_err(|_| MonitorError::GetLatestBlock)?;
        let items = Self::get_accounts(rollup_client, last_block_fetched).await?;
        Ok(Self {
            items,
            last_block_fetched,
            selected: true,
            ..Default::default()
        })
    }
    async fn get_accounts(
        rollup_client: &EthClient,
        last_block_fetched: U256,
    ) -> Result<Vec<RichAccountRow>, MonitorError> {
        // TODO: enable custom private keys
        let private_keys: Vec<String> = LOCAL_DEVNET_PRIVATE_KEYS
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_string())
            .collect();

        let mut accounts = Vec::with_capacity(private_keys.len());
        for pk in private_keys.iter() {
            let secret_key = SecretKey::from_slice(&parse_hex(pk)?)
                .map_err(|e| MonitorError::DecodingError(format!("Invalid private key: {e}")))?;
            let address = get_address_from_secret_key(&secret_key).map_err(|e| {
                MonitorError::DecodingError(format!("Failed to get address from private key: {e}"))
            })?;
            let get_balance = rollup_client
                .get_balance(
                    address,
                    BlockIdentifier::Number(last_block_fetched.as_u64()),
                )
                .await?;
            accounts.push((address, secret_key, get_balance));
        }
        Ok(accounts)
    }

    pub async fn on_tick(&mut self, rollup_client: &EthClient) -> Result<(), MonitorError> {
        let latest_block = rollup_client
            .get_block_number()
            .await
            .map_err(|_| MonitorError::GetLatestBlock)?;
        if latest_block == self.last_block_fetched {
            return Ok(());
        }
        for (address, _private_key, balance) in self.items.iter_mut() {
            *balance = rollup_client
                .get_balance(*address, BlockIdentifier::Number(latest_block.as_u64()))
                .await?;
        }
        self.last_block_fetched = latest_block;
        Ok(())
    }
}

pub fn parse_hex(s: &str) -> Result<Bytes, FromHexError> {
    match s.strip_prefix("0x") {
        Some(s) => hex::decode(s).map(Into::into),
        None => hex::decode(s).map(Into::into),
    }
}

impl StatefulWidget for &mut RichAccountsTable {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State)
    where
        Self: Sized,
    {
        let constraints = vec![
            Constraint::Fill(1),
            Constraint::Length(HASH_LENGTH_IN_DIGITS),
            Constraint::Fill(1),
        ];

        let selected_index = state.selected();

        let rows = self
            .items
            .iter()
            .enumerate()
            .map(|(i, (address, private_key, balance))| {
                let mut row = Row::new(vec![
                    Span::styled(format!("0x{address:x}"), Style::default()),
                    Span::styled(
                        format!("0x{}", private_key.display_secret()),
                        Style::default(),
                    ),
                    Span::styled(balance.to_string(), Style::default()),
                ]);

                if Some(i) == selected_index {
                    row = row.style(Style::default().bg(Color::Blue));
                }

                row
            });

        let rich_accounts_table = Table::new(rows, constraints)
            .header(Row::new(vec!["Address", "Private Key", "Balance"]).style(Style::default()))
            .block(
                Block::bordered()
                    .border_style(Style::default().fg(if self.selected {
                        Color::Magenta
                    } else {
                        Color::Cyan
                    }))
                    .title(Span::styled(
                        "Rich Accounts",
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            );

        rich_accounts_table.render(area, buf, state);
    }
}

impl SelectableScroller for RichAccountsTable {
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
