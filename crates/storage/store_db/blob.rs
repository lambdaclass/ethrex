use ethrex_common::{
    constants::{EMPTY_KECCACK_HASH, EMPTY_TRIE_HASH},
    types::AccountState,
};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};
use tracing::{info, warn};

use bytes::{Buf, Bytes};
use ethrex_common::{H256, types::AccountUpdate};
use ethrex_trie::{
    BranchNode, ExtensionNode, LeafNode, Nibbles, Node, NodeHandle, NodeHash, NodeRef, Trie,
    TrieDB, TrieError,
};
use memmap2::{Mmap, MmapOptions};
use sha3::{Digest, Keccak256};

use crate::{AccountUpdatesList, UpdateBatch, error::StoreError};

/// BlobDbEngine
///
/// This is a log-like storage with external indexing, used for state tries,
/// storage tries and bytecodes.
/// The writer is a handle to the file itself, open in append mode.
/// The reader is a read-only memory-map wrapped in a `Bytes` instance for
/// easy, cheap slicing and zero-copy cloning.
///
/// Concurrency control is done via MVCC: whenever a read transaction starts,
/// the reader is cloned (zero-copy), keeping alive an old mapping. Because
/// the database is append only, its length defines the version, and no entry
/// is ever invalidated later on. When a write transaction happens, a batch
/// of data is written and flushed to disk, the reader replaced with a new
/// mapping of the whole file and the main database indices updated, in that
/// order.
///
/// The external index should be another storage engine that keeps track of:
/// - The length of the blob file, as it represents its transactions;
/// - The offsets of roots of state tries in the blob file;
/// - The offsets of bytecodes in the blob file.
///
/// ACID properties:
/// - Atomicity: ensured by append-only nature and external tracking of
///   transactions in a separate ACID database and by multi-versioning;
/// - Consistency: the map only gets updated after a transaction is fully
///   flushed and the DB supports only a single writer, so it necessarily
///   reads the latest version when updating data;
/// - Isolation: each reader has its own copy of the map at the time of
///   transaction start and the single writer can make sure no new versions
///   were appended with the mutex taken;
/// - Durability: the main DB index is only updated after the last write
///   is flushed to disk, and the transaction only reports success after
///   the main DB commits.
#[derive(Debug)]
pub struct BlobDbEngine {
    writer: Mutex<File>,
    reader: Mutex<Bytes>,
}

pub struct BlobDbRoTxn {
    reader: Bytes,
    root: NodeHandle,
}

pub struct BlobDbRwTxn<'w> {
    ro: BlobDbRoTxn,
    guard: MutexGuard<'w, File>,
    buffer: BufWriter<File>,
}

trait BlobCodec: Sized {
    fn encode(&self, w: impl std::io::Write) -> Result<u64, StoreError>;
    fn decode(bytes: &[u8]) -> Result<Self, StoreError>;
}

