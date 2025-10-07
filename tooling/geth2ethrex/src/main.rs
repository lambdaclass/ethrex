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
use rocksdb::{
    DBWithThreadMode, IngestExternalFileOptions, MultiThreaded, Options, SingleThreaded,
    SstFileWriter,
};
use std::collections::HashMap;
use std::fs::{File, read};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::scope;
use std::time::Instant;
use tempfile::{NamedTempFile, tempfile_in};
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

    // TODO: share a cache between DB instances (or not)
    pub fn triedb(
        &self,
        bucket_senders: [SyncSender<([u8; 32], Vec<u8>)>; 16],
    ) -> eyre::Result<Box<dyn TrieDB>> {
        let trie_db =
            DBWithThreadMode::open_for_read_only(&Options::default(), self.state_db.path(), false)?;
        GethTrieDBWithNodeBuckets::new_with_buckets(trie_db, bucket_senders)
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
    bucket_senders: [SyncSender<([u8; 32], Vec<u8>)>; 16],
}
impl GethTrieDBWithNodeBuckets {
    pub fn new_with_buckets(
        db: DBWithThreadMode<SingleThreaded>,
        bucket_senders: [SyncSender<([u8; 32], Vec<u8>)>; 16],
    ) -> eyre::Result<Box<dyn TrieDB>> {
        Ok(Box::new(Self { db, bucket_senders }))
    }
}

impl TrieDB for GethTrieDBWithNodeBuckets {
    fn get(&self, hash: NodeHash) -> Result<Option<Vec<u8>>, TrieError> {
        let hash = hash.finalize().0;
        let value = self.db.get(hash).unwrap().unwrap();
        debug_assert!(value.len() <= u16::MAX as usize);
        debug_assert_eq!(keccak(&value).0, hash);

        // We use a redundant encoding, but I'm not changing it in this tool.
        // For now, decode and then encode.
        let node = Node::decode_raw(&value)?;
        let encoded = node.encode_to_vec();
        // TODO: if contention seems to be a problem, buffer some entries here
        // before sending. Would need a flush at the end and be a bit more code.
        self.bucket_senders[(hash[0] >> 4) as usize]
            .send((hash, encoded.clone()))
            .unwrap();
        Ok(Some(encoded))
    }
    fn put_batch(&self, _key_values: Vec<(NodeHash, Vec<u8>)>) -> Result<(), TrieError> {
        unimplemented!()
    }
}

