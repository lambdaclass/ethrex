use ethrex_common::types::{ChainConfig, Genesis};
use tracing::info;

pub struct InfoBox {
    title: String,
    items: Vec<String>,
    width: usize,
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

fn split_into_chunks(s: &str, chunk: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    if chunk == 0 {
        return chunks;
    }
    let mut it = s.chars();
    loop {
        let part: String = it.by_ref().take(chunk).collect();
        if part.is_empty() {
            break;
        }
        chunks.push(part);
    }
    chunks
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
        self.width = width.max(60);
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

        let inner_width = self.width;
        let border = "═".repeat(inner_width);
        let title_len = char_len(&self.title);
        let available_width = inner_width.saturating_sub(4);

        let title_line = if title_len <= available_width {
            let title_padding = (inner_width.saturating_sub(title_len + 2)) / 2;
            let right_padding = inner_width.saturating_sub(title_padding + title_len + 2);
            format!(
                "{} {} {}",
                " ".repeat(title_padding),
                self.title,
                " ".repeat(right_padding)
            )
        } else {
            let mut t = self.title.chars().take(available_width).collect::<String>();
            t.push('…');
            let title_padding = 1;
            let right_padding = inner_width.saturating_sub(title_padding + char_len(&t) + 1);
            format!(
                "{}{}{}",
                " ".repeat(title_padding),
                t,
                " ".repeat(right_padding)
            )
        };

        info!("╔{border}╗");
        info!("║{title_line}║");
        info!("╠{border}╣");

        for (i, item) in self.items.iter().enumerate() {
            if i > 0 {
                info!("║{}║", " ".repeat(inner_width));
            }

            for line in item.lines() {
                let content_with_space = format!(" {line}");
                let content_len = char_len(&content_with_space);

                if content_len <= inner_width {
                    let padding_needed = inner_width.saturating_sub(content_len);
                    let display_line =
                        format!("{content_with_space}{}", " ".repeat(padding_needed));
                    info!("║{display_line}║");
                } else {
                    let words: Vec<&str> = line.split_whitespace().collect();
                    let mut current_line = String::new();

                    for word in words {
                        let mut word_with_space = format!(" {word}");
                        let curr_len = char_len(&current_line);
                        let word_len = char_len(&word_with_space);

                        if curr_len + word_len <= inner_width {
                            current_line.push_str(&word_with_space);
                            continue;
                        }

                        if curr_len > 0 {
                            let padding = inner_width.saturating_sub(curr_len);
                            info!("║{current_line}{}║", " ".repeat(padding));
                            current_line.clear();
                        }

                        let chunk_cap = inner_width.saturating_sub(1);
                        if word_len <= inner_width {
                            current_line = format!(" {word}");
                            continue;
                        }

                        let raw = word.to_string();
                        for chunk in split_into_chunks(&raw, chunk_cap) {
                            let mut line_to_print = String::from(" ");
                            line_to_print.push_str(&chunk);
                            let l = char_len(&line_to_print);
                            if l < inner_width {
                                let pad = inner_width - l;
                                info!("║{line_to_print}{}║", " ".repeat(pad));
                            } else {
                                info!("║{line_to_print}║");
                            }
                        }
                        current_line.clear();
                        word_with_space.clear();
                    }

                    if !current_line.is_empty() {
                        let padding = inner_width.saturating_sub(char_len(&current_line));
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
