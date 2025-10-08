// EthrexDB Data:
// 1. CF_CANONICAL_BLOCK_HASHES: [u8; 8] => [u8; 32]
// 2. CF_BLOCK_NUMBERS:          [u8; 32] => [u8; 8]
// 3. CF_HEADERS:                [u8; 32] => BlockHeaderRLP
// 4. CF_BODIES:                 [u8; 32] => BlockBodyRLP
// 5. CF_ACCOUNT_CODES:          [u8; 32] => AccountCodeRLP
// 6. CF_STATE_TRIE_NODES:       [u8; 32] => Vec<u8>
// 7. CF_STORAGE_TRIE_NODES:     [u8; 64] => Vec<u8>
//
// GethDB Data:
// Hashes: either ancient/chain/hashes.{meta,cidx,cdat} or by querying
// ['h' || block_num || 'n'] in the statedb.
// Headers: either ancient/chain/headers.{meta,cidx,cdat} or
// 'h'-prefixed values in statedb. Note the full key there is
// ['h' || block_num || header_hash], where the header_hash can be
// determined as stated above.
// Bodies: either ancient/chain/bodies.{meta,cidx,cdat} or 'b'-prefixed
// values in statedb. We shall use the same mechanism as for headers,
// actually reusing the computed hash.
// Trie Nodes: only in statedb, with only the hash as key.
// Bytecodes: in statedb, ['c' || code_hash ].
use clap::Parser;
use ethrex_common::types::BlockHeader;
use ethrex_common::types::BlockNumber;
use ethrex_common::utils::keccak;
use ethrex_common::{
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::AccountState,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::{Node, NodeHash, Trie, TrieDB, TrieError};
use eyre::OptionExt;
use rocksdb::Cache;
use rocksdb::Env;
use rocksdb::WaitForCompactOptions;
use rocksdb::{DBWithThreadMode, Options, SingleThreaded};
use std::collections::BTreeSet;
use std::fs::File;
use std::io;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Instant;
use tracing::debug;
use tracing::info;
use tracing::warn;
use tracing_subscriber::FmtSubscriber;

const BLOCK_HASH_LOOKUP_DEPTH: u64 = 128;
static GETH_DB_PATH: LazyLock<&str> = LazyLock::new(|| Args::parse().input_dir.leak());
static ETHREX_DB_PATH: LazyLock<&str> = LazyLock::new(|| Args::parse().output_dir.leak());

pub fn main() -> eyre::Result<()> {
    let args = Args::parse();
    tracing::subscriber::set_global_default(FmtSubscriber::new())
        .expect("setting default subscriber failed");
    // init_datadir(&args.output_dir);
    // let store = open_store(&args.output_dir);
    geth2ethrex(args.block_number)
}

fn geth2ethrex(block_number: BlockNumber) -> eyre::Result<()> {
    let migration_start: Instant = Instant::now();

    info!("Opening Ethrex DB");
    let ethrex_db = open_ethrexdb()?;
    info!("Opening Geth DB");
    let gethdb = GethDB::open()?;
    info!("Query hash table");
    let hashes = gethdb.read_hashes_from_gethdb(
        block_number.saturating_sub(BLOCK_HASH_LOOKUP_DEPTH),
        block_number,
    )?;
    let block_hash = *hashes.last().ok_or_eyre("missing block hash")?;
    info!("Query block data");
    let [header_rlp, body_rlp] = gethdb.read_block_from_gethdb(block_number, block_hash)?;
    let header: BlockHeader = RLPDecode::decode(&header_rlp)?;

    info!("Inserting block header");
    let block_hash_rlp = block_hash.encode_to_vec();
    ethrex_db.put_cf(
        ethrex_db.cf_handle("headers").unwrap(),
        block_hash_rlp.clone(),
        header_rlp,
    )?;
    info!("Inserting block body");
    ethrex_db.put_cf(
        ethrex_db.cf_handle("bodies").unwrap(),
        block_hash_rlp,
        body_rlp,
    )?;
    info!("Inserting canonical hashes");
    let headers_cf = ethrex_db.cf_handle("canonical_block_hashes").unwrap();
    for (i, hash) in hashes.into_iter().rev().enumerate() {
        ethrex_db.put_cf(headers_cf, (block_number - i as u64).encode_to_vec(), hash)?;
    }
    info!("Inserting latest block number");
    ethrex_db.put_cf(
        ethrex_db.cf_handle("chain_data").unwrap(),
        4u32.encode_to_vec(),
        block_number.to_be_bytes(),
    )?;

    let mut codes = BTreeSet::new();
    let mut storages = Vec::new();
    let account_triedb = gethdb.triedb()?;
    let account_trie = Trie::open(account_triedb, header.state_root);
    let account_trie_cf = ethrex_db.cf_handle("state_trie_nodes").unwrap();
    info!("Iterating account trie");
    for (path, node) in account_trie.into_iter() {
        if let Node::Leaf(leaf) = &node {
            let state = AccountState::decode(&leaf.value)?;
            if state.code_hash != *EMPTY_KECCACK_HASH {
                codes.insert(state.code_hash);
            }
            if state.storage_root != *EMPTY_TRIE_HASH {
                storages.push((path.to_bytes(), state.storage_root));
            }
        }
        let hash = node.compute_hash();
        ethrex_db.put_cf(account_trie_cf, hash.encode_to_vec(), node.encode_to_vec())?;
    }
    info!("Inserting account codes");
    let code_cf = ethrex_db.cf_handle("account_codes").unwrap();
    for code_hash in codes {
        let code = gethdb
            .read_code(code_hash.0)?
            .ok_or_else(|| eyre::eyre!("missing code hash"))?;
        ethrex_db.put_cf(code_cf, code_hash, code)?;
    }
    info!("Iterating storage tries");
    let storages_cf = ethrex_db.cf_handle("storage_tries_nodes").unwrap();
    for (hashed_address, storage_root) in storages {
        let storage_triedb = gethdb.triedb()?;
        let storage_trie = Trie::open(storage_triedb, storage_root);
        for (_path, node) in storage_trie.into_iter() {
            let hash = node.compute_hash();
            ethrex_db.put_cf(
                storages_cf,
                [&hashed_address[..], &hash.finalize().0].concat(),
                node.encode_to_vec(),
            )?;
        }
    }
    info!("Compacting Ethrex DB");
    ethrex_db.flush_wal(true)?;
    ethrex_db.flush()?;
    ethrex_db.compact_range(Option::<[u8; 0]>::None, Option::<[u8; 0]>::None);
    let migration_time = migration_start.elapsed().as_secs_f64();
    info!("Migration complete in {migration_time} seconds");
    Ok(())
}

fn open_ethrexdb() -> eyre::Result<DBWithThreadMode<SingleThreaded>> {
    // Quick and dirty way to ensure the DB is initialized correctly
    let _ = ethrex_storage::store_db::rocksdb::Store::new((*ETHREX_DB_PATH).as_ref())?;
    let (ethrex_opts, ethrex_cfs) = Options::load_latest(
        *ETHREX_DB_PATH,
        Env::new()?,
        true,
        Cache::new_lru_cache(16 << 20),
    )?;
    let ethrex_db = DBWithThreadMode::<SingleThreaded>::open_cf_descriptors(
        &ethrex_opts,
        *ETHREX_DB_PATH,
        ethrex_cfs,
    )?;
    Ok(ethrex_db)
}

struct GethDB {
    ancient_db_path: PathBuf,
    state_db: DBWithThreadMode<SingleThreaded>,
}
impl GethDB {
    pub fn open() -> eyre::Result<Self> {
        let ancient_db_path = AsRef::<Path>::as_ref(*GETH_DB_PATH).join("ancient/chain");
        // let mut opts = Options::default();
        // opts.create_if_missing(true); // FIXME: just for local testing
        // let state_db = DBWithThreadMode::open(&opts, gethdb_dir)?;
        let state_db =
            DBWithThreadMode::open_for_read_only(&Options::default(), *GETH_DB_PATH, false)?;
        Ok(Self {
            ancient_db_path,
            state_db,
        })
    }

    // NOTES:
    // - Metadata seems to only be useful in write-mode or for recovery.
    //   We'll probably fail quickly with or without recovery, so at least
    //   for now we'll skip it.
    fn read_from_freezer_table(
        &self,
        name: &str,
        compressed: bool,
        first: u64,
        last: u64,
    ) -> eyre::Result<Vec<Vec<u8>>> {
        let idx_path = self.ancient_db_path.join(format!(
            "{name}.{}",
            if compressed { "cidx" } else { "ridx" }
        ));
        let index_file = File::open(&idx_path)?;
        let size = index_file.metadata()?.size();
        if first * 6 >= size {
            return Ok(Vec::new());
        }
        // We need one index back to find the start of the entries.
        let to_read = if last * 6 >= size {
            (size - first * 6) as usize
        } else {
            ((last - first + 2) * 6) as usize
        };
        warn!(
            path = idx_path.to_string_lossy().to_string(),
            size, first, last, to_read, "About to read index file"
        );
        let mut index_buf = vec![0; to_read];
        index_file.read_exact_at(&mut index_buf, 6 * first)?;
        let index_entries: Vec<_> = index_buf
            .chunks_exact(6)
            .map(|entry_bytes| {
                let fnum_bytes = entry_bytes[..2].try_into().unwrap();
                let offs_bytes = entry_bytes[2..].try_into().unwrap();
                (
                    u16::from_be_bytes(fnum_bytes),
                    u32::from_be_bytes(offs_bytes),
                )
            })
            .collect();
        if !index_entries.is_sorted() {
            eyre::bail!("broken index")
        }
        let (Some((first_file, _)), Some((last_file, _))) = (
            index_entries.first().copied(),
            index_entries.last().copied(),
        ) else {
            return Ok(Vec::new());
        };
        let data_files: Vec<_> = (first_file..=last_file)
            .map(|f| {
                File::open(self.ancient_db_path.join(format!(
                    "{name}.{f:04}.{}",
                    if compressed { "cdat" } else { "rdat" }
                )))
            })
            .collect::<io::Result<Vec<_>>>()?;
        let mut data = Vec::with_capacity((last - first + 1) as usize);
        let (mut pnum, mut poff) = index_entries[0];
        for (fnum, offset) in index_entries.into_iter().skip(1) {
            if pnum != fnum {
                poff = 0;
            }
            let file = &data_files[(fnum - first_file) as usize];
            let mut entry_data = vec![0; (offset - poff) as usize];
            file.read_exact_at(&mut entry_data, poff as u64)?;
            if compressed {
                entry_data = snap::raw::Decoder::new().decompress_vec(&entry_data)?;
            }
            (pnum, poff) = (fnum, offset);
            data.push(entry_data);
        }
        Ok(data)
    }

    fn try_read_hashes_from_freezer(&self, first: u64, last: u64) -> eyre::Result<Vec<[u8; 32]>> {
        let hashes_vecs = self.read_from_freezer_table("hashes", false, first, last)?;
        hashes_vecs
            .into_iter()
            .map(|h| <[u8; 32]>::try_from(h.as_slice()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| eyre::eyre!("bad length"))
    }
    fn try_read_hashes_from_statedb(&self, first: u64, last: u64) -> eyre::Result<Vec<[u8; 32]>> {
        // ['h' || block_num || 'n']
        let keys = (first..=last).map(|i| [&b"h"[..], &i.to_be_bytes(), b"n"].concat());
        let mut values = Vec::with_capacity((last - first + 1) as usize);
        for hash in self.state_db.multi_get(keys) {
            let Some(hash) = hash? else {
                continue;
            };
            values.push(hash.try_into().unwrap())
        }
        Ok(values)
    }

    // It is valid for the block to not be in the freezer, but if it's not in the statedb either it's an error
    fn try_read_block_from_freezer(&self, block_num: u64) -> eyre::Result<[Option<Vec<u8>>; 2]> {
        let mut header = self.read_from_freezer_table("headers", true, block_num, block_num)?;
        let mut body = self.read_from_freezer_table("bodies", true, block_num, block_num)?;
        Ok([header.drain(..).next(), body.drain(..).next()])
    }
    fn try_read_block_from_statedb(
        &self,
        block_num: u64,
        block_hash: [u8; 32],
        key_prefixes: &[u8],
    ) -> eyre::Result<[Option<Vec<u8>>; 2]> {
        // ['h' || block_num || header_hash]
        let keys = key_prefixes
            .iter()
            .map(|p| [&[*p][..], &block_num.to_be_bytes(), &block_hash].concat());
        let mut values = self.state_db.multi_get(keys);
        debug_assert_eq!(values.len(), 2);
        if values.len() != 2 {
            eyre::bail!("rocksdb returned an unexpected number of results");
        }
        let mut values = values.drain(..);
        Ok([values.next().unwrap()?, values.next().unwrap()?])
    }

    pub fn read_hashes_from_gethdb(&self, first: u64, last: u64) -> eyre::Result<Vec<[u8; 32]>> {
        let frozen_hashes = self.try_read_hashes_from_freezer(first, last)?;
        let state_hashes = if last - first + 1 != frozen_hashes.len() as u64 {
            self.try_read_hashes_from_statedb(first + frozen_hashes.len() as u64, last)?
        } else {
            Vec::new()
        };
        Ok([frozen_hashes, state_hashes].concat())
    }
    pub fn read_block_from_gethdb(
        &self,
        block_num: u64,
        block_hash: [u8; 32],
    ) -> eyre::Result<[Vec<u8>; 2]> {
        match self.try_read_block_from_freezer(block_num)? {
            [None, None] => {
                let [Some(header), Some(body)] =
                    self.try_read_block_from_statedb(block_num, block_hash, b"hb")?
                else {
                    eyre::bail!("missing body and/or header")
                };
                Ok([header, body])
            }
            [Some(header), None] => {
                let [None, Some(body)] =
                    self.try_read_block_from_statedb(block_num, block_hash, b"b")?
                else {
                    eyre::bail!("missing body")
                };
                Ok([header, body])
            }
            [None, Some(body)] => {
                let [Some(header), None] =
                    self.try_read_block_from_statedb(block_num, block_hash, b"h")?
                else {
                    eyre::bail!("missing header")
                };
                Ok([header, body])
            }
            [Some(header), Some(body)] => Ok([header, body]),
        }
    }

    pub fn read_code(&self, code_hash: [u8; 32]) -> Result<Option<Vec<u8>>, rocksdb::Error> {
        self.state_db.get([&b"c"[..], &code_hash].concat())
    }

    pub fn triedb(&self) -> eyre::Result<Box<dyn TrieDB>> {
        let trie_db =
            DBWithThreadMode::open_for_read_only(&Options::default(), self.state_db.path(), false)?;
        Ok(GethTrieDBWithNodeBuckets::with_db(trie_db))
    }
}

struct GethTrieDBWithNodeBuckets {
    db: DBWithThreadMode<SingleThreaded>,
}
impl GethTrieDBWithNodeBuckets {
    pub fn with_db(db: DBWithThreadMode<SingleThreaded>) -> Box<dyn TrieDB> {
        Box::new(Self { db })
    }
}

impl TrieDB for GethTrieDBWithNodeBuckets {
    fn get(&self, hash: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let hash = hash.finalize().0;
        let Some(value) = self
            .db
            .get(hash)
            .map_err(|e| TrieError::DbError(anyhow::anyhow!("node get failed: {e}")))?
        else {
            warn!(
                hash = format!(
                    "{:032x}{:032x}",
                    u128::from_be_bytes(hash[..16].try_into().unwrap()),
                    u128::from_be_bytes(hash[16..].try_into().unwrap())
                ),
                "MISSING TRIE NODE"
            );
            return Ok(None);
        };
        debug_assert!(value.len() <= u16::MAX as usize);
        debug_assert_eq!(keccak(&value).0, hash);

        // We use a redundant encoding, but I'm not changing it in this tool.
        // For now, decode and then encode.
        let node = Node::decode_raw(&value)?;
        let encoded = node.encode_to_vec();
        Ok(Some(encoded))
    }
    fn put_batch(&self, _key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        unimplemented!()
    }
}

#[derive(Parser)]
struct Args {
    #[arg(
        required = true,
        value_name = "NUMBER",
        help = "Block number to sync to"
    )]
    block_number: BlockNumber,
    #[arg(
        required = true,
        long = "input_dir",
        value_name = "INPUT_DIRECTORY",
        help = "Receives the name of the directory where the State Dump will be read from."
    )]
    pub input_dir: String,
    #[arg(
        required = true,
        long = "output_dir",
        value_name = "OUTPUT_DIRECTORY",
        help = "Receives the name of the directory where the State Dump will be written to."
    )]
    pub output_dir: String,
}
