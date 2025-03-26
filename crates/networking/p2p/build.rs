use std::path::PathBuf;

use serde::Deserialize;

const CONFIG_PATH: &str = "config.json";
const CONSTANTS_PATH: &str = "constants.rs";

#[derive(Deserialize)]
#[serde(rename_all = "UPPERCASE")]
struct P2PConfig {
    sync_config: SyncConfig,
}

#[derive(Deserialize)]
#[serde(rename_all = "UPPERCASE")]
struct SyncConfig {
    max_parallel_fetches: usize,
    batch_size: usize,
    node_batch_size: usize,
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=config.json");

    let config_data: P2PConfig = serde_json::from_reader(
        std::fs::File::open(CONFIG_PATH).expect("Failed to open p2p config file"),
    )
    .expect("Failed to read p2p config file");
    let out_dir = std::env::var_os("OUT_DIR").unwrap();
    let destination_path = std::path::Path::new(&out_dir).join(CONSTANTS_PATH);
    config_data.write_to_destination(destination_path);
    // std::fs::write(&path, "pub fn test() { todo!() }").unwrap();
}

impl P2PConfig {
    fn write_to_destination(&self, destination_path: PathBuf) {
        std::fs::write(
            destination_path,
            format!(
                "pub const MAX_PARALLEL_FETCHES: usize = {};
            pub const BATCH_SIZE: usize = {};
            pub const NODE_BATCH_SIZE: usize = {};",
                self.sync_config.max_parallel_fetches,
                self.sync_config.batch_size,
                self.sync_config.node_batch_size
            ),
        )
        .unwrap()
    }
}
