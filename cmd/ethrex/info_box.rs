use ethrex_common::types::{ChainConfig, Genesis};
use tracing::info;

pub struct InfoBox {
    title: String,
    items: Vec<String>,
    width: usize,
}

impl InfoBox {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_string(),
            items: Vec::new(),
            width: 90,
        }
    }

    pub fn with_width(mut self, width: usize) -> Self {
        self.width = width;
        self
    }

    pub fn add_chain_config(mut self, config: &ChainConfig) -> Self {
        let config_display = config.display_config().trim_end().to_string();
        self.items.push(config_display);
        self
    }

    pub fn add_genesis_hash(mut self, genesis: &Genesis) -> Self {
        let hash = genesis.get_block().hash();
        self.items.push(format!("Genesis Block Hash: {:#x}", hash));
        self
    }

    pub fn add_custom(mut self, key: &str, value: &str) -> Self {
        self.items.push(format!("{}: {}", key, value));
        self
    }

    pub fn display(self) {
        if self.items.is_empty() {
            return;
        }

        let border = "═".repeat(self.width);
        let title_padding = (self.width.saturating_sub(self.title.len() + 2)) / 2;
        let title_line = format!(
            "{} {} {}",
            " ".repeat(title_padding),
            self.title,
            " ".repeat(
                self.width
                    .saturating_sub(title_padding + self.title.len() + 2)
            )
        );

        info!("╔{}╗", border);
        info!("║{}║", title_line);
        info!("╠{}╣", border);

        for (i, item) in self.items.iter().enumerate() {
            if i > 0 {
                info!("║{}║", " ".repeat(self.width));
            }

            for line in item.lines() {
                let content = format!(" {}", line);
                let padding_needed = self.width.saturating_sub(content.len().min(self.width));

                let display_line = if content.len() > self.width {
                    format!(" {}...", &line[..self.width.saturating_sub(5)])
                } else {
                    format!("{}{}", content, " ".repeat(padding_needed))
                };

                info!("║{}║", display_line);
            }
        }

        info!("╚{}╝", border);
    }
}

pub fn init_info_box() -> InfoBox {
    InfoBox::new("NETWORK CONFIGURATION")
}

pub fn display_chain_initialization(genesis: &Genesis) {
    init_info_box()
        .add_chain_config(&genesis.config)
        .add_genesis_hash(genesis)
        .display();
}
