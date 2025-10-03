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
//
// Due to the age of the blocks this is expected to be used with, we
// always try the ancients first, and only go thorugh the statedb when
// the data is not in the freezer.
use clap::{ArgGroup, Parser};
use ethrex_common::Address;
use ethrex_common::types::BlockHash;
use ethrex_common::{BigEndianHash, H256, U256, types::BlockNumber};
use ethrex_common::{
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::{AccountState, Block},
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;
use ethrex_trie::{NodeHash, Trie, TrieDB, TrieError};
use rocksdb::{DBWithThreadMode, MultiThreaded, Options};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::{debug, info};
use tracing_subscriber::FmtSubscriber;

/// Max account dumps to ask for in a single request. The current value matches geth's maximum output.
const MAX_ACCOUNTS: usize = 256;
/// Amount of blocks before the target block to request hashes for. These may be needed to execute the next block after the target block.
const BLOCK_HASH_LOOKUP_DEPTH: u64 = 128;
/// Amount of state dumps to process before updating checkpoint
const DUMPS_BEFORE_CHECKPOINT: usize = 10;

struct GethDB {
    ancient_db_path: PathBuf,
    state_db: DBWithThreadMode<MultiThreaded>,
}
impl GethDB {
    pub fn open(gethdb_dir: impl AsRef<Path>) -> eyre::Result<Self> {
        let ancient_db_path = gethdb_dir.as_ref().join("ancient/chain");
        let state_db =
            DBWithThreadMode::open_for_read_only(&Options::default(), gethdb_dir, false)?;
        Ok(Self {
            ancient_db_path,
            state_db,
        })
    }

    fn try_read_hashes_from_freezer(&self, first: u64, last: u64) -> eyre::Result<Vec<[u8; 32]>> {
        // TODO
        Ok(Vec::new())
    }
    fn try_read_hashes_from_statedb(&self, first: u64, last: u64) -> eyre::Result<Vec<[u8; 32]>> {
        // ['h' || block_num || 'n']
        let keys = (first..=last).map(|i| {
            let mut key = [0u8; 34];
            key[0] = b'h';
            key[33] = b'n';
            key[1..33].copy_from_slice(&i.to_be_bytes());
            key
        });
        let values = self.state_db.multi_get(keys);
        println!("{values:?}");
        Ok(Vec::new())
    }

    // It is valid for the block to not be in the freezer, but if it's not in the statedb either it's an error
    fn try_read_block_from_freezer(
        &self,
        block_num: u64,
        block_hash: [u8; 32],
    ) -> eyre::Result<Option<Block>> {
        eyre::bail!("not implemented yet")
    }
    fn try_read_block_from_statedb(
        &self,
        block_num: u64,
        block_hash: [u8; 32],
    ) -> eyre::Result<Block> {
        todo!()
    }

    pub fn read_hashes_from_gethdb(&self, first: u64, last: u64) -> eyre::Result<Vec<[u8; 32]>> {
        let frozen_hashes = self.try_read_hashes_from_freezer(first, last)?;
        let state_hashes =
            self.try_read_hashes_from_statedb(first + frozen_hashes.len() as u64, last)?;
        Ok([frozen_hashes, state_hashes].concat())
    }
    pub fn read_block_from_gethdb(
        &self,
        block_num: u64,
        block_hash: [u8; 32],
    ) -> eyre::Result<Block> {
        let Some(block) = self.try_read_block_from_freezer(block_num, block_hash)? else {
            return self.try_read_block_from_statedb(block_num, block_hash);
        };
        Ok(block)
    }
    pub fn open_trie(&self, root_hash: [u8; 32]) -> eyre::Result<Trie> {
        todo!()
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
//
//    If absolutely necessary, we might implement our own lightweight iterator.
struct GethTrieIterator {}
struct GethTrieDBWithNodeBuckets {
    db: DBWithThreadMode<MultiThreaded>,
    // No love for trait TrieDB
    buckets: [Arc<Mutex<BufWriter<File>>>; 16],
}
impl GethTrieDBWithNodeBuckets {
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
        let value = self.db.get(hash).unwrap().unwrap();
        debug_assert!(value.len() <= u16::MAX as usize);

        let mut bucket = self.buckets[(hash[0] >> 4) as usize].lock().unwrap();
        let mut buffer = [0u8; 34];
        buffer[..32].copy_from_slice(&hash);
        buffer[32..].copy_from_slice(&(value.len() as u16).to_le_bytes());
        let _ = bucket.write_all(&buffer).unwrap();
        let _ = bucket.write_all(&value).unwrap();
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

    let migration_time = migration_start.elapsed().as_secs_f64();
    info!("Migration complete in {migration_time}");
    Ok(())
}

fn hash_next(hash: H256) -> H256 {
    H256::from_uint(&(hash.into_uint() + 1))
}

// store.add_block(block).await?;
// store
//     .forkchoice_update(Some(block_hashes), block_number, block_hash, None, None)
//     .await?;

#[derive(Parser)]
#[clap(group = ArgGroup::new("input").required(true).args(&["ipc_path", "input_dir"]).multiple(false))]
struct Args {
    #[arg(
        required = true,
        value_name = "NUMBER",
        help = "Block number to sync to"
    )]
    block_number: BlockNumber,
    #[arg(
        long = "input_dir",
        value_name = "INPUT_DIRECTORY",
        help = "Receives the name of the directory where the State Dump will be read from."
    )]
    pub input_dir: String,
    #[arg(
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
