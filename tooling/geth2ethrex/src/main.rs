// GOAL: be ready for full sync from post-merge block in <1h.
// RATIONALE: this roughly correlates to a precomputed version
// of accounts and storages insertions in snap sync, which takes
// ~1.5hs while having to compute all hashes and actually construct
// the trie.
// PLAN:
// The geth2ethrex program takes a sync'd geth archive node and
// initializes an ethrex instance with enough state to full sync
// starting from the provided block number.
// To do that, it loads the LevelDB or PebbleDB as a RocksDB.
// Most of the work can be done with strictly that, but some parts
// require accessing the ancients database, which uses a custom
// format. Specifically, the block body and header and the necessary
// previous hashes are supposedly stored there.
// Once we have the state root we start iterating the account
// trie and getting the nodes for our DB.
// In archive node, Geth stores all trie nodes, both for the world
// state and for each account, as a single mapping with key-value:
// NodeHash => RLP(Node)
// Because of this, we should be able to use our RocksDBTrieDB.
// We would be able to also migrate without decoding, but we need
// to iterate the trie anyway to filter useless nodes, as the state
// can be massive. However, we can do the actual work inside the
// TrieDB if we use a custom one that saves to file buckets the right
// data. That would save us hashing and encoding at least.
// Iteration can be done in parallel by partitioning the keys.
// Results will be constructed to SST files for our RocksDB to ingest.
// Some care is needed to make sure there is no overlap in the final
// SST files, and that they are internally sorted. This allows us to
// import as L1 for maximum performance.
// Storage tries should be processed only once and produced for all
// of the accounts.
// The same is essentially true for bytecode. Bytcode might end up in
// blob files.
//
// To write to the WAL:
// 1. CF_CANONICAL_BLOCK_HASHES: [u8; 8] => [u8; 32]
// 2. CF_BLOCK_NUMBERS:          [u8; 32] => [u8; 8]
// 3. CF_HEADERS:                [u8; 32] => BlockHeaderRLP
// 4. CF_BODIES:                 [u8; 32] => BlockBodyRLP
// To ingest as SST:
// 5. CF_ACCOUNT_CODES:          [u8; 32] => AccountCodeRLP
// 6. CF_STATE_TRIE_NODES:       [u8; 32] => Vec<u8>
// 7. CF_STORAGE_TRIE_NODES:     [u8; 64] => Vec<u8>
//
// 1 and 2 come from a single query to the canonical header hashes,
// most likely in the ancient files.
// 3 and 4 has a single element here, matching the requested block
// number, that must be marked canonical. It also comes most likely
// from ancient.
// 6 comes from iterating the accounts trie for the state root of the
// provided block number, taken from the state DB, and produces the
// inputs to extract 5 and 7, which are deduplicated storage roots
// and bytecode hashes. Bytecode hashes are then simply read and
// re-exported in order, while storage tries need to get the actual
// nodes and then produce the right prefixed keys for our DB for each
// account.
// The choice of whether to produce SSTs or write to WAL comes from
// the observation that 1-4 are very little data, not worth producing
// more immutable files, and faster to load by processing the WAL
// into memtables, while 5-7 is a lot more data and producing it
// already optimized will help full sync proceed faster, with less
// read amplification.
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
//
// Due to the age of the blocks this is expected to be used with, we
// always try the ancients first, and only go thorugh the statedb when
// the data is not in the freezer.
//
// AFTER AN INITIAL WORKING VERSION:
// Migrate to use either Era archives or path-based archive as base,
// as they are friendlier both with sync time and disk usage.
// For now I was working on already synced hash-based archives.
use clap::{ArgGroup, Parser};
use ethrex_common::Address;
use ethrex_common::types::{BlockHash, BlockHeader};
use ethrex_common::utils::keccak;
use ethrex_common::{BigEndianHash, H256, U256, types::BlockNumber};
use ethrex_common::{
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::{AccountState, Block},
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;
use ethrex_trie::{NodeHash, Trie, TrieDB, TrieError};
use eyre::OptionExt;
use rocksdb::{DBWithThreadMode, MultiThreaded, Options, SingleThreaded};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::{debug, info};
use tracing_subscriber::FmtSubscriber;

const BLOCK_HASH_LOOKUP_DEPTH: u64 = 128;

struct GethDB {
    ancient_db_path: PathBuf,
    state_db: DBWithThreadMode<MultiThreaded>,
}
impl GethDB {
    pub fn open(gethdb_dir: impl AsRef<Path>) -> eyre::Result<Self> {
        let ancient_db_path = gethdb_dir.as_ref().join("ancient/chain");
        // let mut opts = Options::default();
        // opts.create_if_missing(true); // FIXME: just for local testing
        // let state_db = DBWithThreadMode::open(&opts, gethdb_dir)?;
        let state_db =
            DBWithThreadMode::open_for_read_only(&Options::default(), gethdb_dir, false)?;
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
        let index_file = File::open(idx_path)?;
        let size = index_file.metadata()?.size();
        let last = last.min(size / 6);
        // We need one index back to find the start of the entries.
        let to_read = ((last - first + 2) * 6) as usize;
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
            println!("reading from {poff} to {offset}");
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
        let values = self.state_db.multi_get(keys);
        println!("{values:?}");
        Ok(Vec::new())
    }

    // It is valid for the block to not be in the freezer, but if it's not in the statedb either it's an error
    fn try_read_block_from_freezer(&self, block_num: u64) -> eyre::Result<[Option<Vec<u8>>; 2]> {
        let mut header = self.read_from_freezer_table("headers", true, block_num, block_num)?;
        let mut body = self.read_from_freezer_table("bodies", true, block_num, block_num)?;
        assert_eq!(header.len(), 1);
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

    pub fn triedb(
        &self,
        bucket_path: impl AsRef<Path>,
        bucket_prefix: &str,
    ) -> eyre::Result<Box<dyn TrieDB>> {
        let trie_db =
            DBWithThreadMode::open_for_read_only(&Options::default(), self.state_db.path(), false)?;
        GethTrieDBWithNodeBuckets::new_with_prefix(trie_db, bucket_path, bucket_prefix)
    }
}

// Our iterator performs two separate but related tasks:
// 1. On one hand, we use it to iterate the key-values of accounts/storages tries,
//    so we can externally classify what we need to ask next;
// 2. On the other hand, we use it to internally save all the nodes to file buckets,
//    based on hash, so we can later sort them and insert them to the SST.
//    For accounts this is direct. For storages we have to do it in steps:
//    1. We get all the tries once, by grouping accounts that share the same trie;
//    2. We prepend to the tries the root (actually, we can do this in the iterator
//       adding a prefix field);
//    3. We sort the nodes by key, meaning (storage_root, storage_slot_hash);
//    4. We convert the back the accounts to be sorted by address_hash and be followed
//       by storage_root;
//    5. We iterate the accounts, locating the storage root by binary search, and
//       replacing the storage root part with the current account hashed address before
//       inserting.
//    We also pre-classify big and small accounts based on the observation that small
//    accounts tend to have more repetitions. Storage roots with fewer than 100 repetitions
//    are partitioned and iterated in parallel internally, while the others don't use
//    use external parallelism (i.e. we choose whether to iterate a single trie in chunks
//    or many tries at the same time).
//    The internal parallelism is easy to achieve by just setting the root_hash to the
//    subtrie I want to iterate. I.e., if I want 16 threads, I extract separately the
//    root node, and then create 16 trie instances with root_hash corresponding to each
//    child, then iterate those in parallel.
//    The buckets need to be reused by thread to simplify and accelerate work.
//    Really big accounts probably need to be bucketed internally as well, taking the node
//    hash instead of the address.
//    Another plausible way to classify the tries by size is to always iterate to the first
//    leaf and extraplote from theri depths: given keys get randomized by keccak, the
//    cardinality of a trie is approximately 16 ^ H, where H is height measured in branch nodes.
//
//    If absolutely necessary, we might implement our own lightweight iterator.
struct GethTrieIterator {}
struct GethTrieDBWithNodeBuckets {
    db: DBWithThreadMode<SingleThreaded>,
    // No love for trait TrieDB
    buckets: [Arc<Mutex<BufWriter<File>>>; 16],
}
impl GethTrieDBWithNodeBuckets {
    pub fn new_with_prefix(
        db: DBWithThreadMode<SingleThreaded>,
        bucket_path: impl AsRef<Path>,
        bucket_prefix: &str,
    ) -> eyre::Result<Box<dyn TrieDB>> {
        let mut buckets = Vec::with_capacity(16);
        for i in 0..16 {
            let f = File::create_new(bucket_path.as_ref().join(format!("{bucket_prefix}.{i:x}")))?;
            buckets.push(Arc::new(Mutex::new(BufWriter::new(f))));
        }
        let buckets = std::array::from_fn(move |i| buckets[i].clone());
        Ok(Box::new(Self { db, buckets }))
    }
    pub fn flush(&mut self) -> eyre::Result<()> {
        for bucket in &mut self.buckets {
            Arc::get_mut(bucket).unwrap().get_mut().unwrap().flush()?;
        }
        Ok(())
    }
}
impl TrieDB for GethTrieDBWithNodeBuckets {
    fn get(&self, hash: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let hash = hash.finalize().0;
        println!(
            "node hash: {:32x}{:32x}",
            u128::from_be_bytes(hash[..16].try_into().unwrap()),
            u128::from_be_bytes(hash[16..].try_into().unwrap())
        );
        let value = self.db.get(hash).unwrap().unwrap();
        debug_assert!(value.len() <= u16::MAX as usize);
        debug_assert_eq!(keccak(&value).0, hash);

        let mut bucket = self.buckets[(hash[0] >> 4) as usize].lock().unwrap();
        let mut buffer = [0u8; 34];
        buffer[..32].copy_from_slice(&hash);
        buffer[32..].copy_from_slice(&(value.len() as u16).to_le_bytes());
        bucket.write_all(&buffer).unwrap();
        bucket.write_all(&value).unwrap();
        println!("value size: {}", value.len());
        Ok(Some(value))
    }
    fn put_batch(&self, _key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        unimplemented!()
    }
}

pub fn geth2ethrex(
    block_number: BlockNumber,
    output_dir: String,
    input_dir: String,
) -> eyre::Result<()> {
    const N_THREADS: usize = 16;
    let migration_start: Instant = Instant::now();

    let gethdb = GethDB::open(input_dir)?;
    let hashes = gethdb.read_hashes_from_gethdb(
        block_number.saturating_sub(BLOCK_HASH_LOOKUP_DEPTH),
        block_number,
    )?;
    let block_hash = *hashes.last().ok_or_eyre("missing block hash")?;
    // TODO: save hashes to ethrex DB and mark canonical
    let [header, body] = gethdb.read_block_from_gethdb(block_number, block_hash)?;
    println!("{} {}", header.len(), body.len());
    // TODO: save header and body to ethrex DB
    // TODO: extract root hash from header
    let header: BlockHeader = RLPDecode::decode(&header)?;
    println!("state root: {}", header.state_root);
    println!("header block number: {}", header.number);
    println!("parent hash: {}", header.parent_hash);
    println!(
        "last hash: {:32x}{:32x}",
        u128::from_be_bytes(block_hash[..16].try_into().unwrap()),
        u128::from_be_bytes(block_hash[16..].try_into().unwrap())
    );

    let trie = Trie::open(
        gethdb.triedb("/tmp", "account_state_bucket.0")?,
        header.state_root,
    );
    let mut iter = trie.into_iter();
    let first_iter = iter.next();
    println!("first iterated node: {first_iter:?}");
    let node_count = iter.count();
    println!("iterated {node_count} nodes");

    let migration_time = migration_start.elapsed().as_secs_f64();
    info!("Migration complete in {migration_time}");
    Ok(())
}

// store.add_block(block).await?;
// store
//     .forkchoice_update(Some(block_hashes), block_number, block_hash, None, None)
//     .await?;

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

pub fn main() -> eyre::Result<()> {
    let args = Args::parse();
    tracing::subscriber::set_global_default(FmtSubscriber::new())
        .expect("setting default subscriber failed");
    // init_datadir(&args.output_dir);
    // let store = open_store(&args.output_dir);
    geth2ethrex(args.block_number, args.output_dir, args.input_dir)
}
