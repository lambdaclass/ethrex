use std::cmp::min;

use ethrex_common::{Address, H256, types::Block};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Row, StatefulWidget, Table, TableState},
};

use crate::monitor::widget::{
    ADDRESS_LENGTH_IN_DIGITS, BLOCK_SIZE_LENGTH_IN_DIGITS, GAS_USED_LENGTH_IN_DIGITS,
    HASH_LENGTH_IN_DIGITS, NUMBER_LENGTH_IN_DIGITS, TX_NUMBER_LENGTH_IN_DIGITS,
};

pub struct BlocksTable {
    pub state: TableState,
    // block number | #transactions | hash | coinbase | gas | blob gas | size
    pub items: Vec<(String, String, String, String, String, String, String)>,
    last_l2_block_known: u64,
}

impl BlocksTable {
    pub async fn new(store: &Store) -> Self {
        let mut last_l2_block_known = 0;
        let items = Self::refresh_items(&mut last_l2_block_known, store).await;
        Self {
            state: TableState::default(),
            items,
            last_l2_block_known,
        }
    }

    pub async fn on_tick(&mut self, store: &Store) {
        let mut new_blocks = Self::refresh_items(&mut self.last_l2_block_known, store).await;
        new_blocks.truncate(50);

        let n_new_blocks = new_blocks.len();
        self.items.truncate(50 - n_new_blocks);
        self.items.extend_from_slice(&new_blocks);
        self.items.rotate_right(n_new_blocks);
    }

    async fn refresh_items(
        last_l2_block_known: &mut u64,
        store: &Store,
    ) -> Vec<(String, String, String, String, String, String, String)> {
        let new_blocks = Self::get_blocks(last_l2_block_known, store).await;

        let new_blocks_processed = Self::process_blocks(new_blocks).await;

        new_blocks_processed
            .iter()
            .map(|(number, n_txs, hash, coinbase, gas, blob_gas, size)| {
                (
                    number.to_string(),
                    n_txs.to_string(),
                    format!("{hash:#x}"),
                    format!("{coinbase:#x}"),
                    gas.to_string(),
                    blob_gas.map_or("0".to_string(), |bg| bg.to_string()),
                    size.to_string(),
                )
            })
            .collect()
    }

    async fn get_blocks(last_l2_block_known: &mut u64, store: &Store) -> Vec<Block> {
        let last_l2_block_number = store
            .get_latest_block_number()
            .await
            .expect("Failed to get latest L2 block");

        let mut new_blocks = Vec::new();
        while *last_l2_block_known < last_l2_block_number {
            let new_last_l1_fetched_block = min(*last_l2_block_known + 1, last_l2_block_number);

            let new_block = store
                .get_block_by_number(new_last_l1_fetched_block)
                .await
                .unwrap_or_else(|_| {
                    panic!("Failed to get block  by number ({new_last_l1_fetched_block})")
                })
                .unwrap_or_else(|| {
                    panic!("Block {new_last_l1_fetched_block} not found in the store")
                });

            // Update the last L1 block fetched.
            *last_l2_block_known = new_last_l1_fetched_block;

            new_blocks.push(new_block);
        }

        new_blocks
    }

    async fn process_blocks(
        new_blocks: Vec<Block>,
    ) -> Vec<(u64, usize, H256, Address, u64, Option<u64>, usize)> {
        let mut new_blocks_processed = new_blocks
            .iter()
            .map(|block| {
                (
                    block.header.number,
                    block.body.transactions.len(),
                    block.header.hash(),
                    block.header.coinbase,
                    block.header.gas_used,
                    block.header.blob_gas_used,
                    block.encode_to_vec().len(),
                )
            })
            .collect::<Vec<_>>();

        new_blocks_processed.sort_by(
            |(number_a, _, _, _, _, _, _), (number_b, _, _, _, _, _, _)| number_b.cmp(number_a),
        );

        new_blocks_processed
    }
}

impl StatefulWidget for &mut BlocksTable {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State)
    where
        Self: Sized,
    {
        let constraints = vec![
            Constraint::Length(NUMBER_LENGTH_IN_DIGITS),
            Constraint::Length(TX_NUMBER_LENGTH_IN_DIGITS),
            Constraint::Length(HASH_LENGTH_IN_DIGITS),
            Constraint::Length(ADDRESS_LENGTH_IN_DIGITS),
            Constraint::Length(GAS_USED_LENGTH_IN_DIGITS),
            Constraint::Length(GAS_USED_LENGTH_IN_DIGITS),
            Constraint::Length(BLOCK_SIZE_LENGTH_IN_DIGITS),
        ];
        let rows = self
            .items
            .iter()
            .map(|(number, n_txs, hash, coinbase, gas, blob_bas, size)| {
                Row::new(vec![
                    Span::styled(number, Style::default()),
                    Span::styled(n_txs.to_string(), Style::default()),
                    Span::styled(hash, Style::default()),
                    Span::styled(coinbase, Style::default()),
                    Span::styled(gas.to_string(), Style::default()),
                    Span::styled(blob_bas.to_string(), Style::default()),
                    Span::styled(size.to_string(), Style::default()),
                ])
            });
        let latest_blocks_table = Table::new(rows, constraints)
            .header(
                Row::new(vec![
                    "Number", "#Txs", "Hash", "Coinbase", "Gas", "Blob Gas", "Size",
                ])
                .style(Style::default()),
            )
            .block(
                ratatui::widgets::Block::bordered()
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(Span::styled(
                        "L2 Blocks",
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            );

        latest_blocks_table.render(area, buf, state);
    }
}