impl BlobCodec for Node {
    fn encode(&self, mut w: impl std::io::Write) -> Result<u64, StoreError> {
        use Node::*;
        match self {
            Branch(branch) => {
                // Worst case size: 16 non-inline children, taking 40 bytes each plus u8 with flags
                let mut bytes = [0u8; 645];
                let mut valid = 0u16;
                let mut inline = 0u16;
                for (i, choice) in branch.choices.iter().enumerate() {
                    if !choice.is_valid() || choice.hash == NodeHash::Hashed(*EMPTY_KECCACK_HASH) {
                        continue;
                    }
                    valid |= 1 << i;
                    if matches!(choice.hash, NodeHash::Inline(_)) {
                        inline |= 1 << i;
                    }
                }
                let child_count = valid.count_ones();
                // NOTE: bits 6 and 3 are mutually exclusive, meaning bit 3 is free for use if bit 6 is set
                let flags = 0x80;
                bytes[0] = flags;
                bytes[1..3].copy_from_slice(&valid.to_le_bytes());
                bytes[3..5].copy_from_slice(&inline.to_le_bytes());
                let mut hash_cursor = 5;
                let mut handle_cursor = 5 + 32 * child_count as usize;
                for i in 0..16 {
                    let bit = 1 << i;
                    if valid & bit == 0 {
                        continue;
                    }
                    match inline & bit {
                        0 => {
                            let NodeHash::Hashed(hash) = branch.choices[i].hash else {
                                unreachable!();
                            };
                            bytes[hash_cursor..hash_cursor + 32].copy_from_slice(&hash.0);
                            bytes[handle_cursor..handle_cursor + 8]
                                .copy_from_slice(&branch.choices[i].handle.0.to_le_bytes());
                            handle_cursor += 8;
                            // info!(
                            //     child = i,
                            //     hash = hex::encode(hash.0),
                            //     handle = hex::encode(branch.choices[i].handle.0.to_be_bytes()),
                            //     "ENCODE BRANCH"
                            // );
                        }
                        _ => {
                            let NodeHash::Inline((data, len)) = branch.choices[i].hash else {
                                unreachable!();
                            };
                            bytes[hash_cursor..hash_cursor + 31].copy_from_slice(&data);
                            bytes[hash_cursor + 31] = len;
                            // info!(
                            //     child = i,
                            //     data = hex::encode(&data[..len as usize]),
                            //     handle = hex::encode(branch.choices[i].handle.0.to_be_bytes()),
                            //     "ENCODE BRANCH"
                            // );
                        }
                    }
                    hash_cursor += 32;
                }
                w.write_all(&bytes[..handle_cursor])
                    .map_err(|e| StoreError::Custom(format!("io error: {e}")))?;
                Ok(handle_cursor as u64)
            }
            Extension(ext) => {
                let mut bytes = [0u8; 106];
                let flags = 0x60;
                let encoded_path = ext.prefix.encode_compact();
                bytes[0] = flags;
                bytes[1] = encoded_path.len() as u8;
                let NodeHash::Hashed(hash) = ext.child.hash else {
                    unreachable!()
                };
                bytes[2..34].copy_from_slice(&hash.0);
                bytes[34..42].copy_from_slice(&ext.child.handle.0.to_le_bytes());
                bytes[42..42 + encoded_path.len()].copy_from_slice(&encoded_path);
                w.write_all(&bytes[..42 + encoded_path.len()])
                    .map_err(|e| StoreError::Custom(format!("io error: {e}")))?;
                // info!(
                //     length = hex::encode((encoded_path.len() + 42).to_be_bytes()),
                //     data = hex::encode(&bytes[..42 + encoded_path.len()]),
                //     "ENCODE EXTENSION"
                // );
                Ok(encoded_path.len() as u64 + 42)
            }
            // TODO: add link field to leaf, for optionally linking to a separate trie.
            // That way we can express the fact that an account can own a storage trie.
            Leaf(leaf) => {
                let mut bytes = [0u8; 180];
                let flags = leaf.link.is_some() as u8 * 0x20;
                let encoded_path = leaf.partial.encode_compact();
                bytes[0] = flags;
                bytes[1] = encoded_path.len() as u8;
                bytes[2] = leaf.value.len() as u8;
                let mut cursor = 3;
                if let Some(handle) = leaf.link {
                    bytes[cursor..cursor + 8].copy_from_slice(&handle.0.to_le_bytes());
                    cursor += 8;
                }
                bytes[cursor..cursor + encoded_path.len()].copy_from_slice(&encoded_path);
                cursor += encoded_path.len();
                bytes[cursor..cursor + leaf.value.len()].copy_from_slice(&leaf.value);
                cursor += leaf.value.len();
                w.write_all(&bytes[..cursor])
                    .map_err(|e| StoreError::Custom(format!("io error: {e}")))?;
                // info!(
                //     length = hex::encode(cursor.to_be_bytes()),
                //     data = hex::encode(&bytes[..cursor]),
                //     "ENCODE LEAF"
                // );
                Ok(cursor as u64)
            }
        }
    }

