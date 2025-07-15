use super::codec::{
    account_address::AccountAddress, account_info_log_entry::AccountInfoLogEntry,
    account_storage_key_bytes::AccountStorageKeyBytes,
    account_storage_log_entry::AccountStorageLogEntry,
    account_storage_value_bytes::AccountStorageValueBytes, block_num_hash::BlockNumHash,
    encodable_account_info::EncodableAccountInfo,
    flat_tables_block_metadata_key::FlatTablesBlockMetadataKey,
};
use crate::UpdateBatch;
use crate::api::StoreEngine;
use crate::error::StoreError;
use crate::rlp::{
    AccountCodeHashRLP, AccountCodeRLP, AccountHashRLP, AccountStateRLP, BlockBodyRLP,
    BlockHashRLP, BlockHeaderRLP, BlockRLP, PayloadBundleRLP, Rlp, TransactionHashRLP,
    TriePathsRLP, TupleRLP,
};
use crate::store::{MAX_SNAPSHOT_READS, STATE_TRIE_SEGMENTS};
use crate::trie_db::libmdbx::LibmdbxTrieDB;
use crate::trie_db::libmdbx_dupsort::LibmdbxDupsortTrieDB;
use crate::trie_db::utils::node_hash_to_fixed_size;
use crate::utils::{ChainDataIndex, SnapStateIndex};
use bytes::Bytes;
use ethereum_types::{H256, U256};
use ethrex_common::Address;
use ethrex_common::types::{
    AccountInfo, AccountState, Block, BlockBody, BlockHash, BlockHeader, BlockNumber, ChainConfig,
    Index, Receipt, Transaction, payload::PayloadBundle,
};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::error::RLPDecodeError;
use ethrex_trie::{Nibbles, NodeHash, Trie};
use libmdbx::orm::{Decodable, DupSort, Encodable, Table};
use libmdbx::{DatabaseOptions, Mode, PageSize, ReadWriteOptions, TransactionKind};
use libmdbx::{
    dupsort,
    orm::{Database, table},
    table_info,
};
use serde_json;
use std::fmt::{Debug, Formatter};
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// The number of blocks to keep in the state and storage log tables
const KEEP_BLOCKS: u64 = 128;

// Define tables
table!(
    /// The canonical block hash for each block number. It represents the canonical chain.
    ( CanonicalBlockHashes ) BlockNumber => BlockHashRLP
);

table!(
    /// Block hash to number table.
    ( BlockNumbers ) BlockHashRLP => BlockNumber
);

table!(
    /// Block headers table.
    ( Headers ) BlockHashRLP => BlockHeaderRLP
);
table!(
    /// Block bodies table.
    ( Bodies ) BlockHashRLP => BlockBodyRLP
);
table!(
    /// Account codes table.
    ( AccountCodes ) AccountCodeHashRLP => AccountCodeRLP
);

dupsort!(
    /// Account info write log table.
    /// The key maps to two blocks: first, and as seek key, the block corresponding to the final
    /// state after applying the log, and second, the parent of the first block in the range,
    /// that is, the state to which this log should be applied and the state we get back after
    /// rewinding these logs.
    ( AccountsStateWriteLog ) (BlockNumHash, BlockNumHash)[BlockNumHash] => AccountInfoLogEntry
);

dupsort!(
    /// Storage write log table.
    /// The key maps to two blocks: first, and as seek key, the block corresponding to the final
    /// state after applying the log, and second, the parent of the first block in the range,
    /// that is, the state to which this log should be applied and the state we get back after
    /// rewinding these logs.
    ( AccountsStorageWriteLog ) (BlockNumHash, BlockNumHash)[BlockNumHash] => AccountStorageLogEntry
);

type StateTriePruningLogEntry = [u8; 32];
dupsort!(
    /// Trie node insertion logs for pruning.
    /// includes the block number as the search key.
    ( StateTriePruningLog ) BlockNumHash[BlockNumber] => StateTriePruningLogEntry
);

type StorageTriesPruningLogEntry = (StorageTriesNodesSeekKey, StorageTriesNodesSuffixKey);
dupsort!(
    /// Trie node insertion logs for pruning.
    /// includes the block number as the search key.
    ( StorageTriesPruningLog ) BlockNumHash[BlockNumber] => StorageTriesPruningLogEntry
);

dupsort!(
    /// Receipts table.
    ( Receipts ) TupleRLP<BlockHash, Index>[Index] => IndexedChunk<Receipt>
);

type StorageTriesNodesSeekKey = [u8; 32];
type StorageTriesNodesSuffixKey = [u8; 33];

dupsort!(
    /// Table containing all storage trie's nodes
    /// Each node is stored by hashed account address and node hash in order to keep different storage trie's nodes separate
    ( StorageTriesNodes ) (StorageTriesNodesSeekKey, StorageTriesNodesSuffixKey)[StorageTriesNodesSeekKey] => Vec<u8>
);

dupsort!(
    /// Transaction locations table.
    ( TransactionLocations ) TransactionHashRLP => Rlp<(BlockNumber, BlockHash, Index)>
);

table!(
    /// Stores chain data, each value is unique and stored as its rlp encoding
    /// See [ChainDataIndex] for available chain values
    ( ChainData ) ChainDataIndex => Vec<u8>
);

table!(
    /// Stores snap state, each value is unique and stored as its rlp encoding
    /// See [SnapStateIndex] for available values
    ( SnapState ) SnapStateIndex => Vec<u8>
);

// Trie storages

dupsort!(
    /// state trie nodes
    ( StateTrieNodes ) NodeHash => Vec<u8>
);

// Local Blocks

table!(
    /// payload id to payload table
    ( Payloads ) u64 => PayloadBundleRLP
);

table!(
    /// Stores blocks that are pending validation.
    ( PendingBlocks ) BlockHashRLP => BlockRLP
);

table!(
    /// State Snapshot used by an ongoing sync process
    ( StateSnapShot ) AccountHashRLP => AccountStateRLP
);

dupsort!(
    /// Storage Snapshot used by an ongoing sync process
    ( StorageSnapShot ) AccountHashRLP => (AccountStorageKeyBytes, AccountStorageValueBytes)[AccountStorageKeyBytes]
);

table!(
    /// Storage trie paths in need of healing stored by hashed address
    ( StorageHealPaths ) AccountHashRLP => TriePathsRLP
);

table!(
    /// Stores invalid ancestors
    ( InvalidAncestors ) BlockHashRLP => BlockHashRLP
);

table!(
    /// Tracks the (BlockNumber, BlockHash, ParentHash) corresponding to the current FlatAccountStorage and FlatAccountInfo
    ( FlatTablesBlockMetadata ) FlatTablesBlockMetadataKey => BlockNumHash
);
table!(
    /// Account storage as a flat mapping from (AccountAddress, Slot) to (Value)
    ( FlatAccountStorage ) (AccountAddress, AccountStorageKeyBytes) [AccountAddress] => AccountStorageValueBytes
);
table!(
    /// Account state as a flat mapping from (AccountAddress) to (State)
    ( FlatAccountInfo ) AccountAddress => EncodableAccountInfo
);

pub struct Store {
    db: Arc<Database>,
}
impl Store {
    pub fn new(path: &str) -> Result<Self, StoreError> {
        let db = Arc::new(init_db(Some(path)).map_err(StoreError::LibmdbxError)?);
        Ok(Store { db })
    }

    // Helper method to write into a libmdbx table
    async fn write<T: Table>(&self, key: T::Key, value: T::Value) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;
            txn.upsert::<T>(key, value)
                .map_err(StoreError::LibmdbxError)?;
            txn.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    // Helper method to write into a libmdbx table in batch
    fn replace_value_or_delete<T: Table>(
        flat_storage_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, T>,
        key: T::Key,
        value: Option<T::Value>,
    ) -> Result<(), StoreError>
    where
        <T as libmdbx::orm::Table>::Key: libmdbx::orm::Decodable,
    {
        match value {
            Some(v) => flat_storage_cursor
                .upsert(key, v)
                .map_err(StoreError::LibmdbxError),
            None => {
                if let Some(_current_data) = flat_storage_cursor
                    .seek_exact(key)
                    .map_err(StoreError::LibmdbxError)?
                {
                    flat_storage_cursor
                        .delete_current()
                        .map_err(StoreError::LibmdbxError)
                } else {
                    Ok(())
                }
            }
        }
    }

    // Helper method to write into a libmdbx table in batch
    async fn write_batch<T: Table>(
        &self,
        key_values: Vec<(T::Key, T::Value)>,
    ) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;

