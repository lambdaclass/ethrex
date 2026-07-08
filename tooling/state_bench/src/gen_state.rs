//! `gen-state`: build a synthetic, deterministic state fixture on disk.
//!
//! The generator writes state directly into the account/storage tries (via the
//! public `Store::open_direct_*_trie` handles, persisting with `Trie::commit`),
//! finalizes a consistent block 0, generates the flat-KV index, and emits a
//! manifest describing everything a later phase needs.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use tracing::{info, warn};

use ethrex_blockchain::build_sharded_storage_trie_streaming;
use ethrex_common::constants::EMPTY_KECCAK_HASH;
use ethrex_common::types::{AccountState, Code, Genesis};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::{NativeCrypto, keccak::keccak_hash};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::api::tables::{
    ACCOUNT_FLATKEYVALUE, ACCOUNT_TRIE_NODES, MISC_VALUES, STORAGE_FLATKEYVALUE, STORAGE_TRIE_NODES,
};
use ethrex_storage::error::StoreError;
use ethrex_storage::{
    EngineType, STORE_SCHEMA_VERSION, Store, StoreConfig, hash_address, hash_key,
};
use ethrex_trie::EMPTY_TRIE_HASH;
use state_bench::fkv::{account_fkv_key, storage_fkv_key};

use crate::manifest::{MANIFEST_FILENAME, Manifest, StateCfSizes};

/// Slots committed to disk between `Trie::commit` calls while building the mega
/// account's storage trie, so peak memory stays bounded regardless of size.
const MEGA_COMMIT_CHUNK: u64 = 100_000;

/// Estimated storage-trie-node bytes each mega slot contributes on disk
/// (leaf + amortized branch nodes + SST overhead). Calibrated empirically so
/// the achieved `STORAGE_TRIE_NODES` SST size lands within +/-10% of the target
/// (see the calibration note in the completion report). The mega slot count is
/// `round(mega_target_bytes / this)`.
const MEGA_STORAGE_NODE_BYTES_PER_SLOT: f64 = 147.6;

/// Decimal gigabyte. `--mega-account-gb` is interpreted in decimal GB so a
/// target of `0.1` means 100 MB of `STORAGE_TRIE_NODES`.
const BYTES_PER_GB: f64 = 1_000_000_000.0;

/// The accessor contract's deployed bytecode. Hand-assembled EVM that loops
/// over 64-byte calldata records performing SLOAD or SSTORE per record. See
/// [`ACCESSOR_ABI`] for the calldata layout. Program (byte offsets in comments):
///
/// ```text
/// 00: PUSH1 0x00        ; i = 0 (calldata cursor)
/// 02: JUMPDEST          ; LOOP
/// 03: DUP1              ; [i i]
/// 04: CALLDATASIZE      ; [size i i]
/// 05: GT                ; [ (size>i) i ]  == (i < size)
/// 06: ISZERO            ; [ (i>=size) i ]
/// 07: PUSH1 0x24        ; END
/// 09: JUMPI             ; if i>=size goto END  -> [i]
/// 0a: DUP1              ; [i i]
/// 0b: CALLDATALOAD      ; [mode i]     (word @ i)
/// 0c: DUP2              ; [i mode i]
/// 0d: PUSH1 0x20        ; [32 i mode i]
/// 0f: ADD               ; [i+32 mode i]
/// 10: CALLDATALOAD      ; [slot mode i] (word @ i+32)
/// 11: SWAP1             ; [mode slot i]
/// 12: PUSH1 0x1a        ; SSTORE_LBL
/// 14: JUMPI             ; if mode!=0 goto SSTORE_LBL -> [slot i]
/// 15: SLOAD             ; [val i]
/// 16: POP               ; [i]
/// 17: PUSH1 0x1d        ; CONT
/// 19: JUMP              ; goto CONT -> [i]
/// 1a: JUMPDEST          ; SSTORE_LBL  [slot i]
/// 1b: DUP1              ; [slot slot i]  (value == key)
/// 1c: SSTORE            ; [i]
/// 1d: JUMPDEST          ; CONT  [i]
/// 1e: PUSH1 0x40        ; [64 i]
/// 20: ADD               ; [i+64]
/// 21: PUSH1 0x02        ; LOOP
/// 23: JUMP              ; goto LOOP
/// 24: JUMPDEST          ; END  [i]
/// 25: STOP
/// ```
const ACCESSOR_BYTECODE: [u8; 38] = [
    0x60, 0x00, // PUSH1 0x00
    0x5B, // JUMPDEST (LOOP=0x02)
    0x80, // DUP1
    0x36, // CALLDATASIZE
    0x11, // GT
    0x15, // ISZERO
    0x60, 0x24, // PUSH1 END
    0x57, // JUMPI
    0x80, // DUP1
    0x35, // CALLDATALOAD
    0x81, // DUP2
    0x60, 0x20, // PUSH1 0x20
    0x01, // ADD
    0x35, // CALLDATALOAD
    0x90, // SWAP1
    0x60, 0x1A, // PUSH1 SSTORE_LBL
    0x57, // JUMPI
    0x54, // SLOAD
    0x50, // POP
    0x60, 0x1D, // PUSH1 CONT
    0x56, // JUMP
    0x5B, // JUMPDEST (SSTORE_LBL=0x1a)
    0x80, // DUP1
    0x55, // SSTORE
    0x5B, // JUMPDEST (CONT=0x1d)
    0x60, 0x40, // PUSH1 0x40
    0x01, // ADD
    0x60, 0x02, // PUSH1 LOOP
    0x56, // JUMP
    0x5B, // JUMPDEST (END=0x24)
    0x00, // STOP
];