// Trie handling:
// - State trie is always computed in parallel. The root is processed first,
//   as well as its child if it's an extension, then one worker per choice
//   for the top level branch.
// - Storage tries are first deduplicated by storage root, so we only need
//   to iterate each trie once.
// - Tries with more than ten repetitions are considered small, and we use
//   only external parallelism for those.
// - Tries with no repetitions are assumed big, and are processed sequentially
//   of one another, with internal parallelism like the state trie, and use
//   separate SSTs.
// - In the middle, it's medium tries, they have internal parallelism but
//   share SST files with small tries.
// - We track big tries in memory, as we need to make sure to insert splits
//   between SSTs before and after each of them. Other than that, everything
//   is buffered to disk, and SSTs get split by size target.
// - SST config should match our DB bottom level, which in turn shouldn't be
//   far from Geth's.
// - We need to use ingest behind to force data to the bottom level, as this
//   is the oldest data our node will have, and we don't want unnecessary
//   compaction to slow us down later.
fn big_trie_bucket_worker(
    bucket_receiver: Receiver<([u8; 32], Vec<u8>)>,
) -> eyre::Result<NamedTempFile> {
    // TODO: change return to option internally, empty files can't be added
    // Internally we use extra buckets based on the second nibble to avoid
    // memory use blowing up during sorting in step 2.
    let lvl2_buckets: Vec<_> = (0..16).filter_map(|_| tempfile::tempfile().ok()).collect();
    let mut entries = 0;
    let mut sst_file = NamedTempFile::new_in("./")?;
    let opts = Options::default();
    let mut sst = SstFileWriter::create(&opts);
    sst.open(&sst_file)?;
    {
        // Step 1: accumulate all incoming nodes into files.
        let mut writers: Vec<_> = lvl2_buckets.iter().map(BufWriter::new).collect();
        while let Ok((hash, encoded)) = bucket_receiver.recv() {
            let writer = &mut writers[hash[0] as usize & 0xf];
            writer.write_all(&hash)?;
            writer.write_all(&(encoded.len() as u16).to_ne_bytes())?;
            writer.write_all(&encoded)?;
            entries += 1;
        }
        if entries == 0 {
            return Ok(sst_file); // FIXME: Ok(None)
        }
        writers.iter_mut().try_for_each(BufWriter::flush)?;
    }
    {
        // Step 2: sort the data and write to SST file.
        let mut sort_buffer = Vec::with_capacity(64 << 20);
        let mut meta_buffer = [0u8; 34];
        for mut bucket in lvl2_buckets {
            bucket.seek(SeekFrom::Start(0))?;
            let mut reader = BufReader::new(bucket);
            // TODO: use two vecs, keep the number of entries.
            // With this I can do just two allocs here:
            // 1. Vec for [[u8;32] || offset || len], by N entries;
            // 2. Vec for file size - vec #1 len bytes.
            // Then we sort only the descriptors by hash.
            // We can actually do that for the files to keep memory usage lower,
            // we can read the data after sorting, when we need it.
            // We risk too much IO though.
            // In any case, we could at lest mmap and use or `read_to_end` the
            // second file.
            // TODO: I could reuse this for big storage tries, but it would need
            // at least one of the following changes:
            // 1. Avoid SST creation, as I don't have all the prefixes for this trie yet;
            // 2. Receive one prefix and assume big tries don't have duplicates, stats support this approach;
            // 3. Parameterize the receiver so it can receive the node with its prefix;
            // 4. Ditto, but with all its prefixes.
            // It might be sensible to split phase 1 and 2 so we can change phase 2
            // when needed.
            loop {
                match reader.read_exact(&mut meta_buffer) {
                    Ok(_) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(e) => {
                        eyre::bail!("read error {e}")
                    }
                }
                let hash: [u8; 32] = meta_buffer[..32].try_into().unwrap();
                let len = u16::from_ne_bytes(meta_buffer[32..].try_into().unwrap()) as usize;
                let mut encoded = vec![0; len];
                reader.read_exact(&mut encoded)?;
                sort_buffer.push((hash, encoded));
            }
            sort_buffer.sort_unstable_by_key(|(hash, _)| *hash);
            sort_buffer.dedup_by_key(|(hash, _)| *hash);
            assert!(sort_buffer.is_sorted_by(|a, b| a < b));
            entries += sort_buffer.len();

            let mut last = [0u8; 32];
            for (hash, encoded) in sort_buffer.drain(..) {
                // println!(
                //     "inserting hash: {:032x}{:032x}",
                //     u128::from_be_bytes(hash[..16].try_into().unwrap()),
                //     u128::from_be_bytes(hash[16..].try_into().unwrap())
                // );
                sst.put(hash, encoded).inspect_err(|_| {
                    println!(
                        "failed to insert: {:032x}{:032x} (prev: {:032x}{:032x})",
                        u128::from_be_bytes(hash[..16].try_into().unwrap()),
                        u128::from_be_bytes(hash[16..].try_into().unwrap()),
                        u128::from_be_bytes(last[..16].try_into().unwrap()),
                        u128::from_be_bytes(last[16..].try_into().unwrap()),
                    );
                })?;
                last = hash;
            }
            sort_buffer.clear();
        }
        sst.finish()?;
    }
    sst_file.flush()?;
    Ok(sst_file)
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

    scope(|s| {
        let (account_bucket_senders, account_bucket_receivers): (Vec<_>, Vec<_>) =
            (0..16).map(|_| sync_channel(1_000)).unzip();
        let account_bucket_senders: [_; 16] = account_bucket_senders.try_into().unwrap();
        let gethdbs = (0..16)
            .map(|_| gethdb.triedb(account_bucket_senders.clone()))
            .collect::<eyre::Result<Vec<_>>>()?;
        std::mem::drop(account_bucket_senders);
        let account_worker_handlers: Vec<_> = account_bucket_receivers
            .into_iter()
            .map(|r| {
                s.spawn(|| {
                    let named = big_trie_bucket_worker(r).unwrap();
                    println!(
                        "sst written to: {} ({} bytes)",
                        &named.path().to_string_lossy(),
                        named.as_file().metadata().unwrap().size()
                    );
                })
            })
            .collect();
        // TODO: refactor into function, will need similar logic for storage
        // tries and it's noisy to have here.
        let Some(root) = gethdbs[0].get(header.state_root.into())? else {
            return Ok(());
        };
        let top_branch = match Node::decode(&root)? {
            Node::Leaf(_) => {
                // TODO: extract account data
                return Ok(());
            }
            Node::Extension(ext) => {
                let child = gethdbs[0].get(ext.child.compute_hash())?.unwrap();
                match Node::decode_raw(&child)? {
                    Node::Branch(branch) => branch,
                    _ => eyre::bail!("bad extension"),
                }
            }
            Node::Branch(branch) => branch,
        };
        let (code_bucket_senders, code_bucket_receivers): (Vec<_>, Vec<_>) =
            (0..16).map(|_| sync_channel(1_000)).unzip();
        let code_bucket_senders: [_; 16] = code_bucket_senders.try_into().unwrap();
        gethdbs.into_iter().enumerate().for_each(|(b, db)| {
            let hash = top_branch.choices[b].compute_hash().finalize();
            let code_bucket_senders = code_bucket_senders.clone();
            s.spawn(move || {
                let trie = Trie::open(db, hash);
                for (path, value) in trie.into_iter().content() {
                    let hash: [u8; 32] = path.try_into().unwrap();
                    let account_state = AccountState::decode(&value).unwrap();
                    // TODO: extract into struct and method.
                    let code_hash = account_state.code_hash;
                    if code_hash != *EMPTY_KECCACK_HASH {
                        code_bucket_senders[code_hash.0[0] as usize >> 4]
                            .send(code_hash.0)
                            .unwrap();
                    }
                    // TODO: struct and method to handle the storage trie.
                }
            });
        });
        Ok(())
    })?;

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
