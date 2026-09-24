/// Standalone binary to seed ~150M old-format (RLP-encoded) RECEIPTS keys
/// into an existing RocksDB database for migration benchmarking.
///
/// Usage: seed_migration_test <db_path>
///
/// This opens the database, writes 150M entries with RLP-encoded (H256, u64)
/// keys and small synthetic receipt values into the "receipts" column family,
/// then exits. After running, reset metadata.json to {"schema_version": 1}
/// and start ethrex to trigger the migration.
use ethrex_storage::api::tables::{RECEIPTS, TABLES};
use rocksdb::{
    BlockBasedOptions, Cache, ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded, Options,
    WriteBatch,
};
use std::collections::HashSet;
use std::time::Instant;

/// RLP-encode a (H256, u64) tuple the same way ethrex_rlp does.
/// Layout: RLP list header + 32-byte hash (with RLP string header) + u64 (with RLP string header)
fn rlp_encode_receipt_key(block_hash: &[u8; 32], index: u64) -> Vec<u8> {
    // RLP-encode the H256 (32 bytes): 0xa0 prefix + 32 bytes = 33 bytes
    // RLP-encode the u64: variable length
    // Then wrap in a list

    let mut hash_encoded = Vec::with_capacity(33);
    hash_encoded.push(0x80 + 32); // string header for 32 bytes
    hash_encoded.extend_from_slice(block_hash);

    let idx_encoded = rlp_encode_u64(index);

    let payload_len = hash_encoded.len() + idx_encoded.len();
    let mut out = Vec::with_capacity(payload_len + 3);

    // List header
    if payload_len < 56 {
        out.push(0xc0 + payload_len as u8);
    } else {
        let len_bytes = minimal_be_bytes(payload_len as u64);
        out.push(0xf7 + len_bytes.len() as u8);
        out.extend_from_slice(&len_bytes);
    }
    out.extend_from_slice(&hash_encoded);
    out.extend_from_slice(&idx_encoded);
    out
}

fn rlp_encode_u64(val: u64) -> Vec<u8> {
    if val == 0 {
        return vec![0x80]; // empty string = 0
    }
    if val < 128 {
        return vec![val as u8]; // single byte
    }
    let bytes = minimal_be_bytes(val);
    let mut out = Vec::with_capacity(1 + bytes.len());
    out.push(0x80 + bytes.len() as u8);
    out.extend_from_slice(&bytes);
    out
}

fn minimal_be_bytes(val: u64) -> Vec<u8> {
    let bytes = val.to_be_bytes();
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(7);
    bytes[start..].to_vec()
}

