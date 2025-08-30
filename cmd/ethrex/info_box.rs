use ethrex_common::types::{ChainConfig, Genesis};
use tracing::info;

pub struct InfoBox {
    title: String,
    items: Vec<String>,
    width: usize,
}

impl InfoBox {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            items: Vec::new(),
            width: 70,
        }
    }

    pub fn with_width(mut self, width: usize) -> Self {
        self.width = width.max(60);
        self
    }

    pub fn add_item(mut self, line: impl Into<String>) -> Self {
        self.items.push(line.into());
        self
    }

    pub fn add_chain_config(self, config: &ChainConfig) -> Self {
        self.add_item(config.display_config().trim_end())
    }

    pub fn add_genesis_hash(self, genesis: &Genesis) -> Self {
        let hash = genesis.get_block().hash();
        self.add_item(format!("Genesis Block Hash: {:x}", hash))
    }

    pub fn display(&self) {
        let border = "═".repeat(self.width);
        info!("{}", border);
        info!("{}", self.title);
        info!("{}", border);
        for item in &self.items {
            for line in item.lines() {
                info!("{}", line);
            }
        }
        info!("{}", border);
    }
}

pub fn display_chain_initialization(genesis: &Genesis) {
    InfoBox::new("NETWORK CONFIGURATION")
        .add_chain_config(&genesis.config)
        .add_genesis_hash(genesis)
        .display();
}
