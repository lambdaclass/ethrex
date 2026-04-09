use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};

use ethrex_binary_trie::key_mapping::{
    CODE_HASH_LEAF_KEY, get_stem_for_base, get_tree_key_for_storage_slot, pack_basic_data,
    tree_key_from_stem,
};
use ethrex_common::{Address, H256, U256, types::Genesis};
use memmap2::Mmap;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use tracing::{info, warn};

use crate::initializers::{init_binary_trie_state, init_store};

struct MigrateConfig {
    in_memory: bool,
    flush_interval: u64,
    preimage_file_size: u64,
}

/// Read available RAM from /proc/meminfo (Linux only).
fn available_ram_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in meminfo.lines() {
        if let Some(rest) = line.strip_prefix("MemAvailable:") {
            let kb_str = rest.trim().strip_suffix("kB")?.trim();
            let kb: u64 = kb_str.parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

/// Auto-tune migration settings based on available RAM and preimage file size.
fn auto_tune_config(preimage_file_size: u64) -> MigrateConfig {
    let available = available_ram_bytes().unwrap_or(0);
    let total_ram = {
        // Read MemTotal for headroom calculation.
        let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        meminfo
            .lines()
            .find_map(|line| {
                let rest = line.strip_prefix("MemTotal:")?;
                let kb: u64 = rest.trim().strip_suffix("kB")?.trim().parse().ok()?;
                Some(kb * 1024)
            })
            .unwrap_or(available)
    };

    // Reserve max(4 GB, 15% of total RAM) as headroom for OS + RocksDB + other.
    let headroom = (total_ram * 15 / 100).max(4 * GB);
    let usable = available.saturating_sub(headroom);

    // Estimated RAM for in-memory preimages: ~1.1x file size.
    let preimage_ram = preimage_file_size * 11 / 10;
    // Trie working set: dirty + warm tiers during migration.
    let trie_overhead = 6 * GB;

    let (in_memory, flush_interval) = if usable > preimage_ram + trie_overhead {
        // Plenty of room: fast mode, large flush interval.
        let remaining = usable - preimage_ram;
        let interval = if remaining > 10 * GB {
            20_000_000
        } else if remaining > 6 * GB {
            10_000_000
        } else {
            5_000_000
        };
        (true, interval)
    } else if usable > preimage_ram + 3 * GB {
        // Tight but doable: fast mode, conservative flush.
        (true, 2_000_000)
    } else {
        // Not enough for in-memory: mmap mode.
        let interval = if usable > 4 * GB {
            5_000_000
        } else {
            2_000_000
        };
        (false, interval)
    };

    info!(
        "Auto-tune: available RAM {} GB, headroom {} GB, usable {} GB, preimage estimate {} GB -> {} mode, flush every {}",
        available / GB,
        headroom / GB,
        usable / GB,
        preimage_ram / GB,
        if in_memory { "in-memory" } else { "mmap" },
        flush_interval,
    );

    MigrateConfig {
        in_memory,
        flush_interval,
        preimage_file_size,
    }
}

const GB: u64 = 1024 * 1024 * 1024;

// Geth's empty code hash: keccak256(b"")
const EMPTY_CODE_HASH: [u8; 32] = [
    0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
    0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
];

/// Migrates a Geth database to an ethrex binary trie using two export files.
///
/// - `preimage_path`: geth db export of preimages (keccak hash -> original key)
/// - `snapshot_path`: geth db export of snapshot (accounts + storage with state values)
///
/// No existing ethrex database needed. Creates the store and binary trie from scratch.
/// Memory usage and flush interval are auto-tuned based on available RAM.
pub async fn migrate_with_preimages(
    preimage_path: &str,
    snapshot_path: &str,
    datadir: &Path,
    genesis: Genesis,
    fast_override: bool,
) -> eyre::Result<()> {
    // Step 1: Open store and init binary trie state.
    let mut store = init_store(datadir, genesis.clone())
        .await
        .map_err(|e| eyre::eyre!("Failed to open store: {e}"))?;

    let binary_trie_state = init_binary_trie_state(&store, datadir, &genesis)?;
    store.set_binary_trie_state(binary_trie_state.clone());

    // Auto-tune based on available RAM.
    let preimage_file_size = std::fs::metadata(preimage_path)
        .map(|m| m.len())
        .unwrap_or(0);
    let config = if fast_override {
        // User explicitly requested fast mode.
        let flush_interval = 10_000_000;
        info!("Fast mode forced. Flush interval: {flush_interval}");
        MigrateConfig {
            in_memory: true,
            flush_interval,
            preimage_file_size,
        }
    } else {
        auto_tune_config(preimage_file_size)
    };

    // Step 2: Parse preimage file.
    let preimages = parse_geth_dump(preimage_path, datadir, config.in_memory)?;
    info!("{} unique preimages loaded", preimages.len());

    // Step 3: Stream the snapshot file and build the binary trie.
    info!("Streaming snapshot file: {snapshot_path}");
    let file =
        File::open(snapshot_path).map_err(|e| eyre::eyre!("Failed to open snapshot file: {e}"))?;
    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    info!("Snapshot file size: {} MB", file_size / 1024 / 1024);

    let mut reader = BufReader::with_capacity(8 * 1024 * 1024, file);
    skip_gethdbdump_header(&mut reader)?;

    let mut account_count = 0u64;
    let mut storage_count = 0u64;
    let mut skipped = 0u64;
    let mut bytes_read = 0u64;
    let started = std::time::Instant::now();
    let mut last_log = std::time::Instant::now();

    // Hold the write lock for the entire migration.
    let mut state = binary_trie_state
        .write()
        .map_err(|e| eyre::eyre!("Binary trie lock error: {e}"))?;

    let mut bufs = EntryBufs::new();
    let mut inserts_since_flush = 0u64;
    let flush_interval = config.flush_interval;
    const BATCH_SIZE: usize = 50_000;

    // Raw entries read from the file, to be processed in parallel.
    // Each entry: (entry_type, keccak_hashes, raw_value)
    let mut raw_batch: Vec<RawEntry> = Vec::with_capacity(BATCH_SIZE);

    loop {
        // Phase 1: Read a batch of raw entries (sequential I/O).
        raw_batch.clear();
        while raw_batch.len() < BATCH_SIZE {
            let Some((op, entry_bytes)) = read_gethdbdump_entry(&mut reader, &mut bufs)? else {
                break;
            };
            bytes_read += entry_bytes;
            if op != 0 {
                continue;
            }

            if bufs.key.len() == 33 && bufs.key[0] == b'a' {
                let mut keccak_addr = [0u8; 32];
                keccak_addr.copy_from_slice(&bufs.key[1..33]);
                raw_batch.push(RawEntry::Account {
                    keccak_addr,
                    slim_rlp: bufs.value.clone(),
                });
            } else if bufs.key.len() == 65 && bufs.key[0] == b'o' {
                let mut keccak_addr = [0u8; 32];
                keccak_addr.copy_from_slice(&bufs.key[1..33]);
                let mut keccak_slot = [0u8; 32];
                keccak_slot.copy_from_slice(&bufs.key[33..65]);
                raw_batch.push(RawEntry::Storage {
                    keccak_addr,
                    keccak_slot,
                    raw_value: bufs.value.clone(),
                });
            }
        }

        if raw_batch.is_empty() {
            break;
        }

        // Phase 2: Process batch in parallel (preimage lookups, RLP decode, BLAKE3).
        let processed: Vec<Option<ProcessedEntry>> = raw_batch
            .par_iter()
            .map(|entry| match entry {
                RawEntry::Account {
                    keccak_addr,
                    slim_rlp,
                } => {
                    let addr_bytes = preimages.get_addr(keccak_addr)?;
                    let address = Address::from_slice(&addr_bytes);
                    let (nonce, balance, code_hash) = decode_slim_account(slim_rlp).ok()?;
                    let code_size = 0u32;
                    let stem = get_stem_for_base(&address);
                    let basic_data_key = tree_key_from_stem(&stem, 0);
                    let basic_data = pack_basic_data(0, code_size, nonce, balance);
                    let code_hash_key = tree_key_from_stem(&stem, CODE_HASH_LEAF_KEY);
                    Some(ProcessedEntry::Account {
                        basic_data_key,
                        basic_data,
                        code_hash_key,
                        code_hash,
                    })
                }
                RawEntry::Storage {
                    keccak_addr,
                    keccak_slot,
                    raw_value,
                } => {
                    let addr_bytes = preimages.get_addr(keccak_addr)?;
                    let address = Address::from_slice(&addr_bytes);
                    let slot_bytes = preimages.get_slot(keccak_slot)?;
                    let storage_key = U256::from_big_endian(&slot_bytes);

                    let value_u256 = if raw_value.is_empty() {
                        U256::zero()
                    } else if let Ok((inner, _)) = decode_rlp_item(raw_value) {
                        if inner.is_empty() || inner.len() > 32 {
                            U256::zero()
                        } else {
                            U256::from_big_endian(inner)
                        }
                    } else {
                        U256::zero()
                    };

                    if value_u256.is_zero() {
                        return None;
                    }

                    let tree_key = get_tree_key_for_storage_slot(&address, storage_key);
                    let value_bytes = value_u256.to_big_endian();
                    Some(ProcessedEntry::Storage {
                        tree_key,
                        value_bytes,
                    })
                }
            })
            .collect();

        // Phase 3: Insert into trie (sequential).
        for (raw, processed) in raw_batch.iter().zip(processed.iter()) {
            match raw {
                RawEntry::Account { .. } => {
                    if let Some(ProcessedEntry::Account {
                        basic_data_key,
                        basic_data,
                        code_hash_key,
                        code_hash,
                    }) = processed
                    {
                        state
                            .trie_insert(*basic_data_key, *basic_data)
                            .map_err(|e| eyre::eyre!("Failed to insert basic_data: {e}"))?;
                        state
                            .trie_insert(*code_hash_key, *code_hash)
                            .map_err(|e| eyre::eyre!("Failed to insert code_hash: {e}"))?;
                        account_count += 1;
                        inserts_since_flush += 2;
                    } else {
                        skipped += 1;
                    }
                }
                RawEntry::Storage { .. } => {
                    if let Some(ProcessedEntry::Storage {
                        tree_key,
                        value_bytes,
                    }) = processed
                    {
                        state
                            .trie_insert(*tree_key, *value_bytes)
                            .map_err(|e| eyre::eyre!("Failed to insert storage: {e}"))?;
                        inserts_since_flush += 1;
                    }
                    storage_count += 1;
                }
            }
        }

        // Periodic flush.
        if inserts_since_flush >= flush_interval {
            info!("Flushing trie to disk ({inserts_since_flush} inserts)...");
            state
                .flush(0, H256::zero())
                .map_err(|e| eyre::eyre!("Flush error: {e}"))?;
            inserts_since_flush = 0;
        }

        // Log progress.
        if last_log.elapsed().as_secs() >= 5 {
            let pct = if file_size > 0 {
                (bytes_read as f64 / file_size as f64 * 100.0) as u32
            } else {
                0
            };
            let elapsed = started.elapsed().as_secs();
            let mb_per_sec = if elapsed > 0 {
                bytes_read / 1024 / 1024 / elapsed
            } else {
                0
            };
            info!(
                "{pct}% ({} MB / {} MB) | {account_count} accounts, {storage_count} storage | {mb_per_sec} MB/s",
                bytes_read / 1024 / 1024,
                file_size / 1024 / 1024,
            );
            last_log = std::time::Instant::now();
        }
    }

    drop(state);

    if skipped > 0 {
        warn!("{skipped} entries skipped due to missing preimages");
    }
    info!("Snapshot processing complete: {account_count} accounts, {storage_count} storage slots");

    // Step 4: Compute state root and flush.
    let state_root = {
        let mut state = binary_trie_state
            .write()
            .map_err(|e| eyre::eyre!("Binary trie lock error: {e}"))?;
        let root = state.state_root();
        H256::from(root)
    };
    info!("Binary trie state root: {state_root:#x}");

    {
        let mut state = binary_trie_state
            .write()
            .map_err(|e| eyre::eyre!("Binary trie lock error: {e}"))?;
        state
            .flush(0, H256::zero())
            .map_err(|e| eyre::eyre!("Failed to flush binary trie: {e}"))?;
    }
    info!("Binary trie flushed to disk. Migration complete.");

    Ok(())
}

/// Raw entry read from the snapshot dump (before parallel processing).
enum RawEntry {
    Account {
        keccak_addr: [u8; 32],
        slim_rlp: Vec<u8>,
    },
    Storage {
        keccak_addr: [u8; 32],
        keccak_slot: [u8; 32],
        raw_value: Vec<u8>,
    },
}

/// Processed entry ready for trie insertion (after parallel processing).
enum ProcessedEntry {
    Account {
        basic_data_key: [u8; 32],
        basic_data: [u8; 32],
        code_hash_key: [u8; 32],
        code_hash: [u8; 32],
    },
    Storage {
        tree_key: [u8; 32],
        value_bytes: [u8; 32],
    },
}

/// Decode a Geth "slim RLP" account: [nonce, balance, root?, codehash?]
///
/// Returns (nonce, balance, code_hash_bytes).
fn decode_slim_account(data: &[u8]) -> eyre::Result<(u64, U256, [u8; 32])> {
    // Slim RLP is an RLP list: [nonce, balance, root?, codehash?]
    // root and codehash are nil (empty bytes) when equal to empty defaults.
    let (list_data, _) = decode_rlp_list(data)?;

    let mut offset = 0;
    let (nonce_bytes, consumed) = decode_rlp_item(&list_data[offset..])?;
    let nonce = bytes_to_u64(nonce_bytes);
    offset += consumed;

    let (balance_bytes, consumed) = decode_rlp_item(&list_data[offset..])?;
    let balance = if balance_bytes.is_empty() {
        U256::zero()
    } else if balance_bytes.len() <= 32 {
        U256::from_big_endian(balance_bytes)
    } else {
        return Err(eyre::eyre!(
            "Balance too large: {} bytes",
            balance_bytes.len()
        ));
    };
    offset += consumed;

    // Root (optional, skip it -- we don't need storage_root)
    if offset < list_data.len() {
        let (_, consumed) = decode_rlp_item(&list_data[offset..])?;
        offset += consumed;
    }

    // CodeHash (optional)
    let code_hash = if offset < list_data.len() {
        let (ch_bytes, _) = decode_rlp_item(&list_data[offset..])?;
        if ch_bytes.is_empty() {
            EMPTY_CODE_HASH
        } else if ch_bytes.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(ch_bytes);
            arr
        } else {
            EMPTY_CODE_HASH
        }
    } else {
        EMPTY_CODE_HASH
    };

    Ok((nonce, balance, code_hash))
}

/// Decode an RLP list, returning the inner payload and total bytes consumed.
fn decode_rlp_list(data: &[u8]) -> eyre::Result<(&[u8], usize)> {
    if data.is_empty() {
        return Err(eyre::eyre!("Empty RLP data"));
    }
    let prefix = data[0];
    if prefix >= 0xf8 {
        let len_of_len = (prefix - 0xf7) as usize;
        if data.len() < 1 + len_of_len {
            return Err(eyre::eyre!("RLP list truncated"));
        }
        let mut len = 0usize;
        for &b in &data[1..1 + len_of_len] {
            len = (len << 8) | b as usize;
        }
        let start = 1 + len_of_len;
        if data.len() < start + len {
            return Err(eyre::eyre!("RLP list payload truncated"));
        }
        Ok((&data[start..start + len], start + len))
    } else if prefix >= 0xc0 {
        let len = (prefix - 0xc0) as usize;
        if data.len() < 1 + len {
            return Err(eyre::eyre!("RLP list payload truncated"));
        }
        Ok((&data[1..1 + len], 1 + len))
    } else {
        Err(eyre::eyre!("Expected RLP list, got 0x{prefix:02x}"))
    }
}

/// Decode a single RLP item (string/bytes), returning a reference to the data and bytes consumed.
fn decode_rlp_item(data: &[u8]) -> eyre::Result<(&[u8], usize)> {
    if data.is_empty() {
        return Err(eyre::eyre!("Empty RLP item"));
    }
    let prefix = data[0];
    if prefix < 0x80 {
        // Single byte
        Ok((&data[0..1], 1))
    } else if prefix <= 0xb7 {
        let len = (prefix - 0x80) as usize;
        if len == 0 {
            Ok((&[], 1))
        } else if data.len() < 1 + len {
            Err(eyre::eyre!("RLP string truncated"))
        } else {
            Ok((&data[1..1 + len], 1 + len))
        }
    } else if prefix <= 0xbf {
        let len_of_len = (prefix - 0xb7) as usize;
        if data.len() < 1 + len_of_len {
            return Err(eyre::eyre!("RLP long string header truncated"));
        }
        let mut len = 0usize;
        for &b in &data[1..1 + len_of_len] {
            len = (len << 8) | b as usize;
        }
        let start = 1 + len_of_len;
        if data.len() < start + len {
            return Err(eyre::eyre!("RLP long string truncated"));
        }
        Ok((&data[start..start + len], start + len))
    } else {
        Err(eyre::eyre!("Expected RLP string item, got 0x{prefix:02x}"))
    }
}

fn bytes_to_u64(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    // Single byte < 0x80 is itself
    let mut val = 0u64;
    for &b in bytes {
        val = (val << 8) | b as u64;
    }
    val
}

// ---------------------------------------------------------------------------
// gethdbdump format parsing (shared by preimage and snapshot files)
// ---------------------------------------------------------------------------

const ADDR_RECORD_SIZE: usize = 32 + 20; // 52
const SLOT_RECORD_SIZE: usize = 32 + 32; // 64

/// Preimage lookup: either in-memory HashMaps (fast, high RAM) or mmap binary search (slow, low RAM).
enum Preimages {
    /// In-memory: O(1) lookups, ~10 GB+ RAM for hoodi.
    InMemory {
        addrs: FxHashMap<[u8; 32], [u8; 20]>,
        slots: FxHashMap<[u8; 32], [u8; 32]>,
    },
    /// Mmap: O(log n) binary search, constant RAM.
    Mmap {
        addr_mmap: Mmap,
        addr_count: usize,
        slot_mmap: Mmap,
        slot_count: usize,
    },
}

impl Preimages {
    fn get_addr(&self, hash: &[u8; 32]) -> Option<[u8; 20]> {
        match self {
            Preimages::InMemory { addrs, .. } => addrs.get(hash).copied(),
            Preimages::Mmap {
                addr_mmap,
                addr_count,
                ..
            } => {
                Self::binary_search(addr_mmap, *addr_count, ADDR_RECORD_SIZE, hash).map(|offset| {
                    let mut arr = [0u8; 20];
                    arr.copy_from_slice(&addr_mmap[offset + 32..offset + 52]);
                    arr
                })
            }
        }
    }

    fn get_slot(&self, hash: &[u8; 32]) -> Option<[u8; 32]> {
        match self {
            Preimages::InMemory { slots, .. } => slots.get(hash).copied(),
            Preimages::Mmap {
                slot_mmap,
                slot_count,
                ..
            } => {
                Self::binary_search(slot_mmap, *slot_count, SLOT_RECORD_SIZE, hash).map(|offset| {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&slot_mmap[offset + 32..offset + 64]);
                    arr
                })
            }
        }
    }

    fn binary_search(
        mmap: &Mmap,
        count: usize,
        record_size: usize,
        hash: &[u8; 32],
    ) -> Option<usize> {
        if count == 0 {
            return None;
        }
        let mut lo = 0usize;
        let mut hi = count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let offset = mid * record_size;
            let key = &mmap[offset..offset + 32];
            match key.cmp(hash.as_slice()) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(offset),
            }
        }
        None
    }

    fn len(&self) -> usize {
        match self {
            Preimages::InMemory { addrs, slots } => addrs.len() + slots.len(),
            Preimages::Mmap {
                addr_count,
                slot_count,
                ..
            } => addr_count + slot_count,
        }
    }
}

