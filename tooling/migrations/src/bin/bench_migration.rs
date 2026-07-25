/// Standalone binary to benchmark the v1→v2 RECEIPTS migration.
///
/// Usage: bench_migration <db_path>
///
/// Opens the RocksDB database, runs the two-CF migration (receipts → receipts_v2),
/// and reports wall-clock time and peak RSS.
///
/// Prerequisites:
///   1. Run `seed_migration_test <db_path>` to seed 150M old-format entries
///   2. Ensure metadata.json has {"schema_version": 1} (or just don't create one)
///   3. Run this binary
use std::time::Instant;

fn get_rss_mb() -> Option<f64> {
    #[cfg(target_os = "macos")]
    {
        use std::mem;
        let mut info: libc::rusage = unsafe { mem::zeroed() };
        let ret = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut info) };
        if ret == 0 {
            // macOS reports maxrss in bytes
            Some(info.ru_maxrss as f64 / (1024.0 * 1024.0))
        } else {
            None
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::mem;
        let mut info: libc::rusage = unsafe { mem::zeroed() };
        let ret = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut info) };
        if ret == 0 {
            // Linux reports maxrss in kilobytes
            Some(info.ru_maxrss as f64 / 1024.0)
        } else {
            None
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

fn main() {
    // Initialize tracing so migration progress logs are visible
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <db_path>", args[0]);
        std::process::exit(1);
    }
    let db_path = &args[1];

    println!("Opening database at: {db_path}");
    let rss_before = get_rss_mb();

    let backend = ethrex_storage::backend::rocksdb::RocksDBBackend::open(
        db_path,
        ethrex_storage::backend::rocksdb::RocksDbOpenOpts::new(
            ethrex_storage::DEFAULT_ROCKSDB_BLOCK_CACHE_SIZE_BYTES,
        ),
    )
    .expect("Failed to open RocksDB");

    let rss_after_open = get_rss_mb();
    println!(
        "Database opened. RSS after open: {:.1} MB",
        rss_after_open.unwrap_or(0.0)
    );

    // Run the migration
    println!("Starting migration v1→v2 (two-CF: receipts → receipts_v2)...");
    let start = Instant::now();

    ethrex_storage::migrations::run_pending_migrations(
        &backend,
        std::path::Path::new(db_path),
        1, // pretend we're at v1
    )
    .expect("Migration failed");

    let elapsed = start.elapsed();
    let rss_after = get_rss_mb();

    println!("\n=== Migration Benchmark Results ===");
    println!("Wall-clock time: {:.1}s", elapsed.as_secs_f64());
    if let Some(before) = rss_before {
        println!("RSS before open:     {:.1} MB", before);
    }
    if let Some(after_open) = rss_after_open {
        println!("RSS after open:      {:.1} MB", after_open);
    }
    if let Some(after) = rss_after {
        println!("Peak RSS (maxrss):   {:.1} MB", after);
    }
    println!("===================================");
}