/// Human-readable description of the accessor's calldata ABI, recorded in the
/// manifest so Phase 3 can encode workload txs without re-deriving it.
const ACCESSOR_ABI: &str = "Calldata is a concatenation of N 64-byte records. \
Each record = [mode: 32-byte big-endian uint, 0 = SLOAD, nonzero = SSTORE] followed by \
[slot: 32-byte big-endian storage key]. For SSTORE, the stored value equals the slot key. \
The contract loops over records until the calldata cursor reaches CALLDATASIZE; keep the \
calldata length a multiple of 64 bytes (CALLDATALOAD zero-pads a short tail). No return value.";

/// Exact rule used to derive every deterministic value, recorded in the manifest.
const DERIVATION_RULE: &str = "digest(tag, index) = keccak256(seed.to_le_bytes(8) || tag_ascii || index.to_le_bytes(8)). \
Addresses = digest[12..32]; slot keys = digest (full 32 bytes); slot values = U256::from_big_endian(digest), bumped to 1 if zero. \
Tags: small account address='csb-small-acct', small slot key='csb-small-slot' (index = account_index*slots_per_account + slot_index), \
small slot value='csb-small-val' (same index), mega account address='csb-mega-acct' (index 0), mega slot key='csb-mega-slot' (index = slot number), \
mega slot value='csb-mega-val' (index = slot number), accessor address='csb-accessor' (index 0), signer key='csb-signer' (index 0, re-hashed with index+1.. until a valid secp256k1 scalar).";

/// Parameters parsed from the `gen-state` CLI, plus the resolved worker count.
pub struct GenStateArgs {
    pub datadir: PathBuf,
    pub num_small_accounts: u64,
    pub slots_per_account: u64,
    pub mega_account_gb: f64,
    pub seed: u64,
    pub genesis: PathBuf,
    pub jobs: usize,
    /// Diagnostic-only: enable RocksDB statistics and log them (plus
    /// per-CF compaction/write indicators) at every mega storage trie
    /// progress checkpoint.
    pub rocksdb_stats: bool,
    /// RocksDB block-cache size in GiB (see the CLI flag). Larger caches keep
    /// the shards' post-eviction re-reads from thrashing on big fixtures.
    pub block_cache_gb: f64,
}

/// keccak256(seed_le || tag || index_le) — the single deterministic primitive.
/// Shared with `gen-workload`, which re-derives the same addresses and slot keys
/// to target the exact accounts/slots this generator seeded.
pub(crate) fn digest(seed: u64, tag: &str, index: u64) -> [u8; 32] {
    let mut buf = Vec::with_capacity(8 + tag.len() + 8);
    buf.extend_from_slice(&seed.to_le_bytes());
    buf.extend_from_slice(tag.as_bytes());
    buf.extend_from_slice(&index.to_le_bytes());
    keccak_hash(&buf)
}

