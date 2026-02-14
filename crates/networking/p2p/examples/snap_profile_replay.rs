//! Replay binary for snap sync offline profiling.
//!
//! Usage:
//!   # With rocksdb (default when feature is enabled):
//!   cargo run --release --example snap_profile_replay -p ethrex-p2p --features rocksdb,c-kzg -- <dataset_path>
//!
//!   # With in-memory backend (pure-compute micro-bench, needs enough RAM):
//!   cargo run --release --example snap_profile_replay -p ethrex-p2p --features c-kzg -- <dataset_path> --backend inmemory

use std::path::PathBuf;

use clap::Parser;
use ethrex_p2p::sync::profile::{self, ProfileBackend};

#[derive(Parser)]
#[command(about = "Replay snap sync phases from a captured dataset for offline profiling")]
struct Args {
    /// Path to the captured dataset directory (containing manifest.json)
    dataset_path: PathBuf,

    /// Storage backend: "rocksdb" (disk-backed, default) or "inmemory" (all state in RAM)
    #[arg(long, default_value_t = default_backend_name())]
    backend: String,

    /// Directory for RocksDB data. If omitted, a temporary directory is used.
    #[arg(long)]
    db_dir: Option<PathBuf>,

    /// Don't clean up the RocksDB directory after the run.
    #[arg(long)]
    keep_db: bool,
}

fn default_backend_name() -> String {
    if cfg!(feature = "rocksdb") {
        "rocksdb".to_string()
    } else {
        "inmemory".to_string()
    }
}

fn parse_backend(name: &str) -> Result<ProfileBackend, String> {
    match name {
        "inmemory" => Ok(ProfileBackend::InMemory),
        #[cfg(feature = "rocksdb")]
        "rocksdb" => Ok(ProfileBackend::RocksDb),
        #[cfg(not(feature = "rocksdb"))]
        "rocksdb" => Err(
            "rocksdb backend requested but ethrex-p2p was compiled without the rocksdb feature. \
             Rebuild with --features rocksdb,c-kzg"
                .to_string(),
        ),
        other => Err(format!("unknown backend: {other} (expected: inmemory, rocksdb)")),
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let backend = parse_backend(&args.backend).unwrap_or_else(|e| {
        eprintln!("Error: {e}");
        std::process::exit(1);
    });

    println!("Loading manifest from {}...", args.dataset_path.display());
    let manifest = profile::load_manifest(&args.dataset_path).unwrap_or_else(|e| {
        eprintln!("Failed to load manifest: {e}");
        std::process::exit(1);
    });
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

    // Determine the working directory for the store.
    let (db_dir, _temp_dir) = match backend {
        ProfileBackend::InMemory => (PathBuf::from("."), None::<tempfile::TempDir>),
        #[cfg(feature = "rocksdb")]
        ProfileBackend::RocksDb => {
            if let Some(ref dir) = args.db_dir {
                std::fs::create_dir_all(dir).unwrap_or_else(|e| {
                    eprintln!("Failed to create db dir {}: {e}", dir.display());
                    std::process::exit(1);
                });
                (dir.clone(), None)
            } else {
                let tmp = tempfile::TempDir::new().unwrap_or_else(|e| {
                    eprintln!("Failed to create temp dir: {e}");
                    std::process::exit(1);
                });
                let path = tmp.path().to_path_buf();
                (path, Some(tmp))
            }
        }
    };

    println!("Starting replay with backend: {backend}");
    if matches!(backend, ProfileBackend::InMemory) {
        println!("  (db_dir ignored for inmemory backend)");
    } else {
        println!("  db_dir: {}", db_dir.display());
    }
    println!();

    let result =
        profile::run_once_with_opts(&args.dataset_path, backend, &db_dir)
            .await
            .unwrap_or_else(|e| {
                eprintln!("Replay failed: {e}");
                std::process::exit(1);
            });

    println!();
    println!("=== Replay Results ===");
    println!("Backend: {}", result.backend);
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

    // Handle cleanup for rocksdb backend.
    #[cfg(feature = "rocksdb")]
    if matches!(backend, ProfileBackend::RocksDb) {
        if args.keep_db {
            // Persist the temp dir so it's not cleaned up on drop.
            if let Some(tmp) = _temp_dir {
                let kept = tmp.keep();
                println!("DB kept at: {}", kept.display());
            } else {
                println!("DB kept at: {}", db_dir.display());
            }
        } else if _temp_dir.is_none() && args.db_dir.is_some() {
            // User provided explicit --db-dir without --keep-db: clean it up.
            let _ = std::fs::remove_dir_all(&db_dir);
            println!("DB cleaned up: {}", db_dir.display());
        }
        // If _temp_dir is Some and keep_db is false, TempDir drops and cleans up automatically.
    }
}
