//! Minimal replay binary for snap sync offline profiling.
//! Usage: cargo run --example snap_profile_replay --no-default-features --features c-kzg -- <dataset_path>

#[cfg(not(feature = "rocksdb"))]
#[tokio::main]
async fn main() {
    use ethrex_p2p::sync::profile;
    use std::path::PathBuf;

    // tracing logs from the profile module will go to stdout via println below

    let dataset_path = std::env::args()
        .nth(1)
        .expect("Usage: snap_profile_replay <dataset_path>");
    let dataset_root = PathBuf::from(&dataset_path);

    println!("Loading manifest from {dataset_path}...");
    let manifest = profile::load_manifest(&dataset_root).expect("Failed to load manifest");
    println!("Manifest loaded:");
    println!("  chain_id: {}", manifest.chain_id);
    println!("  pivot block: {}", manifest.pivot.number);
    println!("  pivot state_root: {:?}", manifest.pivot.state_root);
    println!(
        "  expected post-insert root: {:?}",
        manifest.post_accounts_insert_state_root
    );
    println!("  rocksdb_enabled: {}", manifest.rocksdb_enabled);
    println!();

    println!("Starting replay...");
    let result = profile::run_once(&dataset_root)
        .await
        .expect("Replay failed");

    println!();
    println!("=== Replay Results ===");
    println!(
        "InsertAccounts: {:.2}s",
        result.insert_accounts_duration.as_secs_f64()
    );
    println!(
        "InsertStorages: {:.2}s",
        result.insert_storages_duration.as_secs_f64()
    );
    println!(
        "Total:          {:.2}s",
        result.total_duration.as_secs_f64()
    );
    println!("Computed state root: {:?}", result.computed_state_root);

    let expected = manifest.post_accounts_insert_state_root;
    if result.computed_state_root == expected {
        println!("State root: MATCH");
    } else {
        eprintln!(
            "State root: MISMATCH! Expected {:?}, got {:?}",
            expected, result.computed_state_root
        );
        std::process::exit(1);
    }
}

#[cfg(feature = "rocksdb")]
fn main() {
    eprintln!("This example requires --no-default-features --features c-kzg (no rocksdb)");
    std::process::exit(1);
}