pub(crate) fn derive_address(seed: u64, tag: &str, index: u64) -> Address {
    Address::from_slice(&digest(seed, tag, index)[12..32])
}

pub(crate) fn derive_slot_key(seed: u64, tag: &str, index: u64) -> H256 {
    H256(digest(seed, tag, index))
}

/// Number of storage slots the mega account was seeded with for a given target
/// byte size, using the same calibration constant as generation. `gen-workload`
/// calls this to bound the range of seeded mega slots it can pick for cold reads.
pub(crate) fn mega_slot_count(mega_target_bytes: u64) -> u64 {
    (mega_target_bytes as f64 / MEGA_STORAGE_NODE_BYTES_PER_SLOT).round() as u64
}

/// Non-zero U256 derived from the digest (a zero storage value would be a
/// no-op / trie removal, so bump it to 1).
fn derive_slot_value(seed: u64, tag: &str, index: u64) -> U256 {
    let v = U256::from_big_endian(&digest(seed, tag, index));
    if v.is_zero() { U256::one() } else { v }
}

/// Deterministic funded EOA: keccak(seed || "csb-signer" || n) for n = 0, 1, ...
/// until it is a valid secp256k1 scalar (n = 0 works with overwhelming probability).
fn derive_signer(seed: u64) -> Result<(SecretKey, Address)> {
    for n in 0..256u64 {
        let bytes = digest(seed, "csb-signer", n);
        if let Ok(sk) = SecretKey::from_slice(&bytes) {
            let secp = Secp256k1::new();
            let pk = PublicKey::from_secret_key(&secp, &sk);
            let uncompressed = pk.serialize_uncompressed();
            let addr = Address::from_slice(&keccak_hash(&uncompressed[1..])[12..32]);
            return Ok((sk, addr));
        }
    }
    bail!("failed to derive a valid signer key from seed {seed}")
}

/// Streams flat-KV entries straight from the data written to the tries, so we
/// never call the store's background `generate_flatkeyvalue()` (which re-scans
/// the whole trie and allocates ~fixture-proportional heap during generation —
/// the dominant peak-RSS scaler at multi-GB fixtures). Entries are staged and
/// written+flushed in bounded batches so peak memory stays flat regardless of
/// fixture size.
///
/// Correct by construction: every entry uses the exact hashed key and RLP value
/// that were inserted into the trie, encoded exactly as the store's generator
/// would (`Nibbles::from_bytes` leaf paths for accounts; `apply_prefix` +
/// leaf-path for storage slots). An equivalence test guards the encoding.
struct FkvWriter {
    storage_batch: Vec<(Vec<u8>, Vec<u8>)>,
    account_batch: Vec<(Vec<u8>, Vec<u8>)>,
    batch_limit: usize,
}

impl FkvWriter {
    fn new() -> Self {
        Self {
            storage_batch: Vec::new(),
            account_batch: Vec::new(),
            // ~500k entries/batch bounds the staged Vec to a few tens of MB; the
            // per-batch flush then keeps RocksDB memtables from piling up.
            batch_limit: 500_000,
        }
    }

    /// Stage a storage-slot flat-KV entry. `hashed_key` is the exact byte slice
    /// inserted into `account_hash`'s storage trie; `value_rlp` is the exact RLP
    /// stored at that leaf.
    fn add_storage(
        &mut self,
        store: &Store,
        account_hash: H256,
        hashed_key: &[u8],
        value_rlp: Vec<u8>,
    ) -> Result<()> {
        self.storage_batch
            .push((storage_fkv_key(account_hash, hashed_key), value_rlp));
        if self.storage_batch.len() >= self.batch_limit {
            self.flush_storage(store)?;
        }
        Ok(())
    }

    /// Stage an account flat-KV entry (`state_rlp` = the exact RLP stored at the
    /// account leaf).
    fn add_account(&mut self, hashed_address: &[u8], state_rlp: Vec<u8>) {
        self.account_batch
            .push((account_fkv_key(hashed_address), state_rlp));
    }

