use std::path::{Path, PathBuf};

use serde::Deserialize;

const CONFIG_PATH: &str = "config.toml";
const CONSTANTS_PATH: &str = "constants.rs";

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct P2PConfig {
    sync_config: SyncConfig,
}

#[derive(Deserialize)]
#[serde(rename_all = "UPPERCASE")]
struct SyncConfig {
    batch_size: usize,
    node_batch_size: usize,
    max_parallel_fetches: usize,
    max_channel_messages: usize,
    max_channel_reads: usize,
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=config.toml");

    let config_data: P2PConfig = toml::from_str(
        &std::fs::read_to_string(Path::new(CONFIG_PATH)).expect("Failed to open p2p config file"),
    )
    .expect("Failed to read p2p config file");
    let out_dir = std::env::var_os("OUT_DIR").unwrap();
    let destination_path = std::path::Path::new(&out_dir).join(CONSTANTS_PATH);
    config_data.write_to_destination(destination_path);
}

impl P2PConfig {
    fn write_to_destination(&self, destination_path: PathBuf) {
        std::fs::write(
            destination_path,
            format!(
                "// Sync constants
                pub(crate) const BATCH_SIZE: usize = {};
                pub(crate)const NODE_BATCH_SIZE: usize = {};
                pub(crate)const MAX_PARALLEL_FETCHES: usize = {};
                pub(crate)const MAX_CHANNEL_MESSAGES: usize = {};
                pub(crate)const MAX_CHANNEL_READS: usize = {};",
                self.sync_config.batch_size,
                self.sync_config.node_batch_size,
                self.sync_config.max_parallel_fetches,
                self.sync_config.max_channel_messages,
                self.sync_config.max_channel_reads,
            ),
        )
        .unwrap()
    }
}
