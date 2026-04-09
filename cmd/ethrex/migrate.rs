use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use ethrex_binary_trie::key_mapping::{
    CODE_HASH_LEAF_KEY, chunkify_code, get_stem_for_base, get_tree_key_for_code_chunk,
    get_tree_key_for_storage_slot, pack_basic_data, tree_key_from_stem,
};
use ethrex_common::{Address, H256, U256, types::Genesis};
use rustc_hash::FxHashMap;
use tracing::{info, warn};

use crate::initializers::{init_binary_trie_state, init_store, open_store};
use crate::utils::is_memory_datadir;

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
pub async fn migrate_with_preimages(
    preimage_path: &str,
    snapshot_path: &str,
    datadir: &Path,
    genesis: Genesis,
) -> eyre::Result<()> {
    // Step 1: Open store and init binary trie state.
    let mut store = init_store(datadir, genesis.clone())
        .await
        .map_err(|e| eyre::eyre!("Failed to open store: {e}"))?;

    let binary_trie_state = init_binary_trie_state(&store, datadir, &genesis)?;
    store.set_binary_trie_state(binary_trie_state.clone());

    // Step 2: Parse preimage file into keccak_hash -> preimage map.
    let preimages = parse_geth_dump(preimage_path, "preimage")?;
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

    loop {
        let Some((key, value, entry_bytes)) = read_gethdbdump_entry(&mut reader)? else {
            break;
        };
        bytes_read += entry_bytes;

        // Log progress every 5 seconds
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

        // Account entry: key = "a" (1 byte) + keccak(address) (32 bytes) = 33 bytes
        if key.len() == 33 && key[0] == b'a' {
            let keccak_addr: [u8; 32] = key[1..33].try_into().unwrap();

            let Some(addr_preimage) = preimages.get(&keccak_addr) else {
                skipped += 1;
                continue;
            };
            if addr_preimage.len() != 20 {
                skipped += 1;
                continue;
            }
            let address = Address::from_slice(addr_preimage);

            // Decode slim RLP account: [nonce, balance, root?, codehash?]
            let (nonce, balance, code_hash) = decode_slim_account(&value)?;

            let code_size = 0u32; // TODO: resolve code from separate source if needed

            let stem = get_stem_for_base(&address);
            let basic_data_key = tree_key_from_stem(&stem, 0);
            let basic_data = pack_basic_data(0, code_size, nonce, balance);

            let mut state = binary_trie_state
                .write()
                .map_err(|e| eyre::eyre!("Binary trie lock error: {e}"))?;

            state
                .trie_insert(basic_data_key, basic_data)
                .map_err(|e| eyre::eyre!("Failed to insert basic_data: {e}"))?;

            let code_hash_key = tree_key_from_stem(&stem, CODE_HASH_LEAF_KEY);
            state
                .trie_insert(code_hash_key, code_hash)
                .map_err(|e| eyre::eyre!("Failed to insert code_hash: {e}"))?;

            drop(state);

            account_count += 1;
            if account_count % 100_000 == 0 {
                info!("Processed {account_count} accounts, {storage_count} storage slots...");
            }
        }
        // Storage entry: key = "o" (1 byte) + keccak(address) (32 bytes) + keccak(slot) (32 bytes) = 65 bytes
        else if key.len() == 65 && key[0] == b'o' {
            let keccak_addr: [u8; 32] = key[1..33].try_into().unwrap();
            let keccak_slot: [u8; 32] = key[33..65].try_into().unwrap();

            let Some(addr_preimage) = preimages.get(&keccak_addr) else {
                skipped += 1;
                continue;
            };
            if addr_preimage.len() != 20 {
                skipped += 1;
                continue;
            }
            let address = Address::from_slice(addr_preimage);

            let Some(slot_preimage) = preimages.get(&keccak_slot) else {
                skipped += 1;
                continue;
            };
            if slot_preimage.len() != 32 {
                skipped += 1;
                continue;
            }
            let storage_key = U256::from_big_endian(slot_preimage);

            // Storage value is RLP-encoded bytes (the raw trie value, not U256).
            // In geth's snapshot, storage values are stored as trimmed big-endian bytes.
            let value_u256 = if value.is_empty() {
                U256::zero()
            } else {
                U256::from_big_endian(&value)
            };

            if value_u256.is_zero() {
                storage_count += 1;
                continue;
            }

            let tree_key = get_tree_key_for_storage_slot(&address, storage_key);
            let value_bytes = value_u256.to_big_endian();

            let mut state = binary_trie_state
                .write()
                .map_err(|e| eyre::eyre!("Binary trie lock error: {e}"))?;
            state
                .trie_insert(tree_key, value_bytes)
                .map_err(|e| eyre::eyre!("Failed to insert storage slot: {e}"))?;
            drop(state);

            storage_count += 1;
            if storage_count % 1_000_000 == 0 {
                info!("Processed {account_count} accounts, {storage_count} storage slots...");
            }
        }
        // First entry is a delete of SnapshotRootKey -- skip it.
        // Any other key format -- skip.
    }

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
    } else {
        U256::from_big_endian(balance_bytes)
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
        Ok((&data[start..start + len], start + len))
    } else if prefix >= 0xc0 {
        let len = (prefix - 0xc0) as usize;
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
        } else {
            Ok((&data[1..1 + len], 1 + len))
        }
    } else if prefix <= 0xbf {
        let len_of_len = (prefix - 0xb7) as usize;
        let mut len = 0usize;
        for &b in &data[1..1 + len_of_len] {
            len = (len << 8) | b as usize;
        }
        let start = 1 + len_of_len;
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

/// Parse a gethdbdump file into a map of key -> value.
/// For preimage files: key = last 32 bytes of the DB key (keccak hash), value = preimage.
fn parse_geth_dump(path: &str, kind: &str) -> eyre::Result<FxHashMap<[u8; 32], Vec<u8>>> {
    info!("Reading {kind} file: {path}");
    let file = File::open(path).map_err(|e| eyre::eyre!("Failed to open {kind} file: {e}"))?;
    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    info!("{kind} file size: {} MB", file_size / 1024 / 1024);

    let mut reader = BufReader::with_capacity(8 * 1024 * 1024, file);
    skip_gethdbdump_header(&mut reader)?;

    let mut map: FxHashMap<[u8; 32], Vec<u8>> = FxHashMap::default();
    let mut count = 0u64;

    let mut bytes_read = 0u64;
    let started = std::time::Instant::now();
    let mut last_log = std::time::Instant::now();

    loop {
        let Some((key, value, entry_bytes)) = read_gethdbdump_entry(&mut reader)? else {
            break;
        };
        bytes_read += entry_bytes;
        if key.len() < 32 {
            continue;
        }
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&key[key.len() - 32..]);
        map.insert(hash, value);

        count += 1;
        if last_log.elapsed().as_secs() >= 5 {
            let pct = if file_size > 0 {
                (bytes_read as f64 / file_size as f64 * 100.0) as u32
            } else {
                0
            };
            info!("{pct}% | {count} {kind} entries parsed...");
            last_log = std::time::Instant::now();
        }
    }
    let elapsed = started.elapsed();

    info!(
        "Parsed {count} {kind} entries ({} unique) in {:.1}s",
        map.len(),
        elapsed.as_secs_f64()
    );
    Ok(map)
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

/// Read one (op, key, value) entry from a gethdbdump stream.
/// Returns None on EOF. Returns (key, value, bytes_consumed).
fn read_gethdbdump_entry(
    reader: &mut BufReader<File>,
) -> eyre::Result<Option<(Vec<u8>, Vec<u8>, u64)>> {
    // Read op: first byte. Check for EOF.
    let mut first = [0u8; 1];
    match reader.read_exact(&mut first) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(eyre::eyre!("Failed to read entry: {e}")),
    }

    // Op is RLP-encoded: 0x80 = empty bytes = 0 (add), 0x01 = byte 1 (delete).
    let (op, ob) = if first[0] == 0x80 {
        (0u8, 1u64) // empty string = 0
    } else if first[0] < 0x80 {
        (first[0], 1u64) // single byte value
    } else {
        // Shouldn't happen for op, but handle gracefully
        (first[0], 1u64)
    };

    let (key, kb) = read_rlp_bytes(reader)?;
    let (value, vb) = read_rlp_bytes(reader)?;
    let total_bytes = ob + kb + vb;

    // op 0 = add, 1 = delete. Skip deletes.
    if op != 0 {
        return Ok(Some((vec![], vec![], total_bytes)));
    }

    Ok(Some((key, value, total_bytes)))
}

/// Read a single RLP-encoded byte string from a buffered reader.
/// Returns (data, bytes_consumed_from_stream).
fn read_rlp_bytes(reader: &mut BufReader<File>) -> eyre::Result<(Vec<u8>, u64)> {
    let mut buf1 = [0u8; 1];
    reader.read_exact(&mut buf1)?;
    let prefix = buf1[0];

    if prefix < 0x80 {
        Ok((vec![prefix], 1))
    } else if prefix <= 0xb7 {
        let len = (prefix - 0x80) as usize;
        if len == 0 {
            return Ok((vec![], 1));
        }
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf)?;
        Ok((buf, 1 + len as u64))
    } else if prefix <= 0xbf {
        let len_of_len = (prefix - 0xb7) as usize;
        let mut len_bytes = vec![0u8; len_of_len];
        reader.read_exact(&mut len_bytes)?;
        let mut len = 0usize;
        for &b in &len_bytes {
            len = (len << 8) | b as usize;
        }
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf)?;
        Ok((buf, 1 + len_of_len as u64 + len as u64))
    } else {
        Err(eyre::eyre!(
            "Expected RLP string in stream, got 0x{prefix:02x}"
        ))
    }
}
