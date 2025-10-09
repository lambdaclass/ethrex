use std::time::{Duration, Instant};

use ethrex::utils::default_datadir;
use ethrex_storage::{EngineType, Store};

#[tokio::main]
async fn main() {
    // Load already synced store
    let store = Store::new(default_datadir(), EngineType::RocksDB).expect("failed to create store");

    // Retrieve pivot block header (pivot should be the last executed block).
    let pivot_header = store
        .get_block_header(1375008)
        .expect("failed to get pivot header")
        .expect("pivot header not found in store");

    println!("Building snapshot...");

    let start = Instant::now();
    store
        .generate_snapshot(pivot_header.state_root)
        .await
        .expect("failed to build snapshot");
    let elapsed = start.elapsed();

    println!("Snapshot built in: {}", format_duration(elapsed))
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    let milliseconds = duration.subsec_millis();

    if hours > 0 {
        return format!("{hours:02}h {minutes:02}m {seconds:02}s {milliseconds:03}ms");
    }

    if minutes == 0 {
        return format!("{seconds:02}s {milliseconds:03}ms");
    }

    format!("{minutes:02}m {seconds:02}s")
}
