use ethrex_rpc::EthClient;
use ratatui::widgets::TableState;

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
