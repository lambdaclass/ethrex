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
use ethrex_common::Bytes;
use ethrex_common::H256;
use ethrex_common::{
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::AccountState,
};
use ethrex_rlp::decode::decode_bytes;
use ethrex_rlp::decode::decode_rlp_item;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::structs::Decoder;
use ethrex_trie::Nibbles;
use ethrex_trie::{Node, NodeHash, Trie, TrieDB, TrieError};
use ethrex_storage::Store;
use eyre::OptionExt;
use rocksdb::Cache;
use rocksdb::Env;
use rocksdb::IteratorMode;
use rocksdb::{DBWithThreadMode, Options, SingleThreaded};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs::File;
use std::io;
use std::io::Read;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Instant;
use tracing::info;
use tracing::warn;
use tracing_subscriber::FmtSubscriber;

const BLOCK_HASH_LOOKUP_DEPTH: u64 = 256;
static GETH_DB_PATH: LazyLock<&str> = LazyLock::new(|| {
    AsRef::<Path>::as_ref(&Args::parse().input_dir)
        .join("chaindata")
        .to_string_lossy()
        .into_owned()
        .leak()
});
static GETH_JOURNAL_PATH: LazyLock<&str> = LazyLock::new(|| {
    AsRef::<Path>::as_ref(&Args::parse().input_dir)
        .join("triedb/merkle.journal")
        .to_string_lossy()
        .into_owned()
        .leak()
});
static ETHREX_DB_PATH: LazyLock<&str> = LazyLock::new(|| Args::parse().output_dir.leak());
static VALIDATE_ONLY: LazyLock<bool> = LazyLock::new(|| Args::parse().validate_only);

pub fn main() -> eyre::Result<()> {
    let args = Args::parse();
    tracing::subscriber::set_global_default(FmtSubscriber::new())
        .expect("setting default subscriber failed");
    // init_datadir(&args.output_dir);
    // let store = open_store(&args.output_dir);
    let store = open_store(&args.output_dir);
    geth2ethrex(args.block_number)
}