    fn decode(bytes: &[u8]) -> Result<Self, StoreError> {
        #[inline(always)]
        fn to_fixed<const N: usize>(bytes: &[u8]) -> [u8; N] {
            debug_assert_eq!(bytes.len(), N);
            unsafe { *bytes.as_ptr().cast() }
        }

        let len = bytes.len();
        if len < 1 {
            return Err(StoreError::Custom("empty buffer".to_string()));
        }
        let flags = bytes[0];
        match flags {
            0x80 => {
                if len < 5 {
                    return Err(StoreError::Custom("missing branch flags".to_string()));
                }
                let valid = u16::from_le_bytes(to_fixed(&bytes[1..3]));
                let inline = u16::from_le_bytes(to_fixed(&bytes[3..5]));
                let child_count = valid.count_ones() as usize;
                let inline_count = inline.count_ones() as usize;
                if len < 5 + child_count * 40 - inline_count * 8 {
                    return Err(StoreError::Custom("missing branch children".to_string()));
                }
                let mut children: [NodeRef; 16] = std::array::from_fn(|_| NodeRef::default());
                let mut hash_cursor = 5;
                let mut handle_cursor = 5 + child_count * 32;
                // info!(
                //     child_count,
                //     inline_count,
                //     children = hex::encode(valid.to_be_bytes()),
                //     inline = hex::encode(inline.to_be_bytes()),
                //     "DECODE BRANCH"
                // );
                for i in 0..16 {
                    let bit = 1 << i;
                    if bit & valid == 0 {
                        // info!(child = i, status = "SKIP", "DECODE BRANCH");
                        continue;
                    }
                    if bit & inline != 0 {
                        // info!(child = i, status = "INLINE", "DECODE BRANCH");
                        children[i].hash = NodeHash::Inline((
                            to_fixed(&bytes[hash_cursor..hash_cursor + 31]),
                            bytes[hash_cursor + 31],
                        ));
                    } else {
                        children[i].hash =
                            NodeHash::Hashed(H256(to_fixed(&bytes[hash_cursor..hash_cursor + 32])));
                        children[i].handle = NodeHandle(u64::from_le_bytes(to_fixed(
                            &bytes[handle_cursor..handle_cursor + 8],
                        )));
                        // info!(
                        //     child = i,
                        //     hash = hex::encode(children[i].hash.finalize()),
                        //     handle = hex::encode(children[i].handle.0.to_be_bytes()),
                        //     status = "HASHED",
                        //     "DECODE BRANCH"
                        // );
                        handle_cursor += 8;
                    }
                    hash_cursor += 32;
                }
                Ok(Node::Branch(Box::new(BranchNode::new(children))))
            }
            0x60 => {
                if len < 42 {
                    return Err(StoreError::Custom("missing extension child".to_string()));
                }
                let encoded_path_len = bytes[1] as usize;
                let hash = H256(to_fixed(&bytes[2..34])).into();
                let handle = NodeHandle(u64::from_le_bytes(to_fixed(&bytes[34..42])));
                if len < 42 + encoded_path_len {
                    return Err(StoreError::Custom("missing extension prefix".to_string()));
                }
                let decoded_path = Nibbles::decode_compact(&bytes[42..42 + encoded_path_len]);
                // info!(
                //     prefix = hex::encode(&decoded_path),
                //     child_hash = hex::encode(hash),
                //     child_handle = hex::encode(handle.0.to_be_bytes()),
                //     "DECODE EXTENSION"
                // );
                Ok(Node::Extension(ExtensionNode {
                    child: NodeRef {
                        value: None,
                        hash,
                        handle,
                    },
                    prefix: decoded_path,
                }))
            }
            0x00 | 0x20 => {
                /* decode leaf */
                let metadata_len = 3 + 8 * (flags == 0x20) as usize;
                if len < metadata_len {
                    return Err(StoreError::Custom("missing leaf metadata".to_string()));
                }
                let encoded_path_len = bytes[1] as usize;
                let value_len = bytes[2] as usize;
                let handle = (flags == 0x20)
                    .then(|| NodeHandle(u64::from_le_bytes(to_fixed(&bytes[3..11]))));
                if len < metadata_len + encoded_path_len + value_len {
                    return Err(StoreError::Custom("missing leaf data".to_string()));
                }
                let decoded_path =
                    Nibbles::decode_compact(&bytes[metadata_len..metadata_len + encoded_path_len]);
                let value = bytes
                    [metadata_len + encoded_path_len..metadata_len + encoded_path_len + value_len]
                    .to_vec();
                // info!(
                //     value = hex::encode(&value),
                //     partial = hex::encode(&decoded_path),
                //     link = hex::encode(handle.unwrap_or_default().0.to_be_bytes()),
                //     "DECODE LEAF"
                // );
                Ok(Node::Leaf(LeafNode {
                    value,
                    partial: decoded_path,
                    link: handle,
                }))
            }
            _ => Err(StoreError::Custom(format!("invalid flags: {flags:x}"))),
        }
    }
}