    fn flush_storage(&mut self, store: &Store) -> Result<()> {
        if self.storage_batch.is_empty() {
            return Ok(());
        }
        store
            .write_batch(
                STORAGE_FLATKEYVALUE,
                std::mem::take(&mut self.storage_batch),
            )
            .context("writing storage flat-KV batch")?;
        // NB: no explicit flush here. An earlier version flushed every batch to
        // bound memtables, but profiling showed those synchronous full-DB
        // flushes dominated the parallel build (stalling the producer and
        // starving the shard workers) while barely affecting peak RSS —
        // RocksDB's own `db_write_buffer_size` trigger bounds memtables. Only
        // `finish()` flushes, once, so the sizing checkpoint sees the data.
        Ok(())
    }

    /// Flush all remaining entries and mark flat-KV generation complete so the
    /// store — and a later reopen by `run` — treats the flat-KV as authoritative.
    fn finish(mut self, store: &Store) -> Result<()> {
        self.flush_storage(store)?;
        if !self.account_batch.is_empty() {
            store
                .write_batch(
                    ACCOUNT_FLATKEYVALUE,
                    std::mem::take(&mut self.account_batch),
                )
                .context("writing account flat-KV batch")?;
        }
        // Completion sentinel: the store marks flat-KV done by persisting a lone
        // 0xff byte at MISC_VALUES/last_written (matches the background generator;
        // a reopen expands it to an all-0xff cursor => every path uses the FKV).
        store
            .write_batch(MISC_VALUES, vec![(b"last_written".to_vec(), vec![0xff])])
            .context("writing flat-KV completion sentinel")?;
        store.flush().context("final flat-KV flush")?;
        Ok(())
    }
}

/// Insert one storage-bearing account's slots into its storage trie, persist,
/// and return the resulting storage root. Stages each slot's flat-KV entry too.
fn build_small_storage_trie(
    store: &Store,
    fkv: &mut FkvWriter,
    account_hash: H256,
    seed: u64,
    account_index: u64,
    slots_per_account: u64,
) -> Result<H256> {
    let mut trie = store.open_direct_storage_trie(account_hash, *EMPTY_TRIE_HASH)?;
    for slot_index in 0..slots_per_account {
        let global_index = account_index * slots_per_account + slot_index;
        let key = derive_slot_key(seed, "csb-small-slot", global_index);
        let value = derive_slot_value(seed, "csb-small-val", global_index);
        let hashed = hash_key(&key);
        let value_rlp = value.encode_to_vec();
        trie.insert(hashed.clone(), value_rlp.clone())?;
        fkv.add_storage(store, account_hash, &hashed, value_rlp)?;
    }
    Ok(trie.hash(&NativeCrypto)?)
}

/// Diagnostic-only: log RocksDB statistics plus compaction/write indicators
/// for `STORAGE_TRIE_NODES`, so a mega build can be judged CPU-bound vs
/// compaction/IO-bound. Gated by `--rocksdb-stats`; a no-op (and thus inert)
/// when statistics weren't enabled on the store (in-memory backend, or
/// `--rocksdb-stats` off).
fn log_rocksdb_progress_stats(store: &Store, slots: u64, total: u64) {
    let total_sst_files_size =
        store.rocksdb_cf_property(STORAGE_TRIE_NODES, "rocksdb.total-sst-files-size");
    let compaction_pending =
        store.rocksdb_cf_property(STORAGE_TRIE_NODES, "rocksdb.compaction-pending");
    let actual_delayed_write_rate =
        store.rocksdb_cf_property(STORAGE_TRIE_NODES, "rocksdb.actual-delayed-write-rate");
    info!(
        slots,
        total,
        total_sst_files_size,
        compaction_pending,
        actual_delayed_write_rate,
        "mega storage trie STORAGE_TRIE_NODES cf stats"
    );
    if let Some(stats) = store.rocksdb_statistics_string() {
        info!(%stats, "mega storage trie rocksdb statistics");
    }
}