/// Create a minimal synthetic receipt value (RLP-encoded).
/// Receipt: [tx_type(0), succeeded(true), cumulative_gas(21000), bloom(256 zeros), logs(empty)]
fn synthetic_receipt_value() -> Vec<u8> {
    // A minimal Legacy receipt: RLP([1, cumgas, bloom, []])
    // succeeded = 0x01 (single byte)
    // cumulative_gas_used = 21000 = 0x5208
    // bloom = 256 zero bytes
    // logs = empty list

    let succeeded = vec![0x01]; // RLP single byte
    let cumgas = vec![0x82, 0x52, 0x08]; // RLP string: 2 bytes, 0x5208
    // bloom: 256 zero bytes -> string header 0xb9 0x01 0x00 + 256 zeros
    let mut bloom = Vec::with_capacity(259);
    bloom.push(0xb9);
    bloom.push(0x01);
    bloom.push(0x00);
    bloom.extend_from_slice(&[0u8; 256]);
    let logs = vec![0xc0]; // empty list

    let payload_len = succeeded.len() + cumgas.len() + bloom.len() + logs.len();
    let mut out = Vec::with_capacity(payload_len + 4);

    // List header for the receipt
    if payload_len < 56 {
        out.push(0xc0 + payload_len as u8);
    } else {
        let len_bytes = minimal_be_bytes(payload_len as u64);
        out.push(0xf7 + len_bytes.len() as u8);
        out.extend_from_slice(&len_bytes);
    }
    out.extend_from_slice(&succeeded);
    out.extend_from_slice(&cumgas);
    out.extend_from_slice(&bloom);
    out.extend_from_slice(&logs);
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <db_path>", args[0]);
        std::process::exit(1);
    }
    let db_path = &args[1];

    println!("Opening database at: {db_path}");

    // DB options matching ethrex's RocksDBBackend::open()
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    opts.set_max_open_files(512);
    opts.set_max_file_opening_threads(16);
    opts.set_max_background_jobs(8);
    opts.set_compression_type(rocksdb::DBCompressionType::None);

    let block_cache = Cache::new_lru_cache(2 * 1024 * 1024 * 1024); // 2GB for seeding

    // Build CF list from the crate's TABLES constant, plus the legacy RECEIPTS CF
    // that we need to seed old-format entries into.
    let existing_cfs = DBWithThreadMode::<MultiThreaded>::list_cf(&opts, db_path)
        .unwrap_or_else(|_| vec!["default".to_string()]);

    let mut all_cfs: HashSet<String> = existing_cfs.into_iter().collect();
    all_cfs.extend(TABLES.iter().map(|t| t.to_string()));
    all_cfs.insert(RECEIPTS.to_string());
    all_cfs.insert("default".to_string());

    let cf_descriptors: Vec<ColumnFamilyDescriptor> = all_cfs
        .iter()
        .map(|cf_name| {
            let mut cf_opts = Options::default();
            cf_opts.set_write_buffer_size(128 * 1024 * 1024);
            cf_opts.set_max_write_buffer_number(3);
            cf_opts.set_target_file_size_base(256 * 1024 * 1024);

            let mut block_opts = BlockBasedOptions::default();
            block_opts.set_block_size(32 * 1024);
            block_opts.set_block_cache(&block_cache);
            cf_opts.set_block_based_table_factory(&block_opts);

            ColumnFamilyDescriptor::new(cf_name.clone(), cf_opts)
        })
        .collect();

    let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(&opts, db_path, cf_descriptors)
        .expect("Failed to open database");

    let cf = db
        .cf_handle("receipts")
        .expect("receipts column family not found");

    let receipt_value = synthetic_receipt_value();
    println!("Synthetic receipt value: {} bytes", receipt_value.len());

    const TOTAL: u64 = 150_000_000;
    const BATCH_SIZE: u64 = 50_000;
    const RECEIPTS_PER_BLOCK: u64 = 256;

    let start = Instant::now();
    let mut batch = WriteBatch::default();
    let mut count: u64 = 0;

    println!("Seeding {TOTAL} old-format RLP RECEIPTS entries...");

    for i in 0..TOTAL {
        // Generate a deterministic "block hash" from the block index
        let block_idx = i / RECEIPTS_PER_BLOCK;
        let receipt_idx = i % RECEIPTS_PER_BLOCK;
        let mut block_hash = [0u8; 32];
        // Use a prefix that won't collide with real block hashes (starts with 0xFF)
        block_hash[0] = 0xFF;
        block_hash[1] = 0xFE;
        // Encode block_idx into bytes 24..31
        block_hash[24..32].copy_from_slice(&block_idx.to_be_bytes());

        let key = rlp_encode_receipt_key(&block_hash, receipt_idx);
        batch.put_cf(&cf, &key, &receipt_value);
        count += 1;

        if count.is_multiple_of(BATCH_SIZE) {
            db.write(batch).expect("Failed to write batch");
            batch = WriteBatch::default();

            if count.is_multiple_of(5_000_000) {
                let elapsed = start.elapsed().as_secs_f64();
                let rate = count as f64 / elapsed;
                println!(
                    "  {count}/{TOTAL} ({:.1}%) — {:.0} entries/sec — {:.1}s elapsed",
                    count as f64 / TOTAL as f64 * 100.0,
                    rate,
                    elapsed
                );
            }
        }
    }

    // Final batch
    if !count.is_multiple_of(BATCH_SIZE) {
        db.write(batch).expect("Failed to write final batch");
    }

    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "Done! Seeded {count} entries in {elapsed:.1}s ({:.0} entries/sec)",
        count as f64 / elapsed
    );
    println!("Now reset metadata.json to {{\"schema_version\": 1}} and start ethrex.");
    println!("Migration will copy entries from 'receipts' to 'receipts_v2' (two-CF approach).");
    println!(
        "The old 'receipts' CF will be dropped automatically on the next startup after migration."
    );
}