impl BlobDbEngine {
    /// Opens a BlobDbEngine.
    ///
    /// path: if Some, the path to the file backing the DB, otherwise the database lives in memory only.
    /// truncate: expected length of the database for file backed DB, with the following cases:
    /// - if path does not exist and truncate is 0, create it; otherwise
    /// - if path does not exist, return an error, as the database is missing; otherwise
    /// - if path points to a file larger than truncate, truncate the file to truncate files before mapping;
    /// - if path points to a file smaller than truncate, return an error as the database is inconsistent.
    /// Caller is expected to query the truncate value from the main database, defaulting to 0 if missing.
    pub fn open(path: Option<impl AsRef<Path>>, truncate: u64) -> Result<Self, StoreError> {
        let writer = match path {
            Some(path) => File::options()
                .read(true)
                .append(true)
                .create(true)
                .open(path),
            None => tempfile::tempfile(),
        }
        .map_err(|e| StoreError::Custom(format!("open error: {e}")))?;
        let reader = unsafe { MmapOptions::new().populate().map(&writer).expect("") };
        Ok(Self {
            writer: Mutex::new(writer),
            reader: Mutex::new(Bytes::from_owner(reader)),
        })
    }
    pub fn open_state_trie(
        &self,
        root_hash: H256,
        root_handle: NodeHandle,
    ) -> Result<Trie, StoreError> {
        let trie_db = BlobDbRoTxn {
            reader: self.reader.lock().expect("").clone(),
            root: root_handle,
        };
        Ok(Trie::open(Box::new(trie_db), root_hash, root_handle))
    }
    pub fn open_storage_trie(
        &self,
        state_root_hash: H256,
        state_root_handle: NodeHandle,
        account_hash: H256,
    ) -> Result<Trie, StoreError> {
        let state_trie = self.open_state_trie(state_root_hash, state_root_handle)?;
        let Some(account_node) = state_trie
            .db()
            .get_path(Nibbles::from_bytes(&account_hash.0))?
        else {
            // info!(status = "ACCOUNT NOT FOUND", "OPEN STORAGE TRIE");
            return Ok(Trie::stateless());
        };
        let Node::Leaf(account_leaf) = account_node else {
            // info!(status = "NOT A LEAF", "OPEN STORAGE TRIE");
            return Err(StoreError::Trie(TrieError::InconsistentTree));
        };
        let account_state: AccountState =
            ethrex_rlp::decode::RLPDecode::decode(&account_leaf.value)?;
        let Some(storage_root_handle) = account_leaf.link else {
            // info!(
            //     value = hex::encode(account_leaf.value),
            //     storage_root = hex::encode(account_state.storage_root),
            //     status = "MISSING LINK",
            //     "OPEN STORAGE TRIE"
            // );
            debug_assert_eq!(account_state.storage_root, *EMPTY_TRIE_HASH);
            return Ok(Trie::stateless());
        };
        debug_assert_ne!(account_state.storage_root, *EMPTY_TRIE_HASH);
        // info!(
        //     value = hex::encode(account_leaf.value),
        //     storage_root = hex::encode(account_state.storage_root),
        //     link = hex::encode(storage_root_handle.0.to_be_bytes()),
        //     status = "FOUND",
        //     "OPEN STORAGE TRIE"
        // );
        let trie_db = BlobDbRoTxn {
            reader: self.reader.lock().expect("").clone(),
            root: storage_root_handle,
        };
        Ok(Trie::open(
            Box::new(trie_db),
            account_state.storage_root,
            storage_root_handle,
        ))
    }
    pub fn get_node(&self, _node_handle: NodeHandle) -> Result<Node, StoreError> {
        todo!()
    }
    /// Write a batch of data. Returns a tuple containing:
    /// - The new length of the file, representing a new commit;
    /// - A vector of new state roots' hashes and their offsets;
    /// - A vector of new bytecodes' hashes and their offsets.
    pub fn write_batch(&self, _batch: &UpdateBatch) -> Result<(u64, H256, u64), StoreError> {
        todo!()
    }