fn geth2ethrex(block_number: BlockNumber) -> eyre::Result<()> {
    let migration_start: Instant = Instant::now();

    info!("Opening Geth DB");
    let gethdb = GethDB::open()?;
    info!("Opening ethrex db");
    let store = ethrex_storage::Store::new();
    info!("Query hash table");
    let hashes = gethdb.read_hashes_from_gethdb(
        block_number.saturating_sub(BLOCK_HASH_LOOKUP_DEPTH),
        block_number,
    )?;
    let block_hash = *hashes.last().ok_or_eyre("missing block hash")?;
    info!("Query block data");
    let [header_rlp, body_rlp] = gethdb.read_block_from_gethdb(block_number, block_hash)?;
    let header: BlockHeader = RLPDecode::decode(&header_rlp)?;

    if !*VALIDATE_ONLY {
        info!("Opening Ethrex DB");
        let ethrex_db = open_ethrexdb()?;
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
            &body_rlp,
        )?;
        info!("Inserting canonical hashes");
        let hashes_cf = ethrex_db.cf_handle("canonical_block_hashes").unwrap();
        let numbers_cf = ethrex_db.cf_handle("block_numbers").unwrap();
        for (i, hash) in hashes.iter().rev().enumerate() {
            let hash_rlp = hash.encode_to_vec();
            ethrex_db.put_cf(
                hashes_cf,
                (block_number - i as u64).to_le_bytes(),
                hash_rlp.clone(),
            )?;
            ethrex_db.put_cf(
                numbers_cf,
                hash_rlp,
                (block_number - i as u64).to_le_bytes(),
            )?;
        }
        info!("Inserting latest block number");
        ethrex_db.put_cf(
            ethrex_db.cf_handle("chain_data").unwrap(),
            4u8.encode_to_vec(),
            block_number.to_le_bytes(),
        )?;

        let mut codes = BTreeSet::new();
        let mut storages = BTreeMap::new();
        let account_triedb = gethdb.triedb()?;
        let account_trie = Trie::open(account_triedb, header.state_root);
        let account_trie_cf = ethrex_db.cf_handle("state_trie_nodes").unwrap();
        let storages_cf = ethrex_db.cf_handle("storage_tries_nodes").unwrap();
        let pathbased = gethdb.is_path_based()?;
        if !pathbased {
            eyre::bail!("hash-based not supported");
        }
        info!("Inserting account codes (path-based)");
        let db_root_hash = keccak(gethdb.state_db.get(b"A").unwrap().unwrap());
        let db_layer_id = u64::from_be_bytes(
            gethdb
                .state_db
                .get([&b"L"[..], &db_root_hash.0[..]].concat())
                .unwrap()
                .unwrap()
                .try_into()
                .unwrap(),
        );
        debug_assert_ne!(db_layer_id, 0);
        let mut account_nodes = 0;
        for item in gethdb
            .state_db
            .iterator(IteratorMode::From(b"A", rocksdb::Direction::Forward))
            .take_while(|item| {
                item.as_ref()
                    .map(|(k, _)| k.starts_with(b"A"))
                    .unwrap_or_default()
            })
        {
            let (k, v) = item?;
            debug_assert!(k.len() <= 64);
            debug_assert!(!k.is_empty());
            debug_assert_eq!(k[0], b'A');
            let node = Node::decode_raw(&v)?;
            let node_hash = node.compute_hash();
            let node_rlp = node.encode_to_vec();
            ethrex_db.put_cf(account_trie_cf, node_hash.as_ref(), node_rlp)?;
            if let Node::Leaf(leaf) = node {
                let state = AccountState::decode(&leaf.value)?;
                debug_assert_ne!(state.code_hash, H256::zero());
                debug_assert_ne!(state.storage_root, H256::zero());
                if state.code_hash != *EMPTY_KECCACK_HASH {
                    codes.insert(state.code_hash);
                }
                if state.storage_root != *EMPTY_TRIE_HASH {
                    // NOTE: our iterator already appends the partial path for leaves.
                    let hashed_address = Nibbles::from_hex(k[1..].to_vec())
                        .concat(leaf.partial)
                        .to_bytes();
                    debug_assert_eq!(hashed_address.len(), 32);
                    storages.insert(hashed_address, state.storage_root);
                }
            }
            account_nodes += 1;
        }

        info!("Inserting storage tries (path-based)");
        let mut storage_nodes = 0;
        for item in gethdb
            .state_db
            .iterator(IteratorMode::From(b"O", rocksdb::Direction::Forward))
            .take_while(|item| {
                item.as_ref()
                    .map(|(k, _)| k.starts_with(b"O"))
                    .unwrap_or_default()
            })
        {
            let (k, v) = item?;
            let node = Node::decode_raw(&v)?;
            let node_hash = node.compute_hash();
            let node_rlp = node.encode_to_vec();
            ethrex_db.put_cf(
                storages_cf,
                [&k[1..33], node_hash.as_ref()].concat(),
                node_rlp,
            )?;
            storage_nodes += 1;
        }
        info!("Applying Diff Layers");
        let mut diffs = BTreeMap::new();
        // TODO: the difflayers may also be stores in the DB with key "TrieJournal".
        // We should check there as well, possibly first.
        // TODO: check why disklayer is also journaled and whether or not we need
        // to do something about it.
        // TODO: refactor all of this decoding.
        let mut journal = Vec::new();
        File::open(*GETH_JOURNAL_PATH)?.read_to_end(&mut journal)?;
        let (version, mut rest) = u64::decode_unfinished(&journal)?;
        assert_eq!(version, 3);
        let disk_root;
        (disk_root, rest) = H256::decode_unfinished(rest)?;
        assert_eq!(disk_root, db_root_hash);
        // Decode disk layer
        // Seems redundant, but disk layer stores its root again
        let disk_layer_root;
        (disk_layer_root, rest) = H256::decode_unfinished(rest)?;
        assert_eq!(disk_layer_root, disk_root);
        let disk_layer_id;
        (disk_layer_id, rest) = u64::decode_unfinished(rest)?;
        // FIXME: geth only enforces disk_layer_id <= db_layer_id
        // Not sure if inequality might be actually valid
        assert_eq!(disk_layer_id, db_layer_id);
        let mut is_list;
        // TODO: check if we need to use the disk layer nodes, shouldn't
        // it be always empty given it's supposed to match the DB?
        // Maybe it's mostly for unclean shutdown?
        let _disk_nodes_pl;
        (is_list, _disk_nodes_pl, rest) = decode_rlp_item(rest)?;
        assert!(is_list);
        // Ignore kv field until we use snapshots
        // Raw storage key flag
        (_, rest) = bool::decode_unfinished(rest)?;
        // Accounts
        (is_list, _, rest) = decode_rlp_item(rest)?;
        assert!(is_list);
        // Storages
        (is_list, _, rest) = decode_rlp_item(rest)?;
        assert!(is_list);
        // Decode diff layers
        loop {
            let (difflayer_root, difflayer_block, difflayer_nodes_pl);
            info!(rest = rest.len(), "Starting loop");
            (difflayer_root, rest) = H256::decode_unfinished(rest)?;
            (difflayer_block, rest) = u64::decode_unfinished(rest)?;
            info!(
                difflayer_block,
                difflayer_root = format!(
                    "{:032x}{:032x}",
                    u128::from_be_bytes(difflayer_root.0[..16].try_into().unwrap(),),
                    u128::from_be_bytes(difflayer_root.0[16..].try_into().unwrap())
                ),
                "Decoded difflayer header"
            );
            // difflayer_nodes encoding:
            // 1. nodeSet ([]journalNodes)
            // 2. origin ([]journalNodes)
            // journalNodes = (owner_hash (H256::zero() means account trie), []journalNode)
            // journalNode = (path, node)

            (is_list, difflayer_nodes_pl, rest) = decode_rlp_item(rest)?;
            assert!(is_list);
            // Now we can decode the actual nodes
            let mut remaining_nodes = difflayer_nodes_pl;
            let mut journal_node;
            while !remaining_nodes.is_empty() {
                (is_list, journal_node, remaining_nodes) = decode_rlp_item(remaining_nodes)?;
                assert!(is_list);
                let (owner, nodes) = H256::decode_unfinished(journal_node)?;
                let (is_list, mut node_list, after) = decode_rlp_item(nodes)?;
                assert!(is_list);
                assert!(after.is_empty());
                while !node_list.is_empty() {
                    let (is_list, pathnode, path, node);
                    (is_list, pathnode, node_list) = decode_rlp_item(node_list)?;
                    assert!(is_list);
                    (path, node) = decode_bytes(pathnode)?;
                    let (node, after) = decode_bytes(node)?;
                    assert!(after.is_empty());
                    let node = (!node.is_empty()).then(|| Node::decode_raw(node).unwrap());
                    diffs.insert((owner.0, path.to_vec()), node);
                }
            }
            // Check if it has origin nodes, we ignore them
            let (has_origin, _, next) = decode_rlp_item(rest)?;
            if has_origin {
                rest = next;
            }
            // assert!(is_list);
            // difflayer_kvs encoding:
            // 1. stateSet ([]Storage)
            // 2. origin ([]Storage)
            // Ignore kv field until we use snapshots
            // Raw storage key flag
            (_, rest) = bool::decode_unfinished(rest)?;
            // Accounts
            (is_list, _, rest) = decode_rlp_item(rest)?;
            assert!(is_list);
            // Storages
            (is_list, _, rest) = decode_rlp_item(rest)?;
            assert!(is_list);
            // Check it has origin kvs, we ignore them
            let (has_origin, _, next) = decode_rlp_item(rest)?;
            if has_origin {
                // Accounts
                rest = next;
                // Storages
                (is_list, _, rest) = decode_rlp_item(rest)?;
                assert!(is_list);
            }

            if difflayer_root == header.state_root {
                info!("found root");
                assert!(
                    difflayer_block <= block_number,
                    "state root can never go back in time"
                );
                break;
            }
        }
        const ZERO: [u8; 32] = [0u8; 32];
        for ((owner, _path), node) in diffs {
            match (owner, node) {
                (ZERO, Some(node)) => ethrex_db.put_cf(
                    account_trie_cf,
                    node.compute_hash().as_ref(),
                    node.encode_to_vec(),
                )?,
                (hashed_address, Some(node)) => ethrex_db.put_cf(
                    storages_cf,
                    [hashed_address, node.compute_hash().finalize().0].concat(),
                    node.encode_to_vec(),
                )?,
                (_, None) => (), // Don't need to delete until path-based ethrex
            }
        }
        info!(account_nodes, storage_nodes, "All nodes inserted");

        info!("Inserting account codes");

        let code_cf = ethrex_db.cf_handle("account_codes").unwrap();
        for code_hash in &codes {
            let code = gethdb
                .read_code(code_hash.0)?
                .ok_or_else(|| eyre::eyre!("missing code hash"))?;
            ethrex_db.put_cf(
                code_cf,
                code_hash,
                <[u8] as RLPEncode>::encode_to_vec(&code),
            )?;
        }
        info!("Compacting Ethrex DB");
        ethrex_db.flush_wal(true)?;
        ethrex_db.flush()?;
        ethrex_db.compact_range(Option::<[u8; 0]>::None, Option::<[u8; 0]>::None);
        let migration_time = migration_start.elapsed().as_secs_f64();
        info!("Migration complete in {migration_time} seconds");
        std::mem::drop(ethrex_db);
    }

    if cfg!(debug_assertions) {
        // Run validations
        info!("Running validations");
        let store =
            ethrex_storage::Store::new(*ETHREX_DB_PATH, ethrex_storage::EngineType::RocksDB)?;
        let state_root = header.state_root;
        info!("Validating state trie");
        store.open_locked_state_trie(state_root)?.validate()?;
        info!("Validating storage tries and codes");
        for (hashed_address, account_state) in store
            .iter_accounts(state_root)
            .expect("Couldn't open state trie")
        {
            if account_state.storage_root != *EMPTY_TRIE_HASH {
                store
                    .open_locked_storage_trie(hashed_address, account_state.storage_root)?
                    .validate()?;
            }
            if account_state.code_hash != *EMPTY_KECCACK_HASH {
                let code = store
                    .get_account_code(account_state.code_hash)?
                    .expect("inserted code not found");
                assert_eq!(account_state.code_hash, keccak(code));
            }
        }
        // assert_eq!(storages.len(), with_storage_count);
        info!("Validating latest block");

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert_eq!(
            rt.block_on(store.engine.get_latest_block_number())
                .unwrap()
                .unwrap(),
            block_number
        );
        assert_eq!(
            rt.block_on(store.engine.get_canonical_block_hash(block_number))
                .unwrap()
                .unwrap(),
            block_hash.into(),
        );
        assert_eq!(
            store
                .engine
                .get_block_header(block_number)
                .unwrap()
                .unwrap(),
            header
        );
        assert_eq!(
            rt.block_on(store.engine.get_block_body(block_number))
                .unwrap()
                .unwrap()
                .encode_to_vec(),
            body_rlp
        );
        for (block_offset, block_hash) in hashes.iter().rev().enumerate() {
            let block_number = block_number - (block_offset as u64);
            let block_hash = H256::from_slice(block_hash);
            assert_eq!(
                rt.block_on(store.engine.get_block_number(block_hash))
                    .unwrap()
                    .unwrap(),
                block_number
            );
            assert_eq!(
                rt.block_on(store.engine.get_canonical_block_hash(block_number))
                    .unwrap()
                    .unwrap(),
                block_hash
            );
        }
        info!("Validations finished successfully")
    }

    Ok(())
}

fn open_ethrexdb() -> eyre::Result<DBWithThreadMode<SingleThreaded>> {
    // Quick and dirty way to ensure the DB is initialized correctly
    // TODO: remove this and document why: we need the genesis to be
    // already there or else ethrex is going to overwrite our canonical
    // chain and go and try to sync from genesis.
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

    pub fn is_path_based(&self) -> eyre::Result<bool> {
        Ok(self.state_db.get(b"A")?.is_some())
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
    #[arg(
        short = 'v',
        long = "validate_only",
        help = "If true, runs only validations on Ethrex DB",
        default_value = "false"
    )]
    pub validate_only: bool,
}