/// Build the mega account's storage trie in parallel across 16 nibble-sharded
/// workers (see [`build_sharded_storage_trie_streaming`]), staging each slot's
/// flat-KV entry as it is produced. Peak memory grows only logarithmically with
/// fixture size (per-shard eviction + `clear_dirty` bound each worker to ~one
/// chunk; what grows is the depth of the path re-materialized per chunk):
/// measured ~6.2 GB at a 4 GB fixture, projecting to ~8-9 GB at 50 GB. Higher
/// than the serial build's flat ~2.4 GB (16 concurrent arenas) but the fan-out
/// roughly halves wall-clock.
fn build_mega_storage_trie(
    store: &Store,
    fkv: &mut FkvWriter,
    account_hash: H256,
    seed: u64,
    slot_count: u64,
    rocksdb_stats: bool,
) -> Result<H256> {
    let derive = |k: u64| {
        let key = derive_slot_key(seed, "csb-mega-slot", k);
        let value = derive_slot_value(seed, "csb-mega-val", k);
        (H256::from_slice(&hash_key(&key)), value.encode_to_vec())
    };
    let on_slot = |hashed_key: &H256, value_rlp: &[u8]| -> Result<(), StoreError> {
        fkv.add_storage(
            store,
            account_hash,
            hashed_key.as_bytes(),
            value_rlp.to_vec(),
        )
        .map_err(|e| StoreError::Custom(format!("flat-KV staging: {e}")))
    };
    let root = build_sharded_storage_trie_streaming(
        store,
        account_hash,
        slot_count,
        MEGA_COMMIT_CHUNK,
        derive,
        on_slot,
    )?;
    if rocksdb_stats {
        log_rocksdb_progress_stats(store, slot_count, slot_count);
    }
    Ok(root)
}

/// Measure the on-disk SST size of the four state column families.
///
/// The generator's writes accumulate in the WAL + memtables (the 512 MB per-CF
/// write buffer is never reached for small fixtures) and are not turned into
/// SSTs, so a plain reopen would report ~0 SST bytes. Rather than reopen the
/// live datadir (whose RocksDB LOCK is still held by the store's background
/// threads), take a RocksDB checkpoint of the still-open store: a checkpoint
/// with `log_size_for_flush = 0` flushes memtables to fresh SSTs using the
/// store's own CF options (compression stays off for these CFs), producing a
/// standalone snapshot whose `total-sst-files-size` matches a real datadir
/// within the +/-10% tolerance. The checkpoint is opened read-only in its own
/// directory (no LOCK conflict) and removed afterwards.
fn measure_state_cf_sizes(store: &Store, datadir: &Path) -> Result<StateCfSizes> {
    use rocksdb::{DB, Options};

    let checkpoint = checkpoint_path(datadir);
    if checkpoint.exists() {
        std::fs::remove_dir_all(&checkpoint)
            .with_context(|| format!("removing stale checkpoint {}", checkpoint.display()))?;
    }
    store
        .create_checkpoint(&checkpoint)
        .context("creating sizing checkpoint (flushes memtables to SST)")?;

    let sizes = (|| -> Result<StateCfSizes> {
        let opts = Options::default();
        let cf_names = DB::list_cf(&opts, &checkpoint)
            .context("listing column families of the sizing checkpoint")?;
        let db = DB::open_cf_for_read_only(&opts, &checkpoint, &cf_names, false)
            .context("opening the sizing checkpoint read-only")?;

        let size_of = |name: &str| -> Result<u64> {
            let cf = db
                .cf_handle(name)
                .with_context(|| format!("column family {name} missing from checkpoint"))?;
            Ok(db
                .property_int_value_cf(&cf, "rocksdb.total-sst-files-size")?
                .unwrap_or(0))
        };

        Ok(StateCfSizes {
            account_trie_nodes: size_of(ACCOUNT_TRIE_NODES)?,
            storage_trie_nodes: size_of(STORAGE_TRIE_NODES)?,
            account_flatkeyvalue: size_of(ACCOUNT_FLATKEYVALUE)?,
            storage_flatkeyvalue: size_of(STORAGE_FLATKEYVALUE)?,
        })
    })();

    // Always clean up the checkpoint, even if sizing failed.
    let _ = std::fs::remove_dir_all(&checkpoint);
    sizes
}

/// Sibling directory used for the transient sizing checkpoint. Kept next to the
/// datadir so the checkpoint's hardlinks land on the same filesystem.
fn checkpoint_path(datadir: &Path) -> PathBuf {
    let name = datadir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "datadir".to_string());
    let parent = datadir.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!(".{name}-sizeckpt"))
}