    // Main state update method
    pub async fn apply_account_updates(
        &self,
        state_root_hash: H256,
        state_root_handle: NodeHandle,
        // Account address ordered
        // FIXME: pass the vector or btree
        mut account_updates: Vec<AccountUpdate>,
    ) -> Result<Option<AccountUpdatesList>, StoreError> {
        if account_updates.is_empty() {
            return Ok(None);
        }
        // MAIN
        // 2025-08-29T02:05:19.169147Z  INFO ethrex::cli: Adding block 1 with hash 0xc62962979a16131910a073095eadc7e458679bb9336299cd38a6e715c1c47996.
        // 2025-08-29T02:05:19.172120Z  WARN ethrex_storage::store: APPLY UPDATES address="000f3df6d732807ef1319fb7b8bb8522d0beac02" hashed_address="37d65eaa92c6bc4c13a5ec45527f0c18ea8932588728769ec7aecfe6d9f32e42" removed=false added_storages=2 nonce="" balance=""
        // 2025-08-29T02:05:19.172190Z  WARN ethrex_storage::store: APPLY UPDATES address="000f3df6d732807ef1319fb7b8bb8522d0beac02" hashed_address="37d65eaa92c6bc4c13a5ec45527f0c18ea8932588728769ec7aecfe6d9f32e42" key="0000000000000000000000000000000000000000000000000000000000000ffe" hashed_key="aae374f766d485153473226b4da2d56876b809490800f80c06c8cf5b71d7f70d" value="0000000000000000000000000000000000000000000000000000000067d81124"
        // 2025-08-29T02:05:19.172210Z  WARN ethrex_storage::store: APPLY UPDATES address="000f3df6d732807ef1319fb7b8bb8522d0beac02" hashed_address="37d65eaa92c6bc4c13a5ec45527f0c18ea8932588728769ec7aecfe6d9f32e42" key="0000000000000000000000000000000000000000000000000000000000002ffd" hashed_key="983a5c24b0df4652a344b8405128392afcc06b84869a2e117b3ec399f64745a5" value="376450cd7fb9f05ade82a7f88565ac57af449ac696b1a6ac5cc7dac7d467b7d6"
        // BLOB
        // 2025-08-29T02:22:24.890490Z  WARN ethrex_storage::store_db::blob: APPLY UPDATES address="000f3df6d732807ef1319fb7b8bb8522d0beac02" hashed_address="37d65eaa92c6bc4c13a5ec45527f0c18ea8932588728769ec7aecfe6d9f32e42" removed=false added_storages=2 nonce="" balance=""
        // 2025-08-29T02:22:24.890513Z  WARN ethrex_storage::store_db::blob: APPLY UPDATES address="000f3df6d732807ef1319fb7b8bb8522d0beac02" hashed_address="37d65eaa92c6bc4c13a5ec45527f0c18ea8932588728769ec7aecfe6d9f32e42" key="0000000000000000000000000000000000000000000000000000000000000ffe" hashed_key="aae374f766d485153473226b4da2d56876b809490800f80c06c8cf5b71d7f70d" value="0000000000000000000000000000000000000000000000000000000067d81124"
        // 2025-08-29T02:22:24.890530Z  WARN ethrex_storage::store_db::blob: APPLY UPDATES address="000f3df6d732807ef1319fb7b8bb8522d0beac02" hashed_address="37d65eaa92c6bc4c13a5ec45527f0c18ea8932588728769ec7aecfe6d9f32e42" key="0000000000000000000000000000000000000000000000000000000000002ffd" hashed_key="983a5c24b0df4652a344b8405128392afcc06b84869a2e117b3ec399f64745a5" value="376450cd7fb9f05ade82a7f88565ac57af449ac696b1a6ac5cc7dac7d467b7d6"
        // info!("APPLY UPDATES (BLOBDB)");
        // for update in &account_updates {
        //     warn!(
        //         address = hex::encode(update.address),
        //         hashed_address = hex::encode(Keccak256::digest(update.address)),
        //         removed = update.removed,
        //         added_storages = update.added_storage.len(),
        //         nonce = update
        //             .info
        //             .as_ref()
        //             .map(|i| hex::encode(i.nonce.to_be_bytes()))
        //             .unwrap_or_default(),
        //         balance = update
        //             .info
        //             .as_ref()
        //             .map(|i| hex::encode(i.balance.to_big_endian()))
        //             .unwrap_or_default(),
        //         "APPLY UPDATES"
        //     );
        //     for (k, v) in &update.added_storage {
        //         warn!(
        //             address = hex::encode(update.address),
        //             hashed_address = hex::encode(Keccak256::digest(update.address)),
        //             key = hex::encode(k),
        //             hashed_key = hex::encode(Keccak256::digest(k)),
        //             value = hex::encode(v.to_big_endian()),
        //             "APPLY UPDATES"
        //         );
        //     }
        // }
        let mut writer_lock = self.writer.lock().expect("");
        let writer = &mut *writer_lock;
        let reader = self.reader.lock().expect("").clone();
        let mut offset = reader.len() as u64;

        // Discard any incomplete operation
        writer
            .set_len(offset)
            .map_err(|e| StoreError::Custom(format!("truncate failed: {e}")))?;

        let mut buffer = BufWriter::with_capacity(64 * 1024, writer);

        let mut storage_roots = BTreeMap::new();

        let hashed_addresses: Vec<_> = account_updates
            .iter()
            .map(|u| H256(Keccak256::digest(u.address).into()))
            .collect();

        let mut offsets = Vec::with_capacity(64 * 1024);
        for (update, hashed_address) in account_updates.iter_mut().zip(hashed_addresses.iter()) {
            if update.removed {
                storage_roots.insert(*hashed_address, (*EMPTY_TRIE_HASH, None));
                continue;
            }
            if update.added_storage.is_empty() {
                continue;
            }
            let mut storage_trie =
                if Some(&(*EMPTY_TRIE_HASH, None)) == storage_roots.get(hashed_address) {
                    // info!(
                    //     address = hex::encode(hashed_address),
                    //     stage = "OPEN STORAGE",
                    //     status = "CREATE EMPTY",
                    //     "APPLY UPDATES"
                    // );
                    Trie::stateless()
                } else {
                    // info!(
                    //     address = hex::encode(hashed_address),
                    //     stage = "OPEN STORAGE",
                    //     status = "OPEN FROM DB",
                    //     "APPLY UPDATES"
                    // );
                    self.open_storage_trie(state_root_hash, state_root_handle, *hashed_address)?
                };
            for (storage_key, storage_value) in &update.added_storage {
                let hashed_key = Keccak256::digest(storage_key.0).to_vec();
                if storage_value.is_zero() {
                    // info!(
                    //     address = hex::encode(hashed_address),
                    //     key = hex::encode(&hashed_key),
                    //     status = "REMOVE STORAGE",
                    //     "APPLY UPDATES"
                    // );
                    storage_trie.remove(hashed_key)?;
                } else {
                    // info!(
                    //     address = hex::encode(hashed_address),
                    //     key = hex::encode(&hashed_key),
                    //     value = hex::encode(storage_value.to_big_endian()),
                    //     status = "INSERT STORAGE",
                    //     "APPLY UPDATES"
                    // );
                    storage_trie.insert(
                        hashed_key,
                        ethrex_rlp::encode::RLPEncode::encode_to_vec(storage_value),
                    )?;
                }
            }
            let (storage_hash, mut storage_updates) =
                storage_trie.collect_changes_since_last_hash();
            for update in storage_updates.iter_mut() {
                // info!(offset = offset, "UPDATE REF");
                offsets.push(NodeHandle(offset));
                let node = Arc::make_mut(update.value.as_mut().expect(""));
                update_references(&offsets, node);
                offset += node.encode(&mut buffer)?;
            }
            storage_roots.insert(*hashed_address, (storage_hash, offsets.last().cloned()));
            offsets.clear();
        }
        let mut state_trie = self.open_state_trie(state_root_hash, state_root_handle)?;
        for (update, hashed_address) in account_updates.iter().zip(hashed_addresses.iter()) {
            let hashed_address_vec = hashed_address.0.to_vec();
            if update.removed {
                // Remove account from trie
                state_trie.remove(hashed_address_vec)?;
                continue;
            }
            // Add or update AccountState in the trie
            // Fetch current state or create a new state to be inserted
            let (mut account_state, mut link): (AccountState, _) = match state_trie
                .db()
                .get_path(Nibbles::from_bytes(&hashed_address_vec))?
            {
                Some(Node::Leaf(leaf)) => (
                    ethrex_rlp::decode::RLPDecode::decode(&leaf.value)?,
                    leaf.link,
                ),
                Some(_) => unreachable!(),
                None => (AccountState::default(), None),
            };
            if let Some(info) = &update.info {
                account_state.nonce = info.nonce;
                account_state.balance = info.balance;
                account_state.code_hash = info.code_hash;
            }
            if let Some(new_root) = storage_roots.get(hashed_address) {
                account_state.storage_root = new_root.0;
                link = new_root.1;
            }
            state_trie.insert_with_link(
                hashed_address_vec,
                ethrex_rlp::encode::RLPEncode::encode_to_vec(&account_state),
                link,
            )?;
        }
        let (state_trie_root_hash, mut state_updates) =
            state_trie.collect_changes_since_last_hash();
        // info!(
        //     root_hash = hex::encode(state_trie_root_hash),
        //     stage = "COLLECTED STATE CHANGES",
        //     "APPLY UPDATES"
        // );
        for update in state_updates.iter_mut() {
            // info!(offset = offset, "UPDATE REF");
            offsets.push(NodeHandle(offset));
            let node = Arc::make_mut(update.value.as_mut().expect(""));
            update_references(&offsets, node);
            offset += node.encode(&mut buffer)?;
        }

        buffer.flush().expect("");
        let writer = buffer.into_inner().expect("");
        writer.sync_data().expect("");
        let map = unsafe { MmapOptions::new().populate().map(&*writer).expect("") };
        let new_reader = Bytes::from_owner(map);
        let len = new_reader.len() as u64;
        *self.reader.lock().expect("") = new_reader;

        let code_updates = account_updates
            .iter()
            .filter_map(|u| u.info.as_ref().map(|i| (i.code_hash, u.code.clone())))
            .filter_map(|(h, c)| c.map(|c| (h, c)))
            .collect();

        Ok(Some(AccountUpdatesList {
            trie_version: len,
            state_trie_root_hash,
            state_trie_root_handle: *offsets.last().expect(""),
            code_updates,
        }))
    }
}