/// Parse a gethdbdump preimage file.
///
/// In fast mode: loads all preimages into in-memory HashMaps (O(1) lookup, high RAM).
/// In normal mode: streams to sorted flat files on disk, then mmaps for O(log n) lookups.
fn parse_geth_dump(path: &str, datadir: &Path, fast: bool) -> eyre::Result<Preimages> {
    info!(
        "Reading preimage file: {path} (mode: {})",
        if fast { "in-memory" } else { "mmap" }
    );
    let file = File::open(path).map_err(|e| eyre::eyre!("Failed to open preimage file: {e}"))?;
    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    info!("Preimage file size: {} MB", file_size / 1024 / 1024);

    let mut reader = BufReader::with_capacity(8 * 1024 * 1024, file);
    skip_gethdbdump_header(&mut reader)?;

    // Fast mode: collect into HashMaps. Normal mode: stream to flat files.
    // Pre-size to avoid rehashing: ~70 bytes per RLP entry on average.
    let estimated_entries = (file_size / 70) as usize;
    let mut fast_addrs: FxHashMap<[u8; 32], [u8; 20]> = if fast {
        FxHashMap::with_capacity_and_hasher(estimated_entries / 4, Default::default())
    } else {
        FxHashMap::default()
    };
    let mut fast_slots: FxHashMap<[u8; 32], [u8; 32]> = if fast {
        FxHashMap::with_capacity_and_hasher(estimated_entries * 3 / 4, Default::default())
    } else {
        FxHashMap::default()
    };
    let addr_path = datadir.join("preimage_addrs.bin");
    let slot_path = datadir.join("preimage_slots.bin");
    let mut addr_writer = if !fast {
        Some(BufWriter::with_capacity(
            8 * 1024 * 1024,
            File::create(&addr_path)?,
        ))
    } else {
        None
    };
    let mut slot_writer = if !fast {
        Some(BufWriter::with_capacity(
            8 * 1024 * 1024,
            File::create(&slot_path)?,
        ))
    } else {
        None
    };

    let mut addr_count = 0usize;
    let mut slot_count = 0usize;
    let mut count = 0u64;
    let mut bufs = EntryBufs::new();
    let started = std::time::Instant::now();
    let mut last_log = std::time::Instant::now();
    let mut bytes_read = 0u64;

    loop {
        let Some((op, entry_bytes)) = read_gethdbdump_entry(&mut reader, &mut bufs)? else {
            break;
        };
        bytes_read += entry_bytes;
        if op != 0 || bufs.key.len() < 32 {
            continue;
        }
        let hash = &bufs.key[bufs.key.len() - 32..];

        match bufs.value.len() {
            20 => {
                if fast {
                    let mut h = [0u8; 32];
                    h.copy_from_slice(hash);
                    let mut v = [0u8; 20];
                    v.copy_from_slice(&bufs.value);
                    fast_addrs.insert(h, v);
                } else {
                    addr_writer.as_mut().unwrap().write_all(hash)?;
                    addr_writer.as_mut().unwrap().write_all(&bufs.value)?;
                }
                addr_count += 1;
            }
            32 => {
                if fast {
                    let mut h = [0u8; 32];
                    h.copy_from_slice(hash);
                    let mut v = [0u8; 32];
                    v.copy_from_slice(&bufs.value);
                    fast_slots.insert(h, v);
                } else {
                    slot_writer.as_mut().unwrap().write_all(hash)?;
                    slot_writer.as_mut().unwrap().write_all(&bufs.value)?;
                }
                slot_count += 1;
            }
            _ => {}
        }

        count += 1;
        if count % 100_000 == 0 && last_log.elapsed().as_secs() >= 5 {
            let pct = if file_size > 0 {
                (bytes_read as f64 / file_size as f64 * 100.0) as u32
            } else {
                0
            };
            info!("{pct}% | {count} preimage entries parsed...");
            last_log = std::time::Instant::now();
        }
    }

    info!(
        "Parsed {count} entries ({addr_count} addrs, {slot_count} slots) in {:.1}s",
        started.elapsed().as_secs_f64()
    );

    if fast {
        info!(
            "In-memory preimages: {} addrs, {} slots",
            fast_addrs.len(),
            fast_slots.len()
        );
        Ok(Preimages::InMemory {
            addrs: fast_addrs,
            slots: fast_slots,
        })
    } else {
        addr_writer.unwrap().flush()?;
        slot_writer.unwrap().flush()?;

        let addr_file = File::open(&addr_path)?;
        let slot_file = File::open(&slot_path)?;
        let addr_mmap = unsafe { Mmap::map(&addr_file)? };
        let slot_mmap = unsafe { Mmap::map(&slot_file)? };

        info!(
            "Preimage files mmapped ({addr_count} addrs = {} MB, {slot_count} slots = {} MB)",
            addr_count * ADDR_RECORD_SIZE / 1024 / 1024,
            slot_count * SLOT_RECORD_SIZE / 1024 / 1024,
        );

        Ok(Preimages::Mmap {
            addr_mmap,
            addr_count,
            slot_mmap,
            slot_count,
        })
    }
}

