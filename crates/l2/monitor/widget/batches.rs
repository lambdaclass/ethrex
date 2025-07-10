use ethrex_common::{Address, H256, types::batch::Batch};
use ethrex_rpc::EthClient;
use ethrex_storage_rollup::StoreRollup;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Row, StatefulWidget, Table, TableState},
};

use crate::monitor::widget::{HASH_LENGTH_IN_DIGITS, NUMBER_LENGTH_IN_DIGITS};

pub struct BatchesTable {
    pub state: TableState,
    // batch number | # blocks | # messages | commit tx hash | verify tx hash
    #[expect(clippy::type_complexity)]
    pub items: Vec<(u64, u64, usize, Option<H256>, Option<H256>)>,
    last_l1_block_fetched: u64,
    on_chain_proposer_address: Address,
}

impl BatchesTable {
    pub async fn new(
        on_chain_proposer_address: Address,
        eth_client: &EthClient,
        rollup_store: &StoreRollup,
    ) -> Self {
        let mut last_l1_block_fetched = 0;
        let items = Self::fetch_new_items(
            &mut last_l1_block_fetched,
            on_chain_proposer_address,
            eth_client,
            rollup_store,
        )
        .await;
        Self {
            state: TableState::default(),
            items,
            last_l1_block_fetched,
            on_chain_proposer_address,
        }
    }

    pub async fn on_tick(&mut self, eth_client: &EthClient, rollup_store: &StoreRollup) {
        let mut new_latest_batches = Self::fetch_new_items(
            &mut self.last_l1_block_fetched,
            self.on_chain_proposer_address,
            eth_client,
            rollup_store,
        )
        .await;
        new_latest_batches.truncate(50);

        let n_new_latest_batches = new_latest_batches.len();
        self.items.truncate(50 - n_new_latest_batches);
        self.refresh_items(rollup_store).await;
        self.items.extend_from_slice(&new_latest_batches);
        self.items.rotate_right(n_new_latest_batches);
    }

    async fn refresh_items(&mut self, rollup_store: &StoreRollup) {
        if self.items.is_empty() {
            return;
        }

        let mut from = self.items.last().expect("Expected items in the table").0 - 1;

        let refreshed_batches = Self::get_batches(
            &mut from,
            self.items.first().expect("Expected items in the table").0,
            rollup_store,
        )
        .await;

        let refreshed_items = Self::process_batches(refreshed_batches).await;

        self.items = refreshed_items;
    }

    async fn fetch_new_items(
        last_l2_batch_fetched: &mut u64,
        on_chain_proposer_address: Address,
        eth_client: &EthClient,
        rollup_store: &StoreRollup,
    ) -> Vec<(u64, u64, usize, Option<H256>, Option<H256>)> {
        let last_l2_batch_number = eth_client
            .get_last_committed_batch(on_chain_proposer_address)
            .await
            .expect("Failed to get latest L2 batch");

        let new_batches =
            Self::get_batches(last_l2_batch_fetched, last_l2_batch_number, rollup_store).await;

        Self::process_batches(new_batches).await
    }

    async fn get_batches(from: &mut u64, to: u64, rollup_store: &StoreRollup) -> Vec<Batch> {
        let mut new_batches = Vec::new();

        for batch_number in *from + 1..=to {
            let batch = rollup_store
                .get_batch(batch_number)
                .await
                .unwrap_or_else(|err| {
                    panic!("Failed to get batch by number ({batch_number}): {err}")
                })
                .unwrap_or_else(|| panic!("Batch {batch_number} not found in the rollup store"));

            *from = batch_number;

            new_batches.push(batch);
        }

        new_batches
    }

    async fn process_batches(
        new_batches: Vec<Batch>,
    ) -> Vec<(u64, u64, usize, Option<H256>, Option<H256>)> {
        let mut new_blocks_processed = new_batches
            .iter()
            .map(|batch| {
                (
                    batch.number,
                    batch.last_block - batch.first_block + 1,
                    batch.message_hashes.len(),
                    batch.commit_tx,
                    batch.verify_tx,
                )
            })
            .collect::<Vec<_>>();

        new_blocks_processed
            .sort_by(|(number_a, _, _, _, _), (number_b, _, _, _, _)| number_b.cmp(number_a));

        new_blocks_processed
    }
}

impl StatefulWidget for &mut BatchesTable {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let constraints = vec![
            Constraint::Length(NUMBER_LENGTH_IN_DIGITS),
            Constraint::Length(NUMBER_LENGTH_IN_DIGITS),
            Constraint::Length(17),
            Constraint::Length(HASH_LENGTH_IN_DIGITS),
            Constraint::Length(HASH_LENGTH_IN_DIGITS),
        ];
        let rows = self.items.iter().map(
            |(number, n_blocks, n_messages, commit_tx_hash, verify_tx_hash)| {
                Row::new(vec![
                    Span::styled(number.to_string(), Style::default()),
                    Span::styled(n_blocks.to_string(), Style::default()),
                    Span::styled(n_messages.to_string(), Style::default()),
                    Span::styled(
                        commit_tx_hash
                            .map_or_else(|| "Uncommitted".to_string(), |hash| format!("{hash:#x}")),
                        Style::default(),
                    ),
                    Span::styled(
                        verify_tx_hash
                            .map_or_else(|| "Unverified".to_string(), |hash| format!("{hash:#x}")),
                        Style::default(),
                    ),
                ])
            },
        );
        let committed_batches_table = Table::new(rows, constraints)
            .header(
                Row::new(vec![
                    "Number",
                    "# Blocks",
                    "# L2 to L1 Messages",
                    "Commit Tx Hash",
                    "Verify Tx Hash",
                ])
                .style(Style::default()),
            )
            .block(
                Block::bordered()
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(Span::styled(
                        "L2 Batches",
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            );

        committed_batches_table.render(area, buf, state);
    }
}
