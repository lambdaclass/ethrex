use ethrex::utils::default_datadir;
use ethrex_p2p::sync::build_snapshot;
use ethrex_storage::{EngineType, Store};

fn main() {
    // Load already synced store
    let store = Store::new(default_datadir(), EngineType::RocksDB).expect("failed to create store");

    // Retrieve pivot block header (pivot should be the last executed block).
    let pivot_header = store
        .get_block_header(1375008)
        .expect("failed to get pivot header")
        .expect("pivot header not found in store");

    build_snapshot(pivot_header.state_root, &store).expect("failed to build snapshot");
}