            let mut cursor = txn.cursor::<T>().map_err(StoreError::LibmdbxError)?;
            for (key, value) in key_values {
                cursor
                    .upsert(key, value)
                    .map_err(StoreError::LibmdbxError)?;
            }
            txn.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    // Helper method to read from a libmdbx table
    async fn read<T: Table>(&self, key: T::Key) -> Result<Option<T::Value>, StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_read().map_err(StoreError::LibmdbxError)?;
            txn.get::<T>(key).map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    // Helper method to read from a libmdbx table
    async fn read_bulk<T: Table>(&self, keys: Vec<T::Key>) -> Result<Vec<T::Value>, StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let mut res = Vec::new();
            let txn = db.begin_read().map_err(StoreError::LibmdbxError)?;
            for key in keys {
                let val = txn
                    .get::<T>(key)
                    .map_err(StoreError::LibmdbxError)?
                    .ok_or(StoreError::ReadError)?;
                res.push(val);
            }
            Ok(res)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    // Helper method to read from a libmdbx table
    fn read_sync<T: Table>(&self, key: T::Key) -> Result<Option<T::Value>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        txn.get::<T>(key).map_err(StoreError::LibmdbxError)
    }

    fn get_block_hash_by_block_number(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read_sync::<CanonicalBlockHashes>(number)?
            .map(|block_hash| block_hash.to())
            .transpose()
            .map_err(StoreError::from)
    }

    // Check if the snapshot is at the canonical chain
    fn is_at_canonical_chain(&self, snapshot: BlockNumHash) -> Result<bool, StoreError> {
        let canonical_hash = self
            .get_block_hash_by_block_number(snapshot.block_number)?
            .unwrap_or_default();
        Ok(canonical_hash == snapshot.block_hash)
    }

    // Restore the previous state of the account info and storage
    // Returns the parent block of the current snapshot
    fn undo_block(
        &self,
        current_snapshot: BlockNumHash,
        state_log_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, AccountsStateWriteLog>,
        storage_log_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, AccountsStorageWriteLog>,
        flat_info_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, FlatAccountInfo>,
        flat_storage_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, FlatAccountStorage>,
    ) -> Result<Option<BlockNumHash>, StoreError> {
        tracing::debug!("UNDO: processing block {current_snapshot:?}");

        // Undo account info changes
        let account_parent =
            self.undo_account_changes(current_snapshot, state_log_cursor, flat_info_cursor)?;

        // Undo storage changes
        let storage_parent =
            self.undo_storage_changes(current_snapshot, storage_log_cursor, flat_storage_cursor)?;

        // Both should give us the same parent block if they have logs for the current block
        Ok(account_parent.or(storage_parent))
    }

    /// Iterate over the [`AccountsStateWriteLog`] table and restore the previous state
    /// in the [`FlatAccountInfo`] table
    /// Use the old value to restore the previous account info
    fn undo_account_changes(
        &self,
        current_block: BlockNumHash,
        state_log_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, AccountsStateWriteLog>,
        flat_info_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, FlatAccountInfo>,
    ) -> Result<Option<BlockNumHash>, StoreError> {
        let mut parent_block = None;
        let mut state_logs = state_log_cursor.seek_closest(current_block)?;

        let mut account_updates = vec![];
        while let Some(((final_block, found_parent), log_entry)) = state_logs {
            if final_block != current_block {
                break;
            }

            tracing::warn!(
                "UNDO: found state log for {current_block:?}: {final_block:?}/{found_parent:?}"
            );

            // Save the previous account info change for later update
            account_updates.push((log_entry.address, log_entry.previous_info));

            // We update this here to ensure it's the previous block according
            // to the logs found.
            parent_block = Some(found_parent);
            state_logs = state_log_cursor.next()?;
        }

        self.update_flat_account_info(flat_info_cursor, account_updates.iter().cloned())?;

        Ok(parent_block)
    }

    /// Iterate over the [`AccountsStorageWriteLog`] table and restore the previous state
    /// in the [`FlatAccountStorage`] table
    /// Use the old value to restore the previous storage value
    fn undo_storage_changes(
        &self,
        current_block: BlockNumHash,
        storage_log_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, AccountsStorageWriteLog>,
        flat_storage_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, FlatAccountStorage>,
    ) -> Result<Option<BlockNumHash>, StoreError> {
        let mut parent_block = None;
        let mut storage_logs = storage_log_cursor.seek_closest(current_block)?;

        let mut storage_updates = vec![];
        while let Some(((final_block, found_parent), log_entry)) = storage_logs {
            if final_block != current_block {
                break;
            }

            tracing::warn!(
                "UNDO: found storage log for {current_block:?}: {final_block:?}/{found_parent:?}"
            );

            // Save the previous storage change for later update
            storage_updates.push((log_entry.address, log_entry.slot, log_entry.old_value));

            // We update this here to ensure it's the previous block according
            // to the logs found.
            parent_block = Some(found_parent);
            storage_logs = storage_log_cursor.next()?;
        }

        self.update_flat_account_storage(flat_storage_cursor, storage_updates.iter().cloned())?;

        Ok(parent_block)
    }

    /// Check if there are logs in the [`AccountsStateWriteLog`] or [`AccountsStorageWriteLog`]
    /// tables for the transition to the next block
    fn has_logs_for_transition(
        &self,
        from_snapshot: BlockNumHash,
        to_snapshot: BlockNumHash,
        state_log_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, AccountsStateWriteLog>,
        storage_log_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, AccountsStorageWriteLog>,
    ) -> Result<bool, StoreError> {
        let state_logs = state_log_cursor.seek_closest(to_snapshot)?;
        let storage_logs = storage_log_cursor.seek_closest(to_snapshot)?;

        // Check if there are account info logs
        let has_state_logs = state_logs
            .as_ref()
            .map(|((final_block, parent_block), _)| {
                *final_block == to_snapshot && *parent_block == from_snapshot
            })
            .unwrap_or(false);

        // Check if there are storage logs
        let has_storage_logs = storage_logs
            .as_ref()
            .map(|((final_block, parent_block), _)| {
                *final_block == to_snapshot && *parent_block == from_snapshot
            })
            .unwrap_or(false);

        Ok(has_state_logs || has_storage_logs)
    }

    /// Replay the changes for the transition to the next block. Used to reconstruct the snapshot
    /// state
    fn replay_block_changes(
        &self,
        from_snapshot: BlockNumHash,
        to_snapshot: BlockNumHash,
        state_log_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, AccountsStateWriteLog>,
        storage_log_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, AccountsStorageWriteLog>,
        flat_info_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, FlatAccountInfo>,
        flat_storage_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, FlatAccountStorage>,
    ) -> Result<(), StoreError> {
        tracing::warn!("REPLAY: processing block {to_snapshot:?}");

        // Replay the account info changes
        self.replay_account_changes(
            from_snapshot,
            to_snapshot,
            state_log_cursor,
            flat_info_cursor,
        )?;

        // Replay the storage changes
        self.replay_storage_changes(
            from_snapshot,
            to_snapshot,
            storage_log_cursor,
            flat_storage_cursor,
        )?;

        Ok(())
    }

    /// Iterate over the [`AccountsStateWriteLog`] table and apply the changes for the transition to the next block
    /// in the [`FlatAccountInfo`] table
    /// If the new account info is not zero, we restore it
    /// If the new account info is zero, we delete the account info
    fn replay_account_changes(
        &self,
        from_snapshot: BlockNumHash,
        to_snapshot: BlockNumHash,
        state_log_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, AccountsStateWriteLog>,
        flat_info_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, FlatAccountInfo>,
    ) -> Result<(), StoreError> {
        let mut state_logs = state_log_cursor.seek_closest(to_snapshot)?;

        let mut account_updates = vec![];
        while let Some(((final_block, parent_block), log_entry)) = state_logs {
            if final_block != to_snapshot || parent_block != from_snapshot {
                break;
            }
            tracing::debug!("REPLAY: applying account change for {}", log_entry.address);

            // Save the new account info change for later update
            account_updates.push((log_entry.address, log_entry.info));
            state_logs = state_log_cursor.next()?;
        }

        self.update_flat_account_info(flat_info_cursor, account_updates.iter().cloned())?;

        Ok(())
    }

    /// Iterate over the [`AccountsStorageWriteLog`] table and apply the changes for the transition to the next block
    /// in the [`FlatAccountStorage`] table
    fn replay_storage_changes(
        &self,
        from_snapshot: BlockNumHash,
        to_snapshot: BlockNumHash,
        storage_log_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, AccountsStorageWriteLog>,
        flat_storage_cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, FlatAccountStorage>,
    ) -> Result<(), StoreError> {
        let mut storage_logs = storage_log_cursor.seek_closest(to_snapshot)?;

        let mut storage_updates = vec![];
        while let Some(((final_block, parent_block), log_entry)) = storage_logs {
            // Check that we are processing the logs for the transition
            if final_block != to_snapshot || parent_block != from_snapshot {
                break;
            }

            tracing::debug!(
                "REPLAY: applying storage change {}:{}",
                log_entry.address,
                log_entry.slot
            );

            // Save the new storage change for later update
            storage_updates.push((log_entry.address, log_entry.slot, log_entry.new_value));
            storage_logs = storage_log_cursor.next()?;
        }

        self.update_flat_account_storage(flat_storage_cursor, storage_updates.iter().cloned())?;

        Ok(())
    }

    /// Update the [`FlatAccountInfo`] table with the account info changes
    /// If the account info is not zero, we restore it
    /// If the account info is zero, we delete the account info
    fn update_flat_account_info(
        &self,
        cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, FlatAccountInfo>,
        account_updates: impl Iterator<Item = (Address, AccountInfo)>,
    ) -> Result<(), StoreError> {
        for (address, account_info) in account_updates {
            let value = (account_info != AccountInfo::default())
                .then_some(EncodableAccountInfo(account_info));
            Self::replace_value_or_delete(cursor, address.into(), value)?;
        }
        Ok(())
    }

    /// Update the [`FlatAccountStorage`] table with the storage changes
    /// If the storage value is not zero, we restore it
    /// If the storage value is zero, we delete the storage value
    fn update_flat_account_storage(
        &self,
        cursor: &mut libmdbx::orm::Cursor<'_, libmdbx::RW, FlatAccountStorage>,
        storage_updates: impl Iterator<Item = (Address, H256, U256)>,
    ) -> Result<(), StoreError> {
        for (address, key, value) in storage_updates {
            let storage_key = (address.into(), key.into());
            let storage_value = (!value.is_zero()).then_some(value.into());
            Self::replace_value_or_delete(cursor, storage_key, storage_value)?;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl StoreEngine for Store {
    async fn apply_updates(&self, update_batch: UpdateBatch) -> Result<(), StoreError> {
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            let tx = db.begin_readwrite()?;

            // We only need to update the flat tables if the update batch contains blocks
            // We should review what to do in a reconstruct scenario, do we need to update the snapshot state?
            if let (Some(first_block), Some(last_block)) =
                (update_batch.blocks.first(), update_batch.blocks.last())
            {
                let parent_block = (
                    first_block.header.number - 1,
                    first_block.header.parent_hash,
                )
                    .into();
                let final_block = (last_block.header.number, last_block.hash()).into();

                // Update the account info log table
                let mut account_info_log_cursor = tx.cursor::<AccountsStateWriteLog>()?;
                for (addr, old_info, new_info) in
                    update_batch.account_info_log_updates.iter().cloned()
                {
                    account_info_log_cursor.upsert(
                        (final_block, parent_block),
                        AccountInfoLogEntry {
                            address: addr.0,
                            info: new_info,
                            previous_info: old_info,
                        },
                    )?;
                }

                // Update the account storage log table
                let mut account_storage_logs_cursor = tx.cursor::<AccountsStorageWriteLog>()?;
                for entry in update_batch.storage_log_updates.iter().cloned() {
                    account_storage_logs_cursor.upsert((final_block, parent_block), entry)?;
                }

                let meta = tx
                    .get::<FlatTablesBlockMetadata>(FlatTablesBlockMetadataKey {})?
                    .unwrap_or_default();
                // If we are at the parent block, we need to update the flat tables with the new changes
                // and update the metadata to the final block
                if meta == parent_block {
                    let mut info_cursor = tx.cursor::<FlatAccountInfo>()?;
                    let mut storage_cursor = tx.cursor::<FlatAccountStorage>()?;

                    for (addr, _old_info, new_info) in update_batch.account_info_log_updates {
                        let key = addr;
                        let value = (new_info != AccountInfo::default())
                            .then_some(EncodableAccountInfo(new_info));
                        Self::replace_value_or_delete(&mut info_cursor, key, value)?;
                    }
                    for entry in update_batch.storage_log_updates {
                        let key = (entry.address.into(), entry.slot.into());
                        let value = (!entry.new_value.is_zero()).then_some(entry.new_value.into());
                        Self::replace_value_or_delete(&mut storage_cursor, key, value)?;
                    }
                    tx.upsert::<FlatTablesBlockMetadata>(
                        FlatTablesBlockMetadataKey {},
                        final_block,
                    )?;
                }

                let mut cursor_state_trie_pruning_log = tx.cursor::<StateTriePruningLog>()?;
                let mut cursor_storage_trie_pruning_log = tx.cursor::<StorageTriesPruningLog>()?;

                // For each block in the update batch, we iterate over the account updates (by index)
                // we store account info changes in the table StateWriteBatch
                // store account updates
                for (node_hash, mut node_data) in update_batch.account_updates {
                    tracing::debug!(
                        node_hash = hex::encode(node_hash_to_fixed_size(node_hash)),
                        parent_block_number = parent_block.block_number,
                        parent_block_hash = hex::encode(parent_block.block_hash),
                        final_block_number = final_block.block_number,
                        final_block_hash = hex::encode(final_block.block_hash),
                        "[WRITING STATE TRIE NODE]",
                    );
                    node_data.extend_from_slice(&final_block.block_number.to_be_bytes());
                    tx.upsert::<StateTrieNodes>(node_hash, node_data)?;
                }

                for node_hash in update_batch.invalidated_state_nodes {
                    // Before inserting, we insert the node into the pruning log
                    cursor_state_trie_pruning_log
                        .upsert(final_block, StateTriePruningLogEntry::from(node_hash.0))?;
                }

                for (hashed_address, nodes, invalidated_nodes) in update_batch.storage_updates {
                    let key_1: [u8; 32] = hashed_address.into();
                    for (node_hash, mut node_data) in nodes {
                        let key_2 = node_hash_to_fixed_size(node_hash);

                        tracing::debug!(
                            hashed_address = hex::encode(hashed_address.0),
                            node_hash = hex::encode(node_hash_to_fixed_size(node_hash)),
                            parent_block_number = parent_block.block_number,
                            parent_block_hash = hex::encode(parent_block.block_hash),
                            final_block_number = final_block.block_number,
                            final_block_hash = hex::encode(final_block.block_hash),
                            "[WRITING STORAGE TRIE NODE]",
                        );
                        node_data.extend_from_slice(&final_block.block_number.to_be_bytes());
                        tx.upsert::<StorageTriesNodes>((key_1, key_2), node_data)?;
                    }
                    for node_hash in invalidated_nodes {
                        // NOTE: the hash itself *should* suffice, but this way the value matches
                        // the other table's key.
                        let key_2 = node_hash_to_fixed_size(NodeHash::Hashed(node_hash));
                        cursor_storage_trie_pruning_log.upsert(final_block, (key_1, key_2))?;
                    }
                }
            } else {
                // In case that we are in a reconstruct scenario (L2), we need to update the state and storage tries
                // without the block number extension
                for (node_hash, node_data) in update_batch.account_updates {
                    tx.upsert::<StateTrieNodes>(node_hash, node_data)?;
                }

                for (hashed_address, nodes, _invalidated_nodes) in update_batch.storage_updates {
                    for (node_hash, node_data) in nodes {
                        let key_1: [u8; 32] = hashed_address.into();
                        let key_2 = node_hash_to_fixed_size(node_hash);

                        tx.upsert::<StorageTriesNodes>((key_1, key_2), node_data)?;
                    }
                }
            }

            // store code updates
            for (hashed_address, code) in update_batch.code_updates {
                tx.upsert::<AccountCodes>(hashed_address.into(), code.into())?;
            }

            for block in update_batch.blocks {
                // store block
                let number = block.header.number;
                let hash = block.hash();

                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    tx.upsert::<TransactionLocations>(
                        transaction.compute_hash().into(),
                        (number, hash, index as u64).into(),
                    )?;
                }

                tx.upsert::<Bodies>(
                    hash.into(),
                    BlockBodyRLP::from_bytes(block.body.encode_to_vec()),
                )?;

                tx.upsert::<Headers>(
                    hash.into(),
                    BlockHeaderRLP::from_bytes(block.header.encode_to_vec()),
                )?;

                tx.upsert::<BlockNumbers>(hash.into(), number)?;
            }
            for (block_hash, receipts) in update_batch.receipts {
                let mut key_values = vec![];
                // store receipts

                for mut entries in
                    receipts
                        .into_iter()
                        .enumerate()
                        .filter_map(|(index, receipt)| {
                            let key = (block_hash, index as u64).into();
                            let receipt_rlp = receipt.encode_to_vec();
                            IndexedChunk::from::<Receipts>(key, &receipt_rlp)
                        })
                {
                    key_values.append(&mut entries);
                }
                let mut cursor = tx.cursor::<Receipts>()?;
                for (key, value) in key_values {
                    cursor.upsert(key, value)?;
                }
            }

            tx.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn undo_writes_until_canonical(&self) -> Result<(), StoreError> {
        let tx = self.db.begin_readwrite()?;
        let Some(mut current_snapshot) =
            tx.get::<FlatTablesBlockMetadata>(FlatTablesBlockMetadataKey {})?
        else {
            return Ok(()); // No snapshot to revert
        };

        // Open new cursors for each table
        let mut state_log_cursor = tx.cursor::<AccountsStateWriteLog>()?;
        let mut storage_log_cursor = tx.cursor::<AccountsStorageWriteLog>()?;
        let mut flat_info_cursor = tx.cursor::<FlatAccountInfo>()?;
        let mut flat_storage_cursor = tx.cursor::<FlatAccountStorage>()?;

        // Iterate over the blocks until the snapshot is at the canonical chain
        while !self.is_at_canonical_chain(current_snapshot)? {
            let Some(next_snapshot) = self.undo_block(
                current_snapshot,
                &mut state_log_cursor,
                &mut storage_log_cursor,
                &mut flat_info_cursor,
                &mut flat_storage_cursor,
            )?
            else {
                // If the snapshot don't have parent block, we are in genesis
                tracing::warn!("UNDO: logs exhausted, back to canonical chain");
                break;
            };

            current_snapshot = next_snapshot;
            tracing::debug!("UNDO: moved to snapshot {current_snapshot:?}");
        }

        // Update the snapshot metadata to the last valid snapshot
        tx.upsert::<FlatTablesBlockMetadata>(FlatTablesBlockMetadataKey {}, current_snapshot)?;
        tx.commit().map_err(|err| err.into())
    }

    async fn replay_writes_until_head(&self, head_hash: H256) -> Result<(), StoreError> {
        let tx = self.db.begin_readwrite()?;
        let mut current_snapshot = tx
            .get::<FlatTablesBlockMetadata>(FlatTablesBlockMetadataKey {})?
            .unwrap_or_default();

        // Open cursors for each table
        let mut state_log_cursor = tx.cursor::<AccountsStateWriteLog>()?;
        let mut storage_log_cursor = tx.cursor::<AccountsStorageWriteLog>()?;
        let mut flat_info_cursor = tx.cursor::<FlatAccountInfo>()?;
        let mut flat_storage_cursor = tx.cursor::<FlatAccountStorage>()?;

        for canonical_block in tx
            .cursor::<CanonicalBlockHashes>()?
            .walk(Some(current_snapshot.block_number + 1))
        {
            let (block_num, block_hash_rlp) = canonical_block?;
            let block_hash = block_hash_rlp.to()?;
            // Update the snapshot to the next canonical block
            let next_snapshot = BlockNumHash {
                block_number: block_num,
                block_hash,
            };

            // If there are logs for the transition to the next block, replay the changes
            if self.has_logs_for_transition(
                current_snapshot,
                next_snapshot,
                &mut state_log_cursor,
                &mut storage_log_cursor,
            )? {
                self.replay_block_changes(
                    current_snapshot,
                    next_snapshot,
                    &mut state_log_cursor,
                    &mut storage_log_cursor,
                    &mut flat_info_cursor,
                    &mut flat_storage_cursor,
                )?;
                tracing::debug!("REPLAY: applied changes for block {next_snapshot:?}");
            } else {
                tracing::debug!("REPLAY: No changes for block {next_snapshot:?}");
            }

            current_snapshot = next_snapshot;

            if head_hash == current_snapshot.block_hash {
                tracing::debug!("REPLAY: reached head {head_hash:?}");
                break;
            }
        }

        // Update the snapshot metadata to the last valid snapshot (head)
        tx.upsert::<FlatTablesBlockMetadata>(FlatTablesBlockMetadataKey {}, current_snapshot)?;
        tx.commit().map_err(|err| err.into())
    }

    fn prune_state_and_storage_log(
        &self,
        cancellation_token: CancellationToken,
    ) -> Result<(), StoreError> {
        if cancellation_token.is_cancelled() {
            tracing::warn!("Received shutdown signal, aborting pruning");
            return Ok(());
        }

        let tx = self.db.begin_readwrite()?;

        let stats_pre_state_log = tx
            .table_stat::<StateTriePruningLog>()
            .map_err(|e| anyhow::anyhow!("error: {e}"))?;
        let stats_pre_state_nodes = tx
            .table_stat::<StateTrieNodes>()
            .map_err(|e| anyhow::anyhow!("error: {e}"))?;
        let stats_pre_storage_log = tx
            .table_stat::<StorageTriesPruningLog>()
            .map_err(|e| anyhow::anyhow!("error: {e}"))?;
        let stats_pre_storage_nodes = tx
            .table_stat::<StorageTriesNodes>()
            .map_err(|e| anyhow::anyhow!("error: {e}"))?;

        let mut cursor_state_trie_pruning_log = tx.cursor::<StateTriePruningLog>()?;
        // Get the block number of the last state trie pruning log entry
        if let Some((
            BlockNumHash {
                block_number: last_num,
                block_hash: _,
            },
            _,
        )) = cursor_state_trie_pruning_log.last()?
        {
            let keep_from = last_num.saturating_sub(KEEP_BLOCKS);
            tracing::debug!(keep_from, last_num, "[KEEPING STATE TRIE PRUNING LOG]");

            let mut cursor_state_trie = tx.cursor::<StateTrieNodes>()?;
            let mut kv_state_trie_pruning = cursor_state_trie_pruning_log.first()?;
            // Iterate over the first entries of the pruning log and delete the nodes from the trie
            // until we reach the keep from block number
            while let Some((block, node_hash)) = kv_state_trie_pruning {
                // If the block number is higher than the keep from, we can stop
                // since we reached the keep from block number
                if block.block_number >= keep_from {
                    tracing::debug!(keep_from, last_num, "[STOPPING STATE TRIE PRUNING]");
                    break;
                }

                // Delete the node from the state trie and the pruning log
                let k_delete = NodeHash::Hashed(node_hash.into());
                if let Some((key, _)) = cursor_state_trie.seek_exact(k_delete)? {
                    if key == k_delete {
                        tracing::debug!(
                            node = hex::encode(node_hash.as_ref()),
                            block_number = block.block_number,
                            block_hash = hex::encode(block.block_hash.0.as_ref()),
                            "[DELETING STATE NODE]"
                        );
                        cursor_state_trie.delete_current()?;
                        cursor_state_trie_pruning_log.delete_current()?;
                    }
                }
                kv_state_trie_pruning = cursor_state_trie_pruning_log.next()?;
            }
        }

        let mut cursor_storage_trie_pruning_log = tx.cursor::<StorageTriesPruningLog>()?;
        // Get the block number of the last storage trie pruning log entry
        if let Some((
            BlockNumHash {
                block_number: last_num,
                block_hash: _,
            },
            _,
        )) = cursor_storage_trie_pruning_log.last()?
        {
            let keep_from = last_num.saturating_sub(KEEP_BLOCKS);
            tracing::debug!(keep_from, last_num, "[KEEPING STORAGE TRIE PRUNING LOG]");

            let mut cursor_storage_trie = tx.cursor::<StorageTriesNodes>()?;
            let mut kv_storage_trie_pruning = cursor_storage_trie_pruning_log.first()?;
            // Iterate over the first entries of the pruning log and delete the nodes from the trie
            // until we reach the keep from block number
            while let Some((block, storage_trie_pruning_hash)) = kv_storage_trie_pruning {
                // If the block number is higher than the keep from, we can stop
                // since we reached the keep from block number
                if block.block_number >= keep_from {
                    tracing::debug!(
                        keep_from = keep_from,
                        last_num = last_num,
                        "[STOPPING STORAGE TRIE PRUNING]"
                    );
                    break;
                }

                // If the storage trie hash is found, delete it from the trie and the pruning log
                if let Some((key, _)) = cursor_storage_trie.seek_exact(storage_trie_pruning_hash)? {
                    if key == storage_trie_pruning_hash {
                        tracing::debug!(
                            hashed_address = hex::encode(storage_trie_pruning_hash.0.as_ref()),
                            node_hash = hex::encode(storage_trie_pruning_hash.1.as_ref()),
                            block_number = block.block_number,
                            block_hash = hex::encode(block.block_hash.0.as_ref()),
                            "[DELETING STORAGE NODE]"
                        );
                        cursor_storage_trie.delete_current()?;
                        cursor_storage_trie_pruning_log.delete_current()?;
                    }
                }
                kv_storage_trie_pruning = cursor_storage_trie_pruning_log.next()?;
            }
        }

        // Get the stats after the pruning and log the metrics
        let stats_post_state_log = tx
            .table_stat::<StateTriePruningLog>()
            .map_err(|e| anyhow::anyhow!("error: {e}"))?;
        let stats_post_state_nodes = tx
            .table_stat::<StateTrieNodes>()
            .map_err(|e| anyhow::anyhow!("error: {e}"))?;
        let stats_post_storage_log = tx
            .table_stat::<StorageTriesPruningLog>()
            .map_err(|e| anyhow::anyhow!("error: {e}"))?;
        let stats_post_storage_nodes = tx
            .table_stat::<StorageTriesNodes>()
            .map_err(|e| anyhow::anyhow!("error: {e}"))?;

        tracing::info!(
            state_trie_entries_delta = stats_post_state_nodes.entries() as isize
                - stats_pre_state_nodes.entries() as isize,
            state_trie_log_entries_delta =
                stats_post_state_log.entries() as isize - stats_pre_state_log.entries() as isize,
            storage_trie_entries_delta = stats_post_storage_nodes.entries() as isize
                - stats_pre_storage_nodes.entries() as isize,
            storage_trie_log_entries_delta = stats_post_storage_log.entries() as isize
                - stats_pre_storage_log.entries() as isize,
            "[PRUNING METRICS]",
        );

        debug_assert_eq!(
            stats_post_state_nodes.entries() as isize - stats_pre_state_nodes.entries() as isize,
            stats_post_state_log.entries() as isize - stats_pre_state_log.entries() as isize,
        );
        debug_assert_eq!(
            stats_post_storage_nodes.entries() as isize
                - stats_pre_storage_nodes.entries() as isize,
            stats_post_storage_log.entries() as isize - stats_pre_storage_log.entries() as isize,
        );

        tx.commit().map_err(StoreError::LibmdbxError)
    }

    async fn add_block_header(
        &self,
        block_hash: BlockHash,
        block_header: BlockHeader,
    ) -> Result<(), StoreError> {
        self.write::<Headers>(block_hash.into(), block_header.into())
            .await
    }

    async fn add_block_headers(
        &self,
        block_hashes: Vec<BlockHash>,
        block_headers: Vec<BlockHeader>,
    ) -> Result<(), StoreError> {
        let hashes_and_headers = block_hashes
            .into_iter()
            .zip(block_headers)
            .map(|(hash, header)| (hash.into(), header.into()))
            .collect();
        self.write_batch::<Headers>(hashes_and_headers).await
    }

    fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHeader>, StoreError> {
        let Some(block_hash) = self.get_block_hash_by_block_number(block_number)? else {
            return Ok(None);
        };

        self.read_sync::<Headers>(block_hash.into())?
            .map(|b| b.to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn add_block_body(
        &self,
        block_hash: BlockHash,
        block_body: BlockBody,
    ) -> Result<(), StoreError> {
        self.write::<Bodies>(block_hash.into(), block_body.into())
            .await
    }

    async fn add_blocks(&self, blocks: Vec<Block>) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let tx = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;

            for block in blocks {
                let number = block.header.number;
                let hash = block.hash();

                for (index, transaction) in block.body.transactions.iter().enumerate() {
                    tx.upsert::<TransactionLocations>(
                        transaction.compute_hash().into(),
                        (number, hash, index as u64).into(),
                    )
                    .map_err(StoreError::LibmdbxError)?;
                }

                tx.upsert::<Bodies>(
                    hash.into(),
                    BlockBodyRLP::from_bytes(block.body.encode_to_vec()),
                )
                .map_err(StoreError::LibmdbxError)?;

                tx.upsert::<Headers>(
                    hash.into(),
                    BlockHeaderRLP::from_bytes(block.header.encode_to_vec()),
                )
                .map_err(StoreError::LibmdbxError)?;

                tx.upsert::<BlockNumbers>(hash.into(), number)
                    .map_err(StoreError::LibmdbxError)?;
            }

            tx.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn mark_chain_as_canonical(&self, blocks: &[Block]) -> Result<(), StoreError> {
        let key_values = blocks
            .iter()
            .map(|e| (e.header.number, e.hash().into()))
            .collect();

        self.write_batch::<CanonicalBlockHashes>(key_values).await
    }

    async fn get_block_body(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockBody>, StoreError> {
        if let Some(hash) = self.get_block_hash_by_block_number(block_number)? {
            self.get_block_body_by_hash(hash).await
        } else {
            Ok(None)
        }
    }

    async fn remove_block(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        let Some(hash) = self.get_block_hash_by_block_number(block_number)? else {
            return Ok(());
        };
        let txn = self
            .db
            .begin_readwrite()
            .map_err(StoreError::LibmdbxError)?;

        txn.delete::<CanonicalBlockHashes>(block_number, None)
            .map_err(StoreError::LibmdbxError)?;
        txn.delete::<Bodies>(hash.into(), None)
            .map_err(StoreError::LibmdbxError)?;
        txn.delete::<Headers>(hash.into(), None)
            .map_err(StoreError::LibmdbxError)?;
        txn.delete::<BlockNumbers>(hash.into(), None)
            .map_err(StoreError::LibmdbxError)?;

        txn.commit().map_err(StoreError::LibmdbxError)
    }

    async fn get_block_bodies(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let numbers = (from..=to).collect();
        let hashes = self.read_bulk::<CanonicalBlockHashes>(numbers).await?;
        let blocks = self.read_bulk::<Bodies>(hashes).await?;
        let mut block_bodies = Vec::new();
        for block_body in blocks.into_iter() {
            block_bodies.push(block_body.to()?)
        }
        Ok(block_bodies)
    }

    async fn get_block_bodies_by_hash(
        &self,
        hashes: Vec<BlockHash>,
    ) -> Result<Vec<BlockBody>, StoreError> {
        let hashes = hashes.into_iter().map(|h| h.into()).collect();
        let blocks = self.read_bulk::<Bodies>(hashes).await?;
        let mut block_bodies = Vec::new();
        for block_body in blocks.into_iter() {
            block_bodies.push(block_body.to()?)
        }
        Ok(block_bodies)
    }

    async fn get_block_body_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockBody>, StoreError> {
        self.read::<Bodies>(block_hash.into())
            .await?
            .map(|b| b.to())
            .transpose()
            .map_err(StoreError::from)
    }

    fn get_block_header_by_hash(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockHeader>, StoreError> {
        self.read_sync::<Headers>(block_hash.into())?
            .map(|b| b.to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn add_block_number(
        &self,
        block_hash: BlockHash,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write::<BlockNumbers>(block_hash.into(), block_number)
            .await
    }

    async fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        self.read::<BlockNumbers>(block_hash.into()).await
    }

    fn get_block_number_sync(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<BlockNumber>, StoreError> {
        self.read_sync::<BlockNumbers>(block_hash.into())
    }

    async fn add_account_code(&self, code_hash: H256, code: Bytes) -> Result<(), StoreError> {
        self.write::<AccountCodes>(code_hash.into(), code.into())
            .await
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Option<Bytes>, StoreError> {
        self.read_sync::<AccountCodes>(code_hash.into())?
            .map(|b| b.to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn add_receipt(
        &self,
        block_hash: BlockHash,
        index: Index,
        receipt: Receipt,
    ) -> Result<(), StoreError> {
        let key: Rlp<(BlockHash, Index)> = (block_hash, index).into();
        let Some(entries) = IndexedChunk::from::<Receipts>(key, &receipt.encode_to_vec()) else {
            return Err(StoreError::Custom("Invalid size".to_string()));
        };
        self.write_batch::<Receipts>(entries).await
    }

    async fn get_receipt(
        &self,
        block_number: BlockNumber,
        index: Index,
    ) -> Result<Option<Receipt>, StoreError> {
        if let Some(hash) = self.get_block_hash_by_block_number(block_number)? {
            let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
            let mut cursor = txn.cursor::<Receipts>().map_err(StoreError::LibmdbxError)?;
            let key = (hash, index).into();
            IndexedChunk::read_from_db(&mut cursor, key)
        } else {
            Ok(None)
        }
    }

    async fn add_transaction_location(
        &self,
        transaction_hash: H256,
        block_number: BlockNumber,
        block_hash: BlockHash,
        index: Index,
    ) -> Result<(), StoreError> {
        self.write::<TransactionLocations>(
            transaction_hash.into(),
            (block_number, block_hash, index).into(),
        )
        .await
    }

    async fn get_transaction_location(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<(BlockNumber, BlockHash, Index)>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        let cursor = txn
            .cursor::<TransactionLocations>()
            .map_err(StoreError::LibmdbxError)?;

        let mut transaction_hashes = Vec::new();
        let mut cursor_it = cursor.walk_key(transaction_hash.into(), None);
        while let Some(Ok(tx)) = cursor_it.next() {
            transaction_hashes.push(tx.to().map_err(StoreError::from)?);
        }

        Ok(transaction_hashes
            .into_iter()
            .find(|(number, hash, _index)| {
                self.get_block_hash_by_block_number(*number)
                    .is_ok_and(|o| o == Some(*hash))
            }))
    }

    /// Stores the chain config serialized as json
    async fn set_chain_config(&self, chain_config: &ChainConfig) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::ChainConfig,
            serde_json::to_string(chain_config)
                .map_err(|_| StoreError::DecodeError)?
                .into_bytes(),
        )
        .await
    }

    fn get_chain_config(&self) -> Result<ChainConfig, StoreError> {
        match self.read_sync::<ChainData>(ChainDataIndex::ChainConfig)? {
            None => Err(StoreError::Custom("Chain config not found".to_string())),
            Some(bytes) => {
                let json = String::from_utf8(bytes).map_err(|_| StoreError::DecodeError)?;
                let chain_config: ChainConfig =
                    serde_json::from_str(&json).map_err(|_| StoreError::DecodeError)?;
                Ok(chain_config)
            }
        }
    }

    async fn update_earliest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::EarliestBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn get_earliest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read::<ChainData>(ChainDataIndex::EarliestBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    async fn update_finalized_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::FinalizedBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn get_finalized_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read::<ChainData>(ChainDataIndex::FinalizedBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    async fn update_safe_block_number(&self, block_number: BlockNumber) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::SafeBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn get_safe_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read::<ChainData>(ChainDataIndex::SafeBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    async fn update_latest_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::LatestBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn get_latest_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read::<ChainData>(ChainDataIndex::LatestBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    async fn update_pending_block_number(
        &self,
        block_number: BlockNumber,
    ) -> Result<(), StoreError> {
        self.write::<ChainData>(
            ChainDataIndex::PendingBlockNumber,
            block_number.encode_to_vec(),
        )
        .await
    }

    async fn get_pending_block_number(&self) -> Result<Option<BlockNumber>, StoreError> {
        match self
            .read::<ChainData>(ChainDataIndex::PendingBlockNumber)
            .await?
        {
            None => Ok(None),
            Some(ref rlp) => RLPDecode::decode(rlp)
                .map(Some)
                .map_err(|_| StoreError::DecodeError),
        }
    }

    fn open_storage_trie(
        &self,
        hashed_address: H256,
        storage_root: H256,
    ) -> Result<Trie, StoreError> {
        let db = Box::new(LibmdbxDupsortTrieDB::<StorageTriesNodes, [u8; 32]>::new(
            self.db.clone(),
            hashed_address.0,
        ));
        Ok(Trie::open(db, storage_root))
    }

    fn open_state_trie(&self, state_root: H256) -> Result<Trie, StoreError> {
        let db = Box::new(LibmdbxTrieDB::<StateTrieNodes>::new(self.db.clone()));
        Ok(Trie::open(db, state_root))
    }

    async fn set_canonical_block(
        &self,
        number: BlockNumber,
        hash: BlockHash,
    ) -> Result<(), StoreError> {
        self.write::<CanonicalBlockHashes>(number, hash.into())
            .await
    }

    async fn get_canonical_block_hash(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read::<CanonicalBlockHashes>(number)
            .await
            .map(|o| o.map(|hash_rlp| hash_rlp.to()))?
            .transpose()
            .map_err(StoreError::from)
    }

    fn get_canonical_block_hash_sync(
        &self,
        number: BlockNumber,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read_sync::<CanonicalBlockHashes>(number)
            .map(|o| o.map(|hash_rlp| hash_rlp.to()))?
            .transpose()
            .map_err(StoreError::from)
    }

    async fn add_payload(&self, payload_id: u64, block: Block) -> Result<(), StoreError> {
        self.write::<Payloads>(payload_id, PayloadBundle::from_block(block).into())
            .await
    }

    async fn get_payload(&self, payload_id: u64) -> Result<Option<PayloadBundle>, StoreError> {
        Ok(self
            .read::<Payloads>(payload_id)
            .await?
            .map(|b| b.to())
            .transpose()
            .map_err(StoreError::from)?)
    }

    async fn update_payload(
        &self,
        payload_id: u64,
        payload: PayloadBundle,
    ) -> Result<(), StoreError> {
        self.write::<Payloads>(payload_id, payload.into()).await
    }

    async fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        let (_block_number, block_hash, index) =
            match self.get_transaction_location(transaction_hash).await? {
                Some(location) => location,
                None => return Ok(None),
            };
        self.get_transaction_by_location(block_hash, index).await
    }

    async fn get_transaction_by_location(
        &self,
        block_hash: H256,
        index: u64,
    ) -> Result<Option<Transaction>, StoreError> {
        let block_body = match self.get_block_body_by_hash(block_hash).await? {
            Some(body) => body,
            None => return Ok(None),
        };
        Ok(index
            .try_into()
            .ok()
            .and_then(|index: usize| block_body.transactions.get(index).cloned()))
    }

    async fn get_block_by_hash(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        let header = match self.get_block_header_by_hash(block_hash)? {
            Some(header) => header,
            None => return Ok(None),
        };
        let body = match self.get_block_body_by_hash(block_hash).await? {
            Some(body) => body,
            None => return Ok(None),
        };
        Ok(Some(Block::new(header, body)))
    }

    async fn unset_canonical_block(&self, number: BlockNumber) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.begin_readwrite()
                .map_err(StoreError::LibmdbxError)?
                .delete::<CanonicalBlockHashes>(number, None)
                .map(|_| ())
                .map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn add_pending_block(&self, block: Block) -> Result<(), StoreError> {
        self.write::<PendingBlocks>(block.hash().into(), block.into())
            .await
    }

    async fn get_pending_block(&self, block_hash: BlockHash) -> Result<Option<Block>, StoreError> {
        self.read::<PendingBlocks>(block_hash.into())
            .await?
            .map(|b| b.to())
            .transpose()
            .map_err(StoreError::from)
    }

    async fn add_transaction_locations(
        &self,
        locations: Vec<(H256, BlockNumber, BlockHash, Index)>,
    ) -> Result<(), StoreError> {
        #[allow(clippy::type_complexity)]
        let key_values = locations
            .into_iter()
            .map(|(tx_hash, block_number, block_hash, index)| {
                (tx_hash.into(), (block_number, block_hash, index).into())
            })
            .collect();

        self.write_batch::<TransactionLocations>(key_values).await
    }

    async fn add_receipts(
        &self,
        block_hash: BlockHash,
        receipts: Vec<Receipt>,
    ) -> Result<(), StoreError> {
        let mut key_values = vec![];

        for (index, receipt) in receipts.clone().into_iter().enumerate() {
            let key = (block_hash, index as u64).into();
            let receipt_rlp = receipt.encode_to_vec();
            let Some(mut entries) = IndexedChunk::from::<Receipts>(key, &receipt_rlp) else {
                continue;
            };

            key_values.append(&mut entries);
        }

        self.write_batch::<Receipts>(key_values).await
    }

    fn get_receipts_for_block(&self, block_hash: &BlockHash) -> Result<Vec<Receipt>, StoreError> {
        let mut receipts = vec![];
        let mut receipt_index = 0;
        let mut key = (*block_hash, 0).into();
        let txn = self.db.begin_read().map_err(|_| StoreError::ReadError)?;
        let mut cursor = txn
            .cursor::<Receipts>()
            .map_err(|_| StoreError::CursorError("Receipts".to_owned()))?;

        // We're searching receipts for a block, the keys
        // for the receipt table are of the kind: rlp((BlockHash, Index)).
        // So we search for values in the db that match with this kind
        // of key, until we reach an Index that returns None
        // and we stop the search.
        while let Some(receipt) = IndexedChunk::read_from_db(&mut cursor, key)? {
            receipts.push(receipt);
            receipt_index += 1;
            key = (*block_hash, receipt_index).into();
        }

        Ok(receipts)
    }

    async fn set_header_download_checkpoint(
        &self,
        block_hash: BlockHash,
    ) -> Result<(), StoreError> {
        self.write::<SnapState>(
            SnapStateIndex::HeaderDownloadCheckpoint,
            block_hash.encode_to_vec(),
        )
        .await
    }

    async fn get_header_download_checkpoint(&self) -> Result<Option<BlockHash>, StoreError> {
        self.read::<SnapState>(SnapStateIndex::HeaderDownloadCheckpoint)
            .await?
            .map(|ref h| BlockHash::decode(h))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn set_state_trie_key_checkpoint(
        &self,
        last_keys: [H256; STATE_TRIE_SEGMENTS],
    ) -> Result<(), StoreError> {
        self.write::<SnapState>(
            SnapStateIndex::StateTrieKeyCheckpoint,
            last_keys.to_vec().encode_to_vec(),
        )
        .await
    }

    async fn get_state_trie_key_checkpoint(
        &self,
    ) -> Result<Option<[H256; STATE_TRIE_SEGMENTS]>, StoreError> {
        self.read::<SnapState>(SnapStateIndex::StateTrieKeyCheckpoint)
            .await?
            .map(|ref c| {
                <Vec<H256>>::decode(c)?
                    .try_into()
                    .map_err(|_| RLPDecodeError::InvalidLength)
            })
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn set_storage_heal_paths(
        &self,
        paths: Vec<(H256, Vec<Nibbles>)>,
    ) -> Result<(), StoreError> {
        self.write_batch::<StorageHealPaths>(
            paths
                .into_iter()
                .map(|(hash, paths)| (hash.into(), paths.into()))
                .collect(),
        )
        .await
    }

    async fn take_storage_heal_paths(
        &self,
        limit: usize,
    ) -> Result<Vec<(H256, Vec<Nibbles>)>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        let cursor = txn
            .cursor::<StorageHealPaths>()
            .map_err(StoreError::LibmdbxError)?;

        let mut res = Vec::new();
        let mut cursor_it = cursor.walk(None);
        while let Some(Ok((hash, paths))) = cursor_it.next() {
            res.push((hash.to()?, paths.to()?));
        }

        res = res.into_iter().take(limit).collect::<Vec<_>>();

        // Delete fetched entries from the table
        let txn = self
            .db
            .begin_readwrite()
            .map_err(StoreError::LibmdbxError)?;
        for (hash, _) in res.iter() {
            txn.delete::<StorageHealPaths>((*hash).into(), None)
                .map_err(StoreError::LibmdbxError)?;
        }
        txn.commit().map_err(StoreError::LibmdbxError)?;
        Ok(res)
    }

    async fn set_state_heal_paths(&self, paths: Vec<Nibbles>) -> Result<(), StoreError> {
        self.write::<SnapState>(SnapStateIndex::StateHealPaths, paths.encode_to_vec())
            .await
    }

    async fn get_state_heal_paths(&self) -> Result<Option<Vec<Nibbles>>, StoreError> {
        self.read::<SnapState>(SnapStateIndex::StateHealPaths)
            .await?
            .map(|ref h| <Vec<Nibbles>>::decode(h))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn clear_snap_state(&self) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;
            txn.clear_table::<SnapState>()
                .map_err(StoreError::LibmdbxError)?;
            txn.clear_table::<StorageHealPaths>()
                .map_err(StoreError::LibmdbxError)?;
            txn.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn write_snapshot_account_batch(
        &self,
        account_hashes: Vec<H256>,
        account_states: Vec<AccountState>,
    ) -> Result<(), StoreError> {
        self.write_batch::<StateSnapShot>(
            account_hashes
                .into_iter()
                .map(|h| h.into())
                .zip(account_states.into_iter().map(|a| a.into()))
                .collect(),
        )
        .await
    }

    async fn write_snapshot_storage_batch(
        &self,
        account_hash: H256,
        storage_keys: Vec<H256>,
        storage_values: Vec<U256>,
    ) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;

            for (key, value) in storage_keys.into_iter().zip(storage_values.into_iter()) {
                txn.upsert::<StorageSnapShot>(account_hash.into(), (key.into(), value.into()))
                    .map_err(StoreError::LibmdbxError)?;
            }

            txn.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn write_snapshot_storage_batches(
        &self,
        account_hashes: Vec<H256>,
        storage_keys: Vec<Vec<H256>>,
        storage_values: Vec<Vec<U256>>,
    ) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;
            for (account_hash, (storage_keys, storage_values)) in account_hashes
                .into_iter()
                .zip(storage_keys.into_iter().zip(storage_values.into_iter()))
            {
                for (key, value) in storage_keys.into_iter().zip(storage_values.into_iter()) {
                    txn.upsert::<StorageSnapShot>(account_hash.into(), (key.into(), value.into()))
                        .map_err(StoreError::LibmdbxError)?;
                }
            }
            txn.commit().map_err(StoreError::LibmdbxError)
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    async fn set_state_trie_rebuild_checkpoint(
        &self,
        checkpoint: (H256, [H256; STATE_TRIE_SEGMENTS]),
    ) -> Result<(), StoreError> {
        self.write::<SnapState>(
            SnapStateIndex::StateTrieRebuildCheckpoint,
            (checkpoint.0, checkpoint.1.to_vec()).encode_to_vec(),
        )
        .await
    }

    async fn get_state_trie_rebuild_checkpoint(
        &self,
    ) -> Result<Option<(H256, [H256; STATE_TRIE_SEGMENTS])>, StoreError> {
        let Some((root, checkpoints)) = self
            .read::<SnapState>(SnapStateIndex::StateTrieRebuildCheckpoint)
            .await?
            .map(|ref c| <(H256, Vec<H256>)>::decode(c))
            .transpose()?
        else {
            return Ok(None);
        };
        Ok(Some((
            root,
            checkpoints
                .try_into()
                .map_err(|_| RLPDecodeError::InvalidLength)?,
        )))
    }

    async fn set_storage_trie_rebuild_pending(
        &self,
        pending: Vec<(H256, H256)>,
    ) -> Result<(), StoreError> {
        self.write::<SnapState>(
            SnapStateIndex::StorageTrieRebuildPending,
            pending.encode_to_vec(),
        )
        .await
    }

    async fn get_storage_trie_rebuild_pending(
        &self,
    ) -> Result<Option<Vec<(H256, H256)>>, StoreError> {
        self.read::<SnapState>(SnapStateIndex::StorageTrieRebuildPending)
            .await?
            .map(|ref h| <Vec<(H256, H256)>>::decode(h))
            .transpose()
            .map_err(StoreError::RLPDecode)
    }

    async fn clear_snapshot(&self) -> Result<(), StoreError> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let txn = db.begin_readwrite().map_err(StoreError::LibmdbxError)?;
            txn.clear_table::<StateSnapShot>()
                .map_err(StoreError::LibmdbxError)?;
            txn.clear_table::<StorageSnapShot>()
                .map_err(StoreError::LibmdbxError)?;
            txn.commit().map_err(StoreError::LibmdbxError)?;
            Ok(())
        })
        .await
        .map_err(|e| StoreError::Custom(format!("task panicked: {e}")))?
    }

    fn read_account_snapshot(&self, start: H256) -> Result<Vec<(H256, AccountState)>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        let cursor = txn
            .cursor::<StateSnapShot>()
            .map_err(StoreError::LibmdbxError)?;
        let iter = cursor
            .walk(Some(start.into()))
            .map_while(|res| {
                res.ok().map(|(hash, acc)| match (hash.to(), acc.to()) {
                    (Ok(hash), Ok(acc)) => Some((hash, acc)),
                    _ => None,
                })
            })
            .flatten()
            .take(MAX_SNAPSHOT_READS);
        Ok(iter.collect::<Vec<_>>())
    }

    async fn read_storage_snapshot(
        &self,
        account_hash: H256,
        start: H256,
    ) -> Result<Vec<(H256, U256)>, StoreError> {
        let txn = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        let cursor = txn
            .cursor::<StorageSnapShot>()
            .map_err(StoreError::LibmdbxError)?;
        let iter = cursor
            .walk_key(account_hash.into(), Some(start.into()))
            .map_while(|res| {
                res.ok()
                    .map(|(k, v)| (H256(k.0), U256::from_big_endian(&v.0)))
            })
            .take(MAX_SNAPSHOT_READS);
        Ok(iter.collect::<Vec<_>>())
    }

    async fn get_latest_valid_ancestor(
        &self,
        block: BlockHash,
    ) -> Result<Option<BlockHash>, StoreError> {
        self.read::<InvalidAncestors>(block.into())
            .await
            .map(|o| o.map(|a| a.to()))?
            .transpose()
            .map_err(StoreError::from)
    }

    async fn set_latest_valid_ancestor(
        &self,
        bad_block: BlockHash,
        latest_valid: BlockHash,
    ) -> Result<(), StoreError> {
        self.write::<InvalidAncestors>(bad_block.into(), latest_valid.into())
            .await
    }

    async fn setup_genesis_flat_account_storage(
        &self,
        genesis_block_number: u64,
        genesis_block_hash: H256,
        genesis_accounts: &[(Address, H256, U256)],
    ) -> Result<(), StoreError> {
        let tx = self.db.begin_readwrite()?;
        let mut cursor = tx.cursor::<FlatAccountStorage>()?;
        for (addr, slot, value) in genesis_accounts.iter().cloned() {
            let key = (addr.into(), slot.into());
            if !value.is_zero() {
                cursor.upsert(key, value.into())?;
            } else if cursor.seek_exact(key)?.is_some() {
                cursor.delete_current()?;
            }
        }
        tx.upsert::<FlatTablesBlockMetadata>(
            FlatTablesBlockMetadataKey {},
            (genesis_block_number, genesis_block_hash).into(),
        )?;
        tx.commit().map_err(StoreError::LibmdbxError)
    }

    async fn setup_genesis_flat_account_info(
        &self,
        genesis_block_number: u64,
        genesis_block_hash: H256,
        genesis_accounts: &[(Address, u64, U256, H256, bool)],
    ) -> Result<(), StoreError> {
        let tx = self.db.begin_readwrite()?;
        let mut cursor = tx.cursor::<FlatAccountInfo>()?;
        for (addr, nonce, balance, code_hash, removed) in genesis_accounts.iter().cloned() {
            let key = addr.into();
            if removed {
                if cursor.seek_exact(key)?.is_some() {
                    cursor.delete_current()?;
                }
            } else {
                cursor.upsert(key, (code_hash, balance, nonce).into())?
            }
        }
        tx.upsert::<FlatTablesBlockMetadata>(
            FlatTablesBlockMetadataKey {},
            (genesis_block_number, genesis_block_hash).into(),
        )?;
        tx.commit().map_err(StoreError::LibmdbxError)
    }

    fn get_block_for_current_snapshot(&self) -> Result<Option<BlockHash>, StoreError> {
        Ok(self
            .db
            .begin_read()
            .map_err(StoreError::LibmdbxError)?
            .get::<FlatTablesBlockMetadata>(FlatTablesBlockMetadataKey {})
            .map_err(StoreError::LibmdbxError)?
            .map(|v| v.block_hash))
    }
    fn get_current_storage(&self, address: Address, key: H256) -> Result<Option<U256>, StoreError> {
        let tx = self.db.begin_read().map_err(StoreError::LibmdbxError)?;
        Ok(tx
            .get::<FlatAccountStorage>((address.into(), key.into()))?
            .map(Into::into))
    }

    fn get_current_account_info(
        &self,
        address: Address,
    ) -> Result<Option<AccountInfo>, StoreError> {
        let tx = self.db.begin_read()?;
        Ok(tx.get::<FlatAccountInfo>(address.into())?.map(|i| i.0))
    }
}

impl Debug for Store {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Libmdbx Store").finish()
    }
}

/// For `dupsort` tables, multiple values can be stored under the same key.
/// To maintain an explicit order, each value is assigned an `index`.
/// This is useful when storing large byte sequences that exceed the maximum size limit,
/// requiring them to be split into smaller chunks for storage.
pub struct IndexedChunk<T: RLPEncode + RLPDecode> {
    index: u8,
    value: Rlp<T>,
}

pub trait ChunkTrait<T: RLPEncode + RLPDecode> {
    #[allow(unused)]
    fn index(&self) -> u8;
    fn value_bytes(&self) -> &Vec<u8>;
}

impl<T: RLPEncode + RLPDecode> ChunkTrait<T> for IndexedChunk<T> {
    fn index(&self) -> u8 {
        self.index
    }

    fn value_bytes(&self) -> &Vec<u8> {
        self.value.bytes()
    }
}

impl<T: Send + Sync + RLPEncode + RLPDecode> Decodable for IndexedChunk<T> {
    fn decode(b: &[u8]) -> anyhow::Result<Self> {
        let index = b[0];
        let value = Rlp::from_bytes(b[1..].to_vec());
        Ok(Self { index, value })
    }
}

impl<T: Send + Sync + RLPEncode + RLPDecode> Encodable for IndexedChunk<T> {
    type Encoded = Vec<u8>;

    fn encode(self) -> Self::Encoded {
        // by appending the index at the begging, we enforce the btree ordering from lowest to highest
        let mut buf = vec![self.index];
        buf.extend_from_slice(self.value.bytes());
        buf
    }
}

impl<T: RLPEncode + RLPDecode> IndexedChunk<T> {
    /// Splits a value into a indexed chunks if it exceeds the maximum storage size.
    /// Each chunk is assigned an index to ensure correct ordering when retrieved.
    ///
    /// Warning: The current implementation supports a maximum of 256 chunks per value
    /// because the index is stored as a u8.
    ///
    /// If the data exceeds this limit, `None` is returned to indicate that it cannot be stored.
    pub fn from<Tab: Table>(key: Tab::Key, bytes: &[u8]) -> Option<Vec<(Tab::Key, Self)>>
    where
        Tab::Key: Clone,
    {
        let chunks: Vec<Vec<u8>> = bytes
            // -1 to account for the index byte
            .chunks(DB_MAX_VALUE_SIZE - 1)
            .map(|i| i.to_vec())
            .collect();

        if chunks.len() > 256 {
            return None;
        }

        let chunks = chunks
            .into_iter()
            .enumerate()
            .map(|(index, chunk)| {
                (
                    key.clone(),
                    IndexedChunk {
                        index: index as u8,
                        value: Rlp::from_bytes(chunk),
                    },
                )
            })
            .collect();

        Some(chunks)
    }

    /// Reads multiple stored chunks and reconstructs the original full value.
    /// The chunks are appended in order based on their assigned index.
    pub fn read_from_db<Tab: Table + DupSort, K: TransactionKind>(
        cursor: &mut libmdbx::orm::Cursor<'_, K, Tab>,
        key: Tab::Key,
    ) -> Result<Option<T>, StoreError>
    where
        Tab::Key: Decodable,
        Tab::Value: ChunkTrait<T>,
    {
        let mut value = vec![];

        if let Some((_, chunk)) = cursor.seek_exact(key).map_err(StoreError::LibmdbxError)? {
            value.extend_from_slice(chunk.value_bytes());
        } else {
            return Ok(None);
        }

        // Fetch remaining parts
        while let Some((_, chunk)) = cursor.next_value().map_err(StoreError::LibmdbxError)? {
            value.extend_from_slice(chunk.value_bytes());
        }

        let decoded = T::decode(&value).map_err(StoreError::RLPDecode)?;
        Ok(Some(decoded))
    }
}

impl Encodable for ChainDataIndex {
    type Encoded = [u8; 4];

    fn encode(self) -> Self::Encoded {
        (self as u32).encode()
    }
}

impl Encodable for SnapStateIndex {
    type Encoded = [u8; 4];

    fn encode(self) -> Self::Encoded {
        (self as u32).encode()
    }
}

/// default page size recommended by libmdbx
///
/// - See here: https://github.com/erthink/libmdbx/tree/master?tab=readme-ov-file#limitations
/// - and here: https://libmdbx.dqdkfa.ru/structmdbx_1_1env_1_1geometry.html#a45048bf2de9120d01dae2151c060d459
const DB_PAGE_SIZE: usize = 4096;
/// For a default page size of 4096, the max value size is roughly 1/2 page size.
const DB_MAX_VALUE_SIZE: usize = 2022;
// Maximum DB size, set to 2 TB
const MAX_MAP_SIZE: isize = 1024_isize.pow(4) * 2; // 2 TB

/// Initializes a new database with the provided path. If the path is `None`, the database
/// will be temporary.
pub fn init_db(path: Option<impl AsRef<Path>>) -> anyhow::Result<Database> {
    let tables = [
        table_info!(BlockNumbers),
        table_info!(Headers),
        table_info!(Bodies),
        table_info!(AccountCodes),
        table_info!(Receipts),
        table_info!(TransactionLocations),
        table_info!(ChainData),
        table_info!(StateTrieNodes),
        table_info!(StorageTriesNodes),
        table_info!(CanonicalBlockHashes),
        table_info!(Payloads),
        table_info!(PendingBlocks),
        table_info!(SnapState),
        table_info!(StateSnapShot),
        table_info!(StorageSnapShot),
        table_info!(StorageHealPaths),
        table_info!(InvalidAncestors),
        table_info!(FlatTablesBlockMetadata),
        table_info!(FlatAccountStorage),
        table_info!(FlatAccountInfo),
        table_info!(StateTriePruningLog),
        table_info!(StorageTriesPruningLog),
        table_info!(AccountsStorageWriteLog),
        table_info!(AccountsStateWriteLog),
    ]
    .into_iter()
    .collect();
    let path = path.map(|p| p.as_ref().to_path_buf());
    let options = DatabaseOptions {
        page_size: Some(PageSize::Set(DB_PAGE_SIZE)),
        mode: Mode::ReadWrite(ReadWriteOptions {
            max_size: Some(MAX_MAP_SIZE),
            ..Default::default()
        }),
        max_tables: Some(1024),
        max_readers: Some(1024),
        ..Default::default()
    };
    Database::create_with_options(path, options, &tables)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rlp::TupleRLP;
    use bytes::Bytes;
    use ethrex_common::{
        Address, H256,
        types::{BlockHash, Index, Log, TxType},
    };

    #[test]
    fn mdbx_smoke_test() {
        // Declare tables used for the smoke test
        table!(
            /// Example table.
            ( Example ) String => String
        );

        // Assemble database chart
        let tables = [table_info!(Example)].into_iter().collect();

        let key = "Hello".to_string();
        let value = "World!".to_string();

        let db = Database::create(None, &tables).unwrap();

        // Write values
        {
            let txn = db.begin_readwrite().unwrap();
            txn.upsert::<Example>(key.clone(), value.clone()).unwrap();
            txn.commit().unwrap();
        }
        // Read written values
        let read_value = {
            let txn = db.begin_read().unwrap();
            txn.get::<Example>(key).unwrap()
        };
        assert_eq!(read_value, Some(value));
    }

    #[test]
    fn mdbx_structs_smoke_test() {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct ExampleKey([u8; 32]);

        impl Encodable for ExampleKey {
            type Encoded = [u8; 32];

            fn encode(self) -> Self::Encoded {
                Encodable::encode(self.0)
            }
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct ExampleValue {
            x: u64,
            y: [u8; 32],
        }

        impl Encodable for ExampleValue {
            type Encoded = [u8; 40];

            fn encode(self) -> Self::Encoded {
                let mut encoded = [0u8; 40];
                encoded[..8].copy_from_slice(&self.x.to_ne_bytes());
                encoded[8..].copy_from_slice(&self.y);
                encoded
            }
        }

        impl Decodable for ExampleValue {
            fn decode(b: &[u8]) -> anyhow::Result<Self> {
                let x = u64::from_ne_bytes(b[..8].try_into()?);
                let y = b[8..].try_into()?;
                Ok(Self { x, y })
            }
        }

        // Declare tables used for the smoke test
        table!(
            /// Example table.
            ( StructsExample ) ExampleKey => ExampleValue
        );

        // Assemble database chart
        let tables = [table_info!(StructsExample)].into_iter().collect();
        let key = ExampleKey([151; 32]);
        let value = ExampleValue { x: 42, y: [42; 32] };

        let db = Database::create(None, &tables).unwrap();

        // Write values
        {
            let txn = db.begin_readwrite().unwrap();
            txn.upsert::<StructsExample>(key, value).unwrap();
            txn.commit().unwrap();
        }
        // Read written values
        let read_value = {
            let txn = db.begin_read().unwrap();
            txn.get::<StructsExample>(key).unwrap()
        };
        assert_eq!(read_value, Some(value));
    }

    #[test]
    fn mdbx_dupsort_smoke_test() {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct ExampleKey(u8);

        impl Encodable for ExampleKey {
            type Encoded = [u8; 1];

            fn encode(self) -> Self::Encoded {
                [self.0]
            }
        }
        impl Decodable for ExampleKey {
            fn decode(b: &[u8]) -> anyhow::Result<Self> {
                if b.len() != 1 {
                    anyhow::bail!("Invalid length");
                }
                Ok(Self(b[0]))
            }
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct ExampleValue {
            x: u64,
            y: [u8; 32],
        }

        impl Encodable for ExampleValue {
            type Encoded = [u8; 40];

            fn encode(self) -> Self::Encoded {
                let mut encoded = [0u8; 40];
                encoded[..8].copy_from_slice(&self.x.to_ne_bytes());
                encoded[8..].copy_from_slice(&self.y);
                encoded
            }
        }

        impl Decodable for ExampleValue {
            fn decode(b: &[u8]) -> anyhow::Result<Self> {
                let x = u64::from_ne_bytes(b[..8].try_into()?);
                let y = b[8..].try_into()?;
                Ok(Self { x, y })
            }
        }

        // Declare tables used for the smoke test
        dupsort!(
            /// Example table.
            ( DupsortExample ) ExampleKey => (ExampleKey, ExampleValue) [ExampleKey]
        );

        // Assemble database chart
        let tables = [table_info!(DupsortExample)].into_iter().collect();
        let key = ExampleKey(151);
        let subkey1 = ExampleKey(16);
        let subkey2 = ExampleKey(42);
        let value = ExampleValue { x: 42, y: [42; 32] };

        let db = Database::create(None, &tables).unwrap();

        // Write values
        {
            let txn = db.begin_readwrite().unwrap();
            txn.upsert::<DupsortExample>(key, (subkey1, value)).unwrap();
            txn.upsert::<DupsortExample>(key, (subkey2, value)).unwrap();
            txn.commit().unwrap();
        }
        // Read written values
        {
            let txn = db.begin_read().unwrap();
            let mut cursor = txn.cursor::<DupsortExample>().unwrap();
            let value1 = cursor.seek_exact(key).unwrap().unwrap();
            assert_eq!(value1, (key, (subkey1, value)));
            let value2 = cursor.seek_value(key, subkey2).unwrap().unwrap();
            assert_eq!(value2, (subkey2, value));
        };

        // Walk through duplicates
        {
            let txn = db.begin_read().unwrap();
            let cursor = txn.cursor::<DupsortExample>().unwrap();
            let mut acc = 0;
            for key in cursor.walk_key(key, None).map(|r| r.unwrap().0.0) {
                acc += key;
            }

            assert_eq!(acc, 58);
        }
    }

    // Test IndexedChunks implementation with receipts as the type
    #[test]
    fn mdbx_indexed_chunks_test() {
        dupsort!(
            /// Receipts table.
            ( Receipts ) TupleRLP<BlockHash, Index>[Index] => IndexedChunk<Receipt>
        );

        let tables = [table_info!(Receipts)].into_iter().collect();
        let options = DatabaseOptions {
            page_size: Some(PageSize::Set(DB_PAGE_SIZE)),
            mode: Mode::ReadWrite(ReadWriteOptions {
                max_size: Some(MAX_MAP_SIZE),
                ..Default::default()
            }),
            ..Default::default()
        };
        let db = Database::create_with_options(None, options, &tables).unwrap();

        let mut receipts = vec![];
        for i in 0..10 {
            receipts.push(generate_big_receipt(100 * (i + 1), 10, 10 * (i + 1)));
        }

        // encode receipts
        let block_hash = H256::random();
        let mut key_values = vec![];
        for (i, receipt) in receipts.iter().enumerate() {
            let key = (block_hash, i as u64).into();
            let receipt_rlp = receipt.encode_to_vec();
            let Some(mut entries) = IndexedChunk::from::<Receipts>(key, &receipt_rlp) else {
                continue;
            };
            key_values.append(&mut entries);
        }

        // store values
        let txn = db.begin_readwrite().unwrap();
        let mut cursor = txn.cursor::<Receipts>().unwrap();
        for (key, value) in key_values {
            cursor.upsert(key, value).unwrap()
        }
        txn.commit().unwrap();

        // now retrieve the values and assert they are the same
        let mut stored_receipts = vec![];
        let mut receipt_index = 0;
        let mut key: TupleRLP<BlockHash, Index> = (block_hash, 0).into();
        let txn = db.begin_read().unwrap();
        let mut cursor = txn.cursor::<Receipts>().unwrap();
        while let Some(receipt) = IndexedChunk::read_from_db(&mut cursor, key).unwrap() {
            stored_receipts.push(receipt);
            receipt_index += 1;
            key = (block_hash, receipt_index).into();
        }

        assert_eq!(receipts, stored_receipts);
    }

    // This test verifies the 256-chunk-per-value limitation on indexed chunks.
    // Given a value size of 2022 bytes, we can store up to 256 * 2022 = 517,632 - 256 bytes.
    // The 256 subtraction accounts for the index byte overhead.
    // We expect that exceeding this storage limit results in a `None` when writing.
    #[test]
    fn indexed_chunk_storage_limit_exceeded() {
        dupsort!(
            /// example table.
            ( Example ) BlockHashRLP[Index] => IndexedChunk<Vec<u8>>
        );

        let tables = [table_info!(Example)].into_iter().collect();
        let options = DatabaseOptions {
            page_size: Some(PageSize::Set(DB_PAGE_SIZE)),
            mode: Mode::ReadWrite(ReadWriteOptions {
                max_size: Some(MAX_MAP_SIZE),
                ..Default::default()
            }),
            ..Default::default()
        };
        let _ = Database::create_with_options(None, options, &tables).unwrap();

        let block_hash = H256::random();

        // we want to store the maximum
        let max_data_bytes: usize = 517377;
        let data = Bytes::from(vec![1u8; max_data_bytes]);
        let key = block_hash.into();
        let entries = IndexedChunk::<Vec<u8>>::from::<Example>(key, &data);

        assert!(entries.is_none());
    }

    // This test verifies the 256-chunk-per-value limitation on indexed chunks.
    // Given a value size of 2022 bytes, we can store up to 256 * 2022 = 517,632 - 256 bytes.
    // The 256 subtraction accounts for the index byte overhead.
    // We expect that we can write up to that storage limit.
    #[test]
    fn indexed_chunk_storage_store_max_limit() {
        dupsort!(
            /// example table.
            ( Example ) BlockHashRLP[Index] => IndexedChunk<Vec<u8>>
        );

        let tables = [table_info!(Example)].into_iter().collect();
        let options = DatabaseOptions {
            page_size: Some(PageSize::Set(DB_PAGE_SIZE)),
            mode: Mode::ReadWrite(ReadWriteOptions {
                max_size: Some(MAX_MAP_SIZE),
                ..Default::default()
            }),
            ..Default::default()
        };
        let db = Database::create_with_options(None, options, &tables).unwrap();

        let block_hash = H256::random();

        // we want to store the maximum
        let max_data_bytes: usize = 517376;
        let data = Bytes::from(vec![1u8; max_data_bytes]);
        let key = block_hash.into();
        let entries = IndexedChunk::<Vec<u8>>::from::<Example>(key, &data).unwrap();

        // store values
        let txn = db.begin_readwrite().unwrap();
        let mut cursor = txn.cursor::<Example>().unwrap();
        for (k, v) in entries {
            cursor.upsert(k, v).unwrap();
        }
        txn.commit().unwrap();
    }

    fn generate_big_receipt(
        data_size_in_bytes: usize,
        logs_size: usize,
        topics_size: usize,
    ) -> Receipt {
        let large_data: Bytes = Bytes::from(vec![1u8; data_size_in_bytes]);
        let large_topics: Vec<H256> = std::iter::repeat_n(H256::random(), topics_size).collect();

        let logs = std::iter::repeat_n(
            Log {
                address: Address::random(),
                topics: large_topics.clone(),
                data: large_data.clone(),
            },
            logs_size,
        )
        .collect();

        Receipt {
            tx_type: TxType::EIP7702,
            succeeded: true,
            cumulative_gas_used: u64::MAX,
            logs,
        }
    }
}
