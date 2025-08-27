use bytes::Bytes;
use ethrex_common::{Address, U256};
use ethrex_config::networks::LOCAL_DEVNET_PRIVATE_KEYS;
use ethrex_l2_sdk::get_address_from_secret_key;
use ethrex_rpc::{
    EthClient,
    types::block_identifier::{BlockIdentifier, BlockTag},
};
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

    selected: bool,
}

impl RichAccountsTable {
    pub async fn new(rollup_client: &EthClient) -> Result<Self, MonitorError> {
        let items = Self::get_accounts(rollup_client).await?;
        Ok(Self {
            items,
            ..Default::default()
        })
    }
    async fn get_accounts(rollup_client: &EthClient) -> Result<Vec<RichAccountRow>, MonitorError> {
        // TODO: enable custom private keys
        let private_keys: Vec<String> = LOCAL_DEVNET_PRIVATE_KEYS
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_string())
            .collect();

        let mut accounts = Vec::with_capacity(private_keys.len());
        for pk in private_keys.iter() {
            let secret_key = parse_private_key(pk).map_err(|_| {
                MonitorError::DecodingError("Error while parsing private key".to_string())
            })?;
            let address = get_address_from_secret_key(&secret_key)?;
            let get_balance = rollup_client
                .get_balance(address, BlockIdentifier::Tag(BlockTag::Latest))
                .await?;
            accounts.push((address, secret_key, get_balance));
        }
        Ok(accounts)
    }

    pub async fn on_tick(&mut self, rollup_client: &EthClient) -> Result<(), MonitorError> {
        for (address, _private_key, balance) in self.items.iter_mut() {
            *balance = rollup_client
                .get_balance(*address, BlockIdentifier::Tag(BlockTag::Latest))
                .await?;
        }
        Ok(())
    }
}

pub fn parse_private_key(s: &str) -> Result<SecretKey, MonitorError> {
    Ok(SecretKey::from_slice(&parse_hex(s)?)?)
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

        let rows = self.items.iter().map(|(address, private_key, balance)| {
            Row::new(vec![
                Span::styled(format!("{address}"), Style::default()),
                Span::styled(
                    format!("{}", private_key.display_secret()),
                    Style::default(),
                ),
                Span::styled(balance.to_string(), Style::default()),
            ])
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
                        "L1 to L2 Messages",
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
