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
        self.width = width.max(20);
        self
    }

    pub fn add_chain_config(mut self, config: &ChainConfig) -> Self {
        let config_display = config.display_config().trim_end().to_string();
        self.items.push(config_display);
        self
    }

    pub fn add_genesis_hash(mut self, genesis: &Genesis) -> Self {
        let hash = genesis.get_block().hash();
        self.items.push(format!("Genesis Block Hash: {hash:#x}"));
        self
    }

    pub fn add_custom(mut self, key: &str, value: &str) -> Self {
        self.items.push(format!("{key}: {value}"));
        self
    }

    pub fn display(self) {
        if self.items.is_empty() {
            return;
        }

        let border = "═".repeat(self.width);
        let title_len = self.title.len();
        let available_width = self.width.saturating_sub(4);

        let title_line = if title_len <= available_width {
            let title_padding = (self.width.saturating_sub(title_len + 2)) / 2;
            let right_padding = self.width.saturating_sub(title_padding + title_len + 2);
            format!(
                "{} {} {}",
                " ".repeat(title_padding),
                self.title,
                " ".repeat(right_padding)
            )
        } else {
            format!(" {} ", self.title)
        };

        info!("╔{border}╗");
        info!("║{title_line}║");
        info!("╠{border}╣");

        for (i, item) in self.items.iter().enumerate() {
            if i > 0 {
                info!("║{}║", " ".repeat(self.width));
            }

            for line in item.lines() {
                let content_with_space = format!(" {line}");
                let content_len = content_with_space.len();
                let inner_width = self.width.saturating_sub(2);

                if content_len <= inner_width {
                    let padding_needed = inner_width.saturating_sub(content_len);
                    let display_line =
                        format!("{content_with_space}{}", " ".repeat(padding_needed));
                    info!("║{display_line}║");
                } else {
                    let available_content_width = inner_width.saturating_sub(1);
                    let words: Vec<&str> = line.split_whitespace().collect();
                    let mut current_line = String::new();

                    for word in words {
                        let word_with_space = if current_line.is_empty() {
                            format!(" {word}")
                        } else {
                            format!(" {word}")
                        };

                        if (current_line.len() + word_with_space.len()) <= available_content_width {
                            current_line.push_str(&word_with_space);
                        } else {
                            if !current_line.is_empty() {
                                let padding = inner_width.saturating_sub(current_line.len());
                                info!("║{current_line}{}║", " ".repeat(padding));
                            }
                            current_line = format!(" {word}");
                        }
                    }

                    if !current_line.is_empty() {
                        let padding = inner_width.saturating_sub(current_line.len());
                        info!("║{current_line}{}║", " ".repeat(padding));
                    }
                }
            }
        }

        info!("╚{border}╝");
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