/// Skip the gethdbdump RLP list header.
fn skip_gethdbdump_header(reader: &mut BufReader<File>) -> eyre::Result<()> {
    let mut prefix_byte = [0u8; 1];
    reader
        .read_exact(&mut prefix_byte)
        .map_err(|e| eyre::eyre!("Failed to read header prefix: {e}"))?;

    let header_len = match prefix_byte[0] {
        b if b >= 0xf8 => {
            let len_of_len = (b - 0xf7) as usize;
            let mut len_bytes = vec![0u8; len_of_len];
            reader.read_exact(&mut len_bytes)?;
            let mut len = 0usize;
            for &byte in &len_bytes {
                len = (len << 8) | byte as usize;
            }
            len
        }
        b if b >= 0xc0 => (b - 0xc0) as usize,
        b => return Err(eyre::eyre!("Expected RLP list header, got 0x{b:02x}")),
    };

    let mut skip_buf = vec![0u8; header_len];
    reader.read_exact(&mut skip_buf)?;
    info!("Skipped gethdbdump header ({header_len} bytes)");
    Ok(())
}

/// Reusable buffers for reading gethdbdump entries without allocating.
struct EntryBufs {
    key: Vec<u8>,
    value: Vec<u8>,
}

impl EntryBufs {
    fn new() -> Self {
        Self {
            key: Vec::with_capacity(65),    // max key: "o" + 32 + 32 = 65
            value: Vec::with_capacity(128), // account RLP or storage value
        }
    }
}