impl BlobDbRoTxn {
    pub fn new_empty() -> Self {
        let engine = BlobDbEngine::open(Option::<String>::None, 0).expect("");
        Self {
            root: NodeHandle(0),
            reader: engine.reader.lock().expect("").clone(),
        }
    }
}

impl TrieDB for BlobDbRoTxn {
    fn get(&self, NodeHandle(offset): NodeHandle) -> Result<Option<Node>, TrieError> {
        if self.reader.len() as u64 <= offset {
            // info!(
            //     handle = hex::encode(offset.to_be_bytes()),
            //     length = hex::encode(self.reader.len().to_be_bytes()),
            //     status = "OOB",
            //     "TRIEDB GET"
            // );
            return Ok(None);
        }
        // info!(
        //     handle = hex::encode(offset.to_be_bytes()),
        //     length = hex::encode(self.reader.len().to_be_bytes()),
        //     status = "TO DECODE",
        //     "TRIEDB GET"
        // );
        let after = &self.reader[offset as usize..];
        let node = Node::decode(after).map_err(|e| TrieError::DbError(anyhow::anyhow!(e)))?;
        Ok(Some(node))
    }
    fn get_path(&self, path: Nibbles) -> Result<Option<Node>, TrieError> {
        // info!(path = hex::encode(&path), "TRIEDB GET PATH");
        use Node::*;

        let mut node = self.get(self.root)?;
        let mut path = &path.data[..];
        while !path.is_empty() && path[0] != 16 {
            let node_type = match node {
                None => "NONE",
                Some(Branch(_)) => "BRANCH",
                Some(Extension(_)) => "EXTENSION",
                Some(Leaf(_)) => "LEAF",
            };
            // info!(
            //     path = hex::encode(path),
            //     node_type = node_type,
            //     "TRIEDB GET PATH"
            // );
            let Some(ref inner) = node else {
                return Ok(None);
            };
            match inner {
                Branch(branch) => {
                    let choice = &branch.choices[path[0] as usize];
                    if !choice.is_valid() || choice.hash == NodeHash::Hashed(*EMPTY_KECCACK_HASH) {
                        // info!(
                        //     path = hex::encode(path),
                        //     node_type = node_type,
                        //     status = "MISSING CHILD",
                        //     "TRIEDB GET PATH"
                        // );
                        return Ok(None);
                    }
                    if let NodeHash::Inline((data, len)) = choice.hash {
                        let Node::Leaf(leaf) =
                            ethrex_rlp::decode::RLPDecode::decode(&data[..len as usize])?
                        else {
                            unreachable!("ONLY LEAF CAN BE INLINE");
                        };
                        if path[1..] != leaf.partial.data {
                            // info!(
                            //     expected = hex::encode(&path[1..]),
                            //     got = hex::encode(&leaf.partial.data),
                            //     status = "WRONG INLINE LEAF",
                            //     "TRIEDB GET PATH"
                            // );
                            return Ok(None);
                        }
                        return Ok(Some(Node::Leaf(leaf)));
                    }
                    node = self.get(choice.handle).expect("INVALID CHILD");
                    path = &path[1..];
                }
                Extension(extension) => {
                    let Some(stripped) = path.strip_prefix(&extension.prefix.data[..]) else {
                        // info!(status = "WRONG EXTENSION", "TRIEDB GET PATH");
                        return Ok(None);
                    };
                    node = self.get(extension.child.handle).expect("INVALID CHILD");
                    path = stripped;
                }
                Leaf(leaf) => {
                    if path != leaf.partial.data {
                        // info!(
                        //     expected = hex::encode(path),
                        //     got = hex::encode(&leaf.partial.data),
                        //     status = "WRONG LEAF",
                        //     "TRIEDB GET PATH"
                        // );
                        return Ok(None);
                    }
                    path = &path[leaf.partial.len()..]; // Ensures next iteration is empty
                }
            }
        }
        Ok(node)
    }
}

// Helpers
fn update_references(offsets: &[NodeHandle], node: &mut Node) {
    match node {
        Node::Branch(b) => {
            for c in &mut b.choices {
                if c.is_dirty() && c.is_valid() && !matches!(c.hash, NodeHash::Inline(_)) {
                    c.handle = offsets[(c.handle.0 ^ (1u64 << 63)) as usize];
                }
            }
        }
        Node::Extension(e) => {
            if e.child.is_dirty() {
                e.child.handle = offsets[(e.child.handle.0 ^ (1u64 << 63)) as usize];
            }
        }
        Node::Leaf(_l) => {
            // Nothing to do, links are created with the correct offset already
        }
    }
}
