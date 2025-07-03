use std::cmp::min;

use ethrex_common::{Address, H256, U256};
use ethrex_rpc::{
    EthClient,
    clients::eth::BlockByNumber,
    types::block::{BlockBodyWrapper, RpcBlock},
};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Row, StatefulWidget, Table, TableState},
};

use crate::monitor::widget::{
    ADDRESS_LENGTH_IN_DIGITS, BLOCK_SIZE_LENGTH_IN_DIGITS, GAS_USED_LENGTH_IN_DIGITS,
    HASH_LENGTH_IN_DIGITS, NUMBER_LENGTH_IN_DIGITS, TX_NUMBER_LENGTH_IN_DIGITS,
};

pub struct BlocksTable {
    pub state: TableState,
    // block number | #transactions | hash | coinbase | gas | blob gas | size
    pub items: Vec<(String, String, String, String, String, String, String)>,
    last_l2_block_known: U256,
}

impl BlocksTable {
    pub async fn new(rollup_client: &EthClient) -> Self {
        let mut last_l2_block_known = U256::zero();
        let items = Self::refresh_items(&mut last_l2_block_known, rollup_client).await;
        Self {
            state: TableState::default(),
            items,
            last_l2_block_known,
        }
    }

    pub async fn on_tick(&mut self, rollup_client: &EthClient) {
        let mut new_blocks =
            Self::refresh_items(&mut self.last_l2_block_known, rollup_client).await;

        let n_new_blocks = new_blocks.len();

        if n_new_blocks > 50 {
            new_blocks.truncate(50);
            self.items.extend_from_slice(&new_blocks);
        } else {
            self.items.truncate(50 - n_new_blocks);
            self.items.extend_from_slice(&new_blocks);
            self.items.rotate_right(n_new_blocks);
        }
    }

    async fn refresh_items(
        last_l2_block_known: &mut U256,
        rollup_client: &EthClient,
    ) -> Vec<(String, String, String, String, String, String, String)> {
        let new_blocks = Self::get_blocks(last_l2_block_known, rollup_client).await;

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

    async fn get_blocks(
        last_l2_block_known: &mut U256,
        rollup_client: &EthClient,
    ) -> Vec<RpcBlock> {
        let last_l2_block_number = rollup_client
            .get_block_number()
            .await
            .expect("Failed to get latest L2 block");

        let mut new_blocks = Vec::new();
        while *last_l2_block_known < last_l2_block_number {
            let new_last_l1_fetched_block = min(*last_l2_block_known + 1, last_l2_block_number);

            let new_block = rollup_client
                .get_block_by_number(BlockByNumber::Number(new_last_l1_fetched_block.as_u64()))
                .await
                .unwrap_or_else(|_| {
                    panic!("Failed to get block  by number ({new_last_l1_fetched_block})")
                });

            // Update the last L1 block fetched.
            *last_l2_block_known = new_last_l1_fetched_block;

            new_blocks.push(new_block);
        }

        new_blocks
    }

    async fn process_blocks(
        new_blocks: Vec<RpcBlock>,
    ) -> Vec<(u64, usize, H256, Address, u64, Option<u64>, u64)> {
        let mut new_blocks_processed = new_blocks
            .iter()
            .map(|block| {
                let n_txs = match &block.body {
                    BlockBodyWrapper::Full(full_block_body) => full_block_body.transactions.len(),
                    BlockBodyWrapper::OnlyHashes(only_hashes_block_body) => {
                        only_hashes_block_body.transactions.len()
                    }
                };
                (
                    block.header.number,
                    n_txs,
                    block.header.hash(),
                    block.header.coinbase,
                    block.header.gas_used,
                    block.header.blob_gas_used,
                    block.size,
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
                Block::bordered()
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(Span::styled(
                        "L2 Blocks",
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            );

        latest_blocks_table.render(area, buf, state);
    }
}
