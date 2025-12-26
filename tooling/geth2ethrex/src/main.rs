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
use ethrex_common::H256;
use ethrex_common::types::Code;
use ethrex_common::types::{Block, BlockBody, BlockHeader, BlockNumber};
use ethrex_common::utils::keccak;
use ethrex_common::{
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::AccountState,
};
use ethrex_config::networks::Network;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::decode::decode_bytes;
use ethrex_rlp::decode::decode_rlp_item;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::EngineType;
use ethrex_storage::Store;
use ethrex_trie::Nibbles;
use ethrex_trie::Node;
use ethrex_trie::TrieNode;
use eyre::OptionExt;
use rocksdb::IteratorMode;
use rocksdb::{DBWithThreadMode, Options, SingleThreaded};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::io::Read;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::LazyLock;
use std::time::Instant;
use tracing::info;
use tracing_subscriber::FmtSubscriber;

const BLOCK_HASH_LOOKUP_DEPTH: u64 = 1024;
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
    let store = Store::new(&args.output_dir, EngineType::RocksDB)?;
    geth2ethrex(store, args.block_number, &args)
}

fn geth2ethrex(mut store: Store, block_number: BlockNumber, args: &Args) -> eyre::Result<()> {
    let migration_start: Instant = Instant::now();

    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    rt.block_on(store.add_initial_state(args.network.get_genesis()?))?;

    info!("Opening Geth DB");
    let gethdb = GethDB::open()?;
    info!("Query hash table");

    let hashes = gethdb.read_hashes_from_gethdb(
        block_number.saturating_sub(BLOCK_HASH_LOOKUP_DEPTH),
        block_number,
    )?;
    let block_hash = *hashes.last().ok_or_eyre("missing block hash")?;
    info!("Query block data");
    let [header_rlp, _] = gethdb.read_block_from_gethdb(block_number, block_hash)?;
    let header: BlockHeader = RLPDecode::decode(&header_rlp)?;

    let mut codes = BTreeSet::new();

    let numbers_and_hashes: Vec<_> = hashes
        .iter()
        .rev()
        .enumerate()
        .map(|(i, hash)| (block_number - i as u64, H256::from_slice(hash)))
        .collect();

    if !*VALIDATE_ONLY {
        info!("Inserting blocks");

        for (number, hash) in numbers_and_hashes.iter() {
            let Ok([header_rlp, body_rlp]) =
                gethdb.read_block_from_gethdb(*number, hash.to_fixed_bytes())
            else {
                continue;
            };
            let header: BlockHeader = RLPDecode::decode(&header_rlp)?;
            let body: BlockBody = RLPDecode::decode(&body_rlp)?;
            rt.block_on(store.add_block(Block::new(header, body)))?;
        }
        info!("Inserting canonical hashes");
        rt.block_on(store.forkchoice_update(
            numbers_and_hashes.clone(),
            block_number,
            H256::from_slice(&block_hash),
            None,
            None,
        ))?;

        if !gethdb.is_path_based()? {
            eyre::bail!("hash-based not supported");
        }
        info!("Inserting account state");
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
        let state_trie = store.open_direct_state_trie(*EMPTY_TRIE_HASH)?;
        let mut account_nodes = 0;
        let mut nodes_to_push = vec![];
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
            let node = Node::decode(&v)?;
            let path = Nibbles::from_hex(k[1..].to_vec());
            nodes_to_push.push((path, v.into()));
            if nodes_to_push.len() > 100_000 {
                state_trie
                    .db()
                    .put_batch(std::mem::take(&mut nodes_to_push))?;
            }
            if let Node::Leaf(leaf) = node {
                let state = AccountState::decode(&leaf.value)?;
                debug_assert_ne!(state.code_hash, H256::zero());
                debug_assert_ne!(state.storage_root, H256::zero());
                if state.code_hash != *EMPTY_KECCACK_HASH {
                    codes.insert(state.code_hash);
                }
            }
            account_nodes += 1;
            if account_nodes % 1_000_000 == 0 {
                info!("{account_nodes} account nodes loaded")
            }
        }
        state_trie.db().put_batch(nodes_to_push)?;

        info!("Inserting storage tries");
        let mut storage_nodes = 0;
        let mut storages_to_write: HashMap<H256, Vec<TrieNode>> = Default::default();
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
            let account = H256::from_slice(&k[1..33]);
            let path = Nibbles::from_hex(k[33..].to_vec());
            storages_to_write
                .entry(account)
                .or_default()
                .push((path, v.into()));
            if storage_nodes % 100_000 == 0 {
                rt.block_on(
                    store.write_storage_trie_nodes_batch(storages_to_write.drain().collect()),
                )?;
            }
            storage_nodes += 1;
            if storage_nodes % 1_000_000 == 0 {
                println!("{storage_nodes} storage nodes loaded")
            }
        }
        rt.block_on(store.write_storage_trie_nodes_batch(storages_to_write.drain().collect()))?;

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
        // assert_eq!(disk_layer_root, disk_root); ???
        let disk_layer_id;
        (disk_layer_id, rest) = u64::decode_unfinished(rest)?;
        // FIXME: geth only enforces disk_layer_id <= db_layer_id
        // Not sure if inequality might be actually valid
        // assert_eq!(disk_layer_id, db_layer_id);
        let mut is_list;
        // Decode the disk layer nodes
        let disk_nodes_pl;
        (is_list, disk_nodes_pl, rest) = decode_rlp_item(rest)?;
        decode_nodes(&disk_nodes_pl, &mut diffs)?;
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
            decode_nodes(&difflayer_nodes_pl, &mut diffs)?;
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
        for ((owner, path), node) in diffs {
            match owner {
                ZERO => {
                    if let Ok(Node::Leaf(leaf)) = Node::decode(&node) {
                        let state = AccountState::decode(&leaf.value)?;
                        debug_assert_ne!(state.code_hash, H256::zero());
                        debug_assert_ne!(state.storage_root, H256::zero());
                        if state.code_hash != *EMPTY_KECCACK_HASH {
                            codes.insert(state.code_hash);
                        }
                    }
                    state_trie
                        .db()
                        .put_batch(vec![(Nibbles::from_hex(path), node)])?
                }
                hashed_address => rt.block_on(store.write_storage_trie_nodes_batch(vec![(
                    H256::from_slice(&hashed_address),
                    vec![(Nibbles::from_hex(path), node)],
                )]))?,
            }
        }
        info!(account_nodes, storage_nodes, "All nodes inserted");

        info!("Inserting account codes");

        for code_hash in &codes {
            let code = gethdb
                .read_code(code_hash.0)?
                .ok_or_else(|| eyre::eyre!("missing code hash"))?;
            rt.block_on(store.add_account_code(Code::from_bytecode(code.into())))?;
        }
        let migration_time = migration_start.elapsed().as_secs_f64();
        if args.fkv {
            store.generate_flatkeyvalue()?;
            while store.last_written()? != vec![0xff; 131] {
                std::thread::sleep(std::time::Duration::from_secs(60));
                let current = store.last_written()?;
                info!("FKV generation in progress. Current={current:?}");
            }
            info!("FKV generation complete");
        }
        info!("Migration complete in {migration_time} seconds");
    }

    rt.block_on(store.forkchoice_update(
        numbers_and_hashes,
        block_number,
        H256::from_slice(&block_hash),
        None,
        None,
    ))?;
    rt.block_on(store.load_initial_state())?;
    if true {
        // Run validations
        info!("Running validations");
        let state_root = header.state_root;
        info!("Validating state trie with root {state_root:x}");
        store.open_locked_state_trie(state_root)?.validate()?;
        info!("Validating storage tries and codes");
        for (hashed_address, account_state) in store
            .iter_accounts(state_root)
            .expect("Couldn't open state trie")
        {
            if account_state.storage_root != *EMPTY_TRIE_HASH {
                store
                    .open_locked_storage_trie(
                        hashed_address,
                        state_root,
                        account_state.storage_root,
                    )?
                    .validate()?;
            }
            if account_state.code_hash != *EMPTY_KECCACK_HASH {
                let code = store
                    .get_account_code(account_state.code_hash)?
                    .expect("inserted code not found");
                assert_eq!(account_state.code_hash, keccak(code.bytecode));
            }
        }
        // assert_eq!(storages.len(), with_storage_count);
        info!("Validating latest block");
        assert_eq!(
            rt.block_on(store.get_latest_block_number()).unwrap(),
            block_number
        );
        assert_eq!(
            rt.block_on(store.get_canonical_block_hash(block_number))
                .unwrap()
                .unwrap(),
            block_hash.into(),
        );
        assert_eq!(
            store.get_block_header(block_number).unwrap().unwrap(),
            header
        );
        assert!(store.has_state_root(header.state_root).unwrap());
        info!("Validations finished successfully")
    }

    Ok(())
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

fn decode_nodes(
    buf: &[u8],
    diffs: &mut BTreeMap<([u8; 32], Vec<u8>), Vec<u8>>,
) -> eyre::Result<()> {
    let mut is_list;
    let mut remaining_nodes = buf;
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
            diffs.insert((owner.0, path.to_vec()), node.to_vec());
        }
    }
    Ok(())
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
    #[arg(
        long = "network",
        value_name = "GENESIS_FILE_PATH",
        help = "Receives a `Genesis` struct in json format. You can look at some example genesis files at `fixtures/genesis/*`.",
        long_help = "Alternatively, the name of a known network can be provided instead to use its preset genesis file and include its preset bootnodes. The networks currently supported include holesky, sepolia, hoodi and mainnet. If not specified, defaults to mainnet.",
        help_heading = "Node options",
        env = "ETHREX_NETWORK",
        value_parser = clap::value_parser!(Network),
    )]
    pub network: Network,
    #[arg(
        long = "flatkeyvalue",
        help = "If true, generates FlatKeyValues",
        default_value = "false"
    )]
    pub fkv: bool,
}
