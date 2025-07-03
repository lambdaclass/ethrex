use ethrex_rpc::EthClient;
use ratatui::widgets::TableState;

pub struct NodeStatusTable {
    pub state: TableState,
    pub items: [(String, String); 5],
}

impl NodeStatusTable {
    pub async fn new(rollup_client: &EthClient) -> Self {
        Self {
            state: TableState::default(),
            items: Self::refresh_items(rollup_client).await,
        }
    }

    pub async fn on_tick(&mut self, rollup_client: &EthClient) {
        self.items = Self::refresh_items(rollup_client).await;
    }

    async fn refresh_items(rollup_client: &EthClient) -> [(String, String); 5] {
        let last_update = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let status = rollup_client
            .node_status()
            .await
            .expect("Failed to get node status");
        let last_known_batch = "NaN"; // TODO: Implement last known batch retrieval
        let last_known_block = rollup_client
            .get_block_number()
            .await
            .expect("Failed to get latest known L2 block");
        let follower_nodes = "NaN"; // TODO: Implement follower nodes retrieval

        [
            ("Last Update:".to_string(), last_update),
            ("Status:".to_string(), status.to_string()),
            (
                "Last Known Batch:".to_string(),
                last_known_batch.to_string(),
            ),
            (
                "Last Known Block:".to_string(),
                last_known_block.to_string(),
            ),
            ("Peers:".to_string(), follower_nodes.to_string()),
        ]
    }
}