pub async fn run(args: GenStateArgs) -> Result<()> {
    let GenStateArgs {
        datadir,
        num_small_accounts,
        slots_per_account,
        mega_account_gb,
        seed,
        genesis,
        jobs,
        rocksdb_stats,
        block_cache_gb,
    } = args;

    if datadir.exists() && datadir.read_dir().map(|mut d| d.next().is_some())? {
        bail!(
            "datadir {} already exists and is non-empty; refusing to overwrite a fixture",
            datadir.display()
        );
    }
    std::fs::create_dir_all(&datadir)
        .with_context(|| format!("creating datadir {}", datadir.display()))?;

    // Base genesis: we borrow its chain config AND its alloc. The alloc holds the
    // fork's system contracts (EIP-2935 history, EIP-4788 beacon roots, EIP-7002
    // withdrawals, EIP-7251 consolidations, deposit contract). Those MUST be
    // present in the seeded state or the payload builder's system-operations phase
    // aborts on Amsterdam+ ("system contract has no code after deployment") and no
    // block (hence no BAL) can ever be built or imported on the fixture. We take
    // the alloc out (clearing it on `genesis` so `get_block` builds a header from an
    // empty alloc — its state_root is overridden with our computed root below) and
    // seed every alloc account into the state trie alongside the synthetic accounts.
    let genesis_bytes = std::fs::read(&genesis)
        .with_context(|| format!("reading base genesis {}", genesis.display()))?;
    let mut genesis: Genesis =
        serde_json::from_slice(&genesis_bytes).context("parsing base genesis JSON")?;
    let base_alloc = std::mem::take(&mut genesis.alloc);

    info!(
        datadir = %datadir.display(),
        num_small_accounts,
        slots_per_account,
        mega_account_gb,
        seed,
        jobs,
        "gen-state: opening fresh datadir"
    );

    // gen-state is a bulk WRITE, so the 12 GiB default read cache is wasted —
    // but the shard workers re-read trie nodes after each per-chunk eviction, so
    // the cache still matters for build speed on large fixtures (a cache smaller
    // than the working set thrashes). Default 1 GiB (fine for small builds);
    // `--block-cache-gb` raises it for big ones (see the flag docs).
    let block_cache_bytes = (block_cache_gb * (1u64 << 30) as f64).round() as usize;
    info!(block_cache_gb, "gen-state block cache");
    let store_config = StoreConfig {
        rocksdb_enable_statistics: rocksdb_stats,
        rocksdb_block_cache_size: block_cache_bytes,
        ..StoreConfig::default()
    };
    let mut store = Store::new_with_config(&datadir, EngineType::RocksDB, store_config)
        .context("opening fresh RocksDB store")?;
    store
        .set_chain_config(&genesis.config)
        .await
        .context("applying chain config from base genesis")?;

    // --- Derive deterministic identities -----------------------------------
    let accessor_address = derive_address(seed, "csb-accessor", 0);
    let mega_address = derive_address(seed, "csb-mega-acct", 0);
    let (signer_sk, signer_address) = derive_signer(seed)?;

    // --- Accessor contract ---------------------------------------------------
    // The accessor bytecode is SHARED code: it is stored once (content-addressed
    // by hash) and assigned as the `code_hash` of every storage-bearing account
    // below (small accounts + mega account). Phase-3 txs target those accounts
    // directly so SLOAD/SSTORE hit each account's own pre-seeded storage.
    // A standalone `accessor_address` account (empty storage) is also recorded
    // as the canonical accessor reference in the manifest.
    let accessor_code = Code::from_bytecode(Bytes::from_static(&ACCESSOR_BYTECODE), &NativeCrypto);
    let accessor_code_hash = accessor_code.hash;
    store
        .add_account_code(accessor_code)
        .await
        .context("storing accessor bytecode")?;

    // Accounts get inserted into the state trie once all their storage tries
    // (and thus storage roots) are known. Collect (hashed_address, state) here.
    let mut account_entries: Vec<(Vec<u8>, AccountState)> = Vec::new();

    // Flat-KV is written directly from the data we insert (see `FkvWriter`),
    // bypassing the store's scan-and-allocate background generator.
    let mut fkv = FkvWriter::new();

    // --- Base genesis alloc (system contracts + any pre-funded accounts) -----
    // Seed every account from the base genesis so the fork's system contracts
    // exist in the fixture state. Mirrors `Store::setup_genesis_state_trie`.
    info!(
        alloc_accounts = base_alloc.len(),
        "seeding base genesis alloc"
    );
    for (address, account) in &base_alloc {
        let code = Code::from_bytecode(account.code.clone(), &NativeCrypto);
        let code_hash = code.hash;
        store
            .add_account_code(code)
            .await
            .context("storing genesis alloc account code")?;

        let account_hash = H256::from_slice(&hash_address(address));
        let mut storage_trie = store.open_direct_storage_trie(account_hash, *EMPTY_TRIE_HASH)?;
        for (storage_key, storage_value) in &account.storage {
            if !storage_value.is_zero() {
                let hashed_key = hash_key(&H256(storage_key.to_big_endian()));
                let value_rlp = storage_value.encode_to_vec();
                storage_trie.insert(hashed_key.clone(), value_rlp.clone())?;
                fkv.add_storage(&store, account_hash, &hashed_key, value_rlp)?;
            }
        }
        let storage_root = storage_trie.hash(&NativeCrypto)?;

        account_entries.push((
            hash_address(address),
            AccountState {
                nonce: account.nonce,
                balance: account.balance,
                storage_root,
                code_hash,
            },
        ));
    }

    account_entries.push((
        hash_address(&accessor_address),
        AccountState {
            nonce: 1,
            balance: U256::zero(),
            storage_root: *EMPTY_TRIE_HASH,
            code_hash: accessor_code_hash,
        },
    ));

    // --- Funded signer EOA ---------------------------------------------------
    // ~1e6 ETH so it can fund a long Phase-3 workload.
    let signer_balance = U256::from(1_000_000u64) * U256::exp10(18);
    account_entries.push((
        hash_address(&signer_address),
        AccountState {
            nonce: 0,
            balance: signer_balance,
            storage_root: *EMPTY_TRIE_HASH,
            code_hash: *EMPTY_KECCAK_HASH,
        },
    ));

    // --- N small storage-bearing accounts -----------------------------------
    info!(
        num_small_accounts,
        slots_per_account, "building small accounts"
    );
    // Every storage-bearing account carries the SHARED accessor bytecode
    // (content-addressed code is stored once; sharing the hash is free). This
    // is what makes the pre-seeded storage reachable: a Phase-3 tx sends
    // `to = <this account>` with accessor calldata, and SLOAD/SSTORE then hit
    // THIS account's own (pre-built, cold) storage trie. Contracts carry
    // nonce 1 (EIP-161).
    for i in 0..num_small_accounts {
        let address = derive_address(seed, "csb-small-acct", i);
        let account_hash = H256::from_slice(&hash_address(&address));
        let storage_root =
            build_small_storage_trie(&store, &mut fkv, account_hash, seed, i, slots_per_account)?;
        account_entries.push((
            hash_address(&address),
            AccountState {
                nonce: 1,
                balance: U256::zero(),
                storage_root,
                code_hash: accessor_code_hash,
            },
        ));
    }

    // --- Mega storage account ------------------------------------------------
    let mega_target_bytes = (mega_account_gb * BYTES_PER_GB).round() as u64;
    let mega_slot_count = mega_slot_count(mega_target_bytes);
    info!(
        mega_target_bytes,
        mega_slot_count, "building mega storage account"
    );
    let mega_account_hash = H256::from_slice(&hash_address(&mega_address));
    let mega_storage_root = build_mega_storage_trie(
        &store,
        &mut fkv,
        mega_account_hash,
        seed,
        mega_slot_count,
        rocksdb_stats,
    )?;
    account_entries.push((
        hash_address(&mega_address),
        AccountState {
            nonce: 1,
            balance: U256::zero(),
            storage_root: mega_storage_root,
            code_hash: accessor_code_hash,
        },
    ));

    // --- Insert all accounts into the state trie, compute the root ----------
    info!(accounts = account_entries.len(), "committing state trie");
    let mut state_trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH)?;
    for (hashed_address, state) in &account_entries {
        let state_rlp = state.encode_to_vec();
        state_trie.insert(hashed_address.clone(), state_rlp.clone())?;
        fkv.add_account(hashed_address, state_rlp);
    }
    let computed_state_root = state_trie.hash(&NativeCrypto)?;
    info!(state_root = %computed_state_root, "state trie committed");

    // --- Finalize a consistent block 0 --------------------------------------
    // Genesis header is built from the (now empty-alloc) base genesis, then its
    // state root is overridden with the root we just computed. The header hash
    // is computed lazily, so mutating the field before the first `hash()` call
    // yields a header consistent with our state.
    let mut genesis_block = genesis.get_block();
    genesis_block.header.state_root = computed_state_root;
    let genesis_hash = genesis_block.hash();
    store
        .add_block_header(genesis_hash, genesis_block.header.clone())
        .await
        .context("storing genesis header")?;
    store
        .add_block(genesis_block)
        .await
        .context("storing genesis block")?;
    store
        .update_earliest_block_number(0)
        .await
        .context("setting earliest block number")?;
    store
        .forkchoice_update(vec![], 0, genesis_hash, None, None)
        .await
        .context("forkchoice update to genesis")?;
    info!(hash = %genesis_hash, "block 0 finalized");

    // --- Write flat-KV directly, bypassing the store's scan-based generator -
    // We already staged every account and storage entry as we built the tries;
    // flush them and mark generation complete. This keeps peak RSS flat vs
    // fixture size (the store generator re-scans the whole trie and allocates
    // ~fixture-proportional heap during generation).
    info!("writing flat-KV directly");
    fkv.finish(&store).context("writing flat-KV")?;
    info!("flat-KV complete");

    // Measure per-CF on-disk sizes via a checkpoint of the still-open store,
    // then release it.
    let state_cf_sizes = measure_state_cf_sizes(&store, &datadir)?;
    drop(store);

    let mega_storage_bytes_achieved = state_cf_sizes.storage_trie_nodes;
    let mega_percent_of_target = if mega_target_bytes > 0 {
        mega_storage_bytes_achieved as f64 / mega_target_bytes as f64 * 100.0
    } else {
        0.0
    };

    let manifest = Manifest {
        schema_version: STORE_SCHEMA_VERSION,
        seed,
        jobs,
        num_small_accounts,
        slots_per_account,
        mega_account_gb,
        accessor_contract_address: format!("{accessor_address:#x}"),
        accessor_calldata_abi: ACCESSOR_ABI.to_string(),
        accessor_bytecode: format!("0x{}", hex_encode(&ACCESSOR_BYTECODE)),
        mega_account_address: format!("{mega_address:#x}"),
        small_account_derivation_rule: DERIVATION_RULE.to_string(),
        funded_signer_private_key: format!("0x{}", hex_encode(&signer_sk.secret_bytes())),
        funded_signer_address: format!("{signer_address:#x}"),
        computed_state_root: format!("{computed_state_root:#x}"),
        state_cf_sizes,
        mega_storage_bytes_achieved,
        mega_target_bytes,
        mega_percent_of_target,
    };

    let manifest_path = datadir.join(MANIFEST_FILENAME);
    let manifest_json = serde_json::to_string_pretty(&manifest).context("serializing manifest")?;
    std::fs::write(&manifest_path, manifest_json)
        .with_context(|| format!("writing manifest {}", manifest_path.display()))?;

    if !(90.0..=110.0).contains(&mega_percent_of_target) {
        warn!(
            mega_percent_of_target,
            mega_storage_bytes_achieved,
            mega_target_bytes,
            "mega storage size is outside the +/-10% target band; adjust MEGA_STORAGE_NODE_BYTES_PER_SLOT"
        );
    }

    info!(
        manifest = %manifest_path.display(),
        account_trie_nodes = manifest.state_cf_sizes.account_trie_nodes,
        storage_trie_nodes = manifest.state_cf_sizes.storage_trie_nodes,
        account_flatkeyvalue = manifest.state_cf_sizes.account_flatkeyvalue,
        storage_flatkeyvalue = manifest.state_cf_sizes.storage_flatkeyvalue,
        mega_percent_of_target,
        "gen-state complete"
    );

    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
