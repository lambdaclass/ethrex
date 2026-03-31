//! Compares memtable memory configuration between the standard and checkpoint RocksDB stores.
//!
//! Shows both the configured write buffer limits (maximum potential allocation) and the
//! actual current memtable sizes. RocksDB allocates memtables in small arena blocks that
//! grow toward the configured limit, so the real savings appear under production workload.
//!
//! Run with:
//! ```sh
//! cargo run -p ethrex-storage --example rocksdb_memory_comparison --features rocksdb
//! ```

#[cfg(feature = "rocksdb")]
fn main() {
    use ethrex_storage::backend::rocksdb::RocksDBBackend;

    let tmp_standard = tempfile::tempdir().expect("failed to create temp dir");
    let tmp_checkpoint = tempfile::tempdir().expect("failed to create temp dir");

    let standard = RocksDBBackend::open(tmp_standard.path()).expect("failed to open standard DB");
    let checkpoint = RocksDBBackend::open_checkpoint(tmp_checkpoint.path())
        .expect("failed to open checkpoint DB");

    let standard_bytes = standard.mem_table_total_size();
    let checkpoint_bytes = checkpoint.mem_table_total_size();

    let num_cfs = ethrex_storage::api::tables::TABLES.len() + 1; // +1 for "default"

    // Standard store configured limits (from open()):
    // - db_write_buffer_size: 1 GB global
    // - Per-CF write buffers range from 64 MB to 512 MB
    // - max_write_buffer_number ranges from 3 to 6
    // Theoretical max: sum of (write_buffer_size * max_write_buffer_number) per CF
    // Conservative estimate: ~1 GB global cap limits actual allocation
    let standard_configured_mb = 1024; // 1 GB db_write_buffer_size

    // Checkpoint store configured limits (from open_checkpoint()):
    // - db_write_buffer_size: 64 MB global
    // - Per-CF write buffers: 16 MB uniform
    // - max_write_buffer_number: 2 for all CFs
    let checkpoint_configured_mb = 64; // 64 MB db_write_buffer_size

    println!("=== Configured write buffer limits ===");
    println!("Standard store:   {standard_configured_mb:>6} MB  (db_write_buffer_size)");
    println!("Checkpoint store: {checkpoint_configured_mb:>6} MB  (db_write_buffer_size)");
    println!(
        "Reduction factor: {:.0}x",
        standard_configured_mb as f64 / checkpoint_configured_mb as f64
    );

    println!();
    println!("=== Current memtable allocation ({num_cfs} column families) ===");
    println!(
        "Standard store:   {standard_bytes:>12} bytes ({:.1} MB)",
        standard_bytes as f64 / 1024.0 / 1024.0
    );
    println!(
        "Checkpoint store: {checkpoint_bytes:>12} bytes ({:.1} MB)",
        checkpoint_bytes as f64 / 1024.0 / 1024.0
    );
    println!("(Memtables grow toward the configured limit under write pressure)");
}

#[cfg(not(feature = "rocksdb"))]
fn main() {
    eprintln!("This example requires the `rocksdb` feature. Run with:");
    eprintln!(
        "  cargo run -p ethrex-storage --example rocksdb_memory_comparison --features rocksdb"
    );
}