/// Read one (op, key, value) entry from a gethdbdump stream into reusable buffers.
/// Returns None on EOF. Returns (op, bytes_consumed). Key/value are in bufs.
fn read_gethdbdump_entry(
    reader: &mut BufReader<File>,
    bufs: &mut EntryBufs,
) -> eyre::Result<Option<(u8, u64)>> {
    // Read op: first byte. Check for EOF.
    let mut first = [0u8; 1];
    match reader.read_exact(&mut first) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(eyre::eyre!("Failed to read entry: {e}")),
    }

    // Op is RLP-encoded: 0x80 = empty bytes = 0 (add), 0x01 = byte 1 (delete).
    let op = if first[0] == 0x80 { 0u8 } else { first[0] };
    let mut total_bytes = 1u64;

    let kb = read_rlp_bytes_into(reader, &mut bufs.key)?;
    total_bytes += kb;
    let vb = read_rlp_bytes_into(reader, &mut bufs.value)?;
    total_bytes += vb;

    Ok(Some((op, total_bytes)))
}

/// Read a single RLP-encoded byte string into `dst`, reusing its allocation.
/// Returns bytes consumed from the stream.
fn read_rlp_bytes_into(reader: &mut BufReader<File>, dst: &mut Vec<u8>) -> eyre::Result<u64> {
    let mut buf1 = [0u8; 1];
    reader.read_exact(&mut buf1)?;
    let prefix = buf1[0];

    if prefix < 0x80 {
        dst.clear();
        dst.push(prefix);
        Ok(1)
    } else if prefix <= 0xb7 {
        let len = (prefix - 0x80) as usize;
        dst.clear();
        if len == 0 {
            return Ok(1);
        }
        dst.resize(len, 0);
        reader.read_exact(dst)?;
        Ok(1 + len as u64)
    } else if prefix <= 0xbf {
        let len_of_len = (prefix - 0xb7) as usize;
        let mut len_bytes = [0u8; 8];
        reader.read_exact(&mut len_bytes[..len_of_len])?;
        let mut len = 0usize;
        for &b in &len_bytes[..len_of_len] {
            len = (len << 8) | b as usize;
        }
        dst.clear();
        dst.resize(len, 0);
        reader.read_exact(dst)?;
        Ok(1 + len_of_len as u64 + len as u64)
    } else {
        Err(eyre::eyre!(
            "Expected RLP string in stream, got 0x{prefix:02x}"
        ))
    }
}
