use ethrex_rpc::EthClient;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Row, StatefulWidget, Table, TableState},
};

use crate::monitor::widget::{
    ADDRESS_LENGTH_IN_DIGITS, HASH_LENGTH_IN_DIGITS, NUMBER_LENGTH_IN_DIGITS,
};

pub struct MempoolTable {
    pub state: TableState,
    // hash | sender | nonce
    pub items: Vec<(String, String, String)>,
}

impl MempoolTable {
    pub async fn new(rollup_client: &EthClient) -> Self {
        Self {
            state: TableState::default(),
            items: Self::refresh_items(rollup_client).await,
        }
    }

    pub async fn on_tick(&mut self, rollup_client: &EthClient) {
        self.items = Self::refresh_items(rollup_client).await;
    }

    async fn refresh_items(rollup_client: &EthClient) -> Vec<(String, String, String)> {
        let mempool = rollup_client
            .tx_pool_content()
            .await
            .expect("Failed to get mempool content");

        let mut pending_txs = mempool
            .pending
            .iter()
            .flat_map(|(sender, txs_sorted_by_nonce)| {
                txs_sorted_by_nonce.iter().map(|(nonce, tx)| {
                    (
                        format!("{:#x}", tx.hash),
                        format!("{:#x}", *sender),
                        format!("{nonce}"),
                    )
                })
            })
            .collect::<Vec<_>>();

        pending_txs.sort_by(|(_, sender_a, nonce_a), (_, sender_b, nonce_b)| {
            sender_a.cmp(sender_b).then(nonce_a.cmp(nonce_b))
        });

        pending_txs
    }
}

impl StatefulWidget for &mut MempoolTable {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State)
    where
        Self: Sized,
    {
        let constraints = vec![
            Constraint::Length(HASH_LENGTH_IN_DIGITS),
            Constraint::Length(ADDRESS_LENGTH_IN_DIGITS),
            Constraint::Length(NUMBER_LENGTH_IN_DIGITS),
        ];
        let rows = self.items.iter().map(|(hash, sender, nonce)| {
            Row::new(vec![
                Span::styled(hash, Style::default()),
                Span::styled(sender, Style::default()),
                Span::styled(nonce, Style::default()),
            ])
        });
        let mempool_table = Table::new(rows, constraints)
            .header(Row::new(vec!["Hash", "Sender", "Nonce"]).style(Style::default()))
            .block(
                Block::bordered()
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(Span::styled(
                        "Mempool",
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            );

        mempool_table.render(area, buf, state);
    }
}
