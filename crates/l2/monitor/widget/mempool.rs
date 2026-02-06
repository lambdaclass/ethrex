use ethrex_rpc::EthClient;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Row, StatefulWidget, Table, TableState},
};

use crate::{
    monitor::{
        utils::SelectableScroller,
        widget::{ADDRESS_LENGTH_IN_DIGITS, HASH_LENGTH_IN_DIGITS, NUMBER_LENGTH_IN_DIGITS},
    },
    sequencer::errors::MonitorError,
};

#[derive(Clone, Default)]
pub struct MempoolTable {
    pub state: TableState,
    // type | hash | sender | nonce
    pub items: Vec<(String, String, String, String)>,
    selected: bool,
}

impl MempoolTable {
    pub fn new() -> Self {
        Default::default()
    }

    pub async fn on_tick(&mut self, rollup_client: &EthClient) -> Result<(), MonitorError> {
        self.items = Self::refresh_items(rollup_client).await?;
        Ok(())
    }

    async fn refresh_items(
        rollup_client: &EthClient,
    ) -> Result<Vec<(String, String, String, String)>, MonitorError> {
        let mempool = rollup_client
            .tx_pool_content()
            .await
            .map_err(|_| MonitorError::TxPoolError)?;

        let mut pending_txs = mempool
            .pending
            .iter()
            .flat_map(|(sender, txs_sorted_by_nonce)| {
                txs_sorted_by_nonce.iter().map(|(nonce, tx)| {
                    (
                        format!("{}", tx.tx.tx_type()),
                        format!("{:#x}", tx.hash),
                        format!("{:#x}", *sender),
                        format!("{nonce}"),
                    )
                })
            })
            .collect::<Vec<_>>();

        pending_txs.sort_unstable_by(
            |(_tx_type_a, _, sender_a, nonce_a), (_tx_type_b, _, sender_b, nonce_b)| {
                sender_a.cmp(sender_b).then(nonce_a.cmp(nonce_b))
            },
        );

        Ok(pending_txs)
    }
}

impl StatefulWidget for &mut MempoolTable {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State)
    where
        Self: Sized,
    {
        let constraints = vec![
            Constraint::Length(10), // tx_type
            Constraint::Length(HASH_LENGTH_IN_DIGITS),
            Constraint::Length(ADDRESS_LENGTH_IN_DIGITS),
            Constraint::Length(NUMBER_LENGTH_IN_DIGITS),
        ];
        let rows = self.items.iter().map(|(tx_type, hash, sender, nonce)| {
            Row::new(vec![
                Span::styled(tx_type, Style::default()),
                Span::styled(hash, Style::default()),
                Span::styled(sender, Style::default()),
                Span::styled(nonce, Style::default()),
            ])
        });
        let mempool_table = Table::new(rows, constraints)
            .header(Row::new(vec!["Type", "Hash", "Sender", "Nonce"]).style(Style::default()))
            .block(
                Block::bordered()
                    .border_style(Style::default().fg(if self.selected {
                        Color::Magenta
                    } else {
                        Color::Cyan
                    }))
                    .title(Span::styled(
                        "Mempool",
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            );

        mempool_table.render(area, buf, state);
    }
}

impl SelectableScroller for MempoolTable {
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
