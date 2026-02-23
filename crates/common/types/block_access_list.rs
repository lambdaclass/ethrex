#![allow(dead_code)]

use bytes::{BufMut, Bytes};
use ethereum_types::{Address, H256, U256};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::{RLPEncode, encode_length, list_length},
    structs,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::constants::{EMPTY_BLOCK_ACCESS_LIST_HASH, SYSTEM_ADDRESS};
use crate::utils::keccak;

/// Encode a slice of items in sorted order without cloning.
fn encode_sorted_by<T, K, F>(items: &[T], buf: &mut dyn BufMut, key_fn: F)
where
    T: RLPEncode,
    K: Ord,
    F: Fn(&T) -> K,
{
    if items.is_empty() {
        buf.put_u8(0xc0);
        return;
    }
    let mut indices: Vec<usize> = (0..items.len()).collect();
    indices.sort_by(|&i, &j| key_fn(&items[i]).cmp(&key_fn(&items[j])));

    let payload_len: usize = items.iter().map(|item| item.length()).sum();
    encode_length(payload_len, buf);
    for &i in &indices {
        items[i].encode(buf);
    }
}

/// Calculate the encoded length of a sorted list.
fn sorted_list_length<T: RLPEncode>(items: &[T]) -> usize {
    if items.is_empty() {
        return 1;
    }
    let payload_len: usize = items.iter().map(|item| item.length()).sum();
    list_length(payload_len)
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct StorageChange {
    /// Block access index per EIP-7928 spec (uint16).
    pub block_access_index: u16,
    pub post_value: U256,
}

impl StorageChange {
    /// Creates a new storage change with the given block access index and post value.
    pub fn new(block_access_index: u16, post_value: U256) -> Self {
        Self {
            block_access_index,
            post_value,
        }
    }
}

impl RLPEncode for StorageChange {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.block_access_index)
            .encode_field(&self.post_value)
            .finish();
    }
}

impl RLPDecode for StorageChange {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = structs::Decoder::new(rlp)?;
        let (block_access_index, decoder) = decoder.decode_field("block_access_index")?;
        let (post_value, decoder) = decoder.decode_field("post_value")?;
        let remaining = decoder.finish()?;
        Ok((
            Self {
                block_access_index,
                post_value,
            },
            remaining,
        ))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct SlotChange {
    pub slot: U256,
    pub slot_changes: Vec<StorageChange>,
}

impl SlotChange {
    /// Creates a new slot change for the given slot.
    pub fn new(slot: U256) -> Self {
        Self {
            slot,
            slot_changes: Vec::new(),
        }
    }

    /// Creates a new slot change with the given slot and changes.
    pub fn with_changes(slot: U256, changes: Vec<StorageChange>) -> Self {
        Self {
            slot,
            slot_changes: changes,
        }
    }

    /// Adds a storage change to this slot.
    pub fn add_change(&mut self, change: StorageChange) {
        self.slot_changes.push(change);
    }
}

impl RLPEncode for SlotChange {
    fn encode(&self, buf: &mut dyn BufMut) {
        let payload_len = self.slot.length() + sorted_list_length(&self.slot_changes);
        encode_length(payload_len, buf);
        self.slot.encode(buf);
        encode_sorted_by(&self.slot_changes, buf, |s| s.block_access_index);
    }
}

impl RLPDecode for SlotChange {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = structs::Decoder::new(rlp)?;
        let (slot, decoder) = decoder.decode_field("slot")?;
        let (slot_changes, decoder) = decoder.decode_field("slot_changes")?;
        let remaining = decoder.finish()?;
        Ok((Self { slot, slot_changes }, remaining))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct BalanceChange {
    /// Block access index per EIP-7928 spec (uint16).
    pub block_access_index: u16,
    pub post_balance: U256,
}

impl BalanceChange {
    /// Creates a new balance change with the given block access index and post balance.
    pub fn new(block_access_index: u16, post_balance: U256) -> Self {
        Self {
            block_access_index,
            post_balance,
        }
    }
}

impl RLPEncode for BalanceChange {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.block_access_index)
            .encode_field(&self.post_balance)
            .finish();
    }
}

impl RLPDecode for BalanceChange {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = structs::Decoder::new(rlp)?;
        let (block_access_index, decoder) = decoder.decode_field("block_access_index")?;
        let (post_balance, decoder) = decoder.decode_field("post_balance")?;
        let remaining = decoder.finish()?;
        Ok((
            Self {
                block_access_index,
                post_balance,
            },
            remaining,
        ))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct NonceChange {
    /// Block access index per EIP-7928 spec (uint16).
    pub block_access_index: u16,
    pub post_nonce: u64,
}

impl NonceChange {
    /// Creates a new nonce change with the given block access index and post nonce.
    pub fn new(block_access_index: u16, post_nonce: u64) -> Self {
        Self {
            block_access_index,
            post_nonce,
        }
    }
}

impl RLPEncode for NonceChange {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.block_access_index)
            .encode_field(&self.post_nonce)
            .finish();
    }
}

impl RLPDecode for NonceChange {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = structs::Decoder::new(rlp)?;
        let (block_access_index, decoder) = decoder.decode_field("block_access_index")?;
        let (post_nonce, decoder) = decoder.decode_field("post_nonce")?;
        let remaining = decoder.finish()?;
        Ok((
            Self {
                block_access_index,
                post_nonce,
            },
            remaining,
        ))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct CodeChange {
    /// Block access index per EIP-7928 spec (uint16).
    pub block_access_index: u16,
    pub new_code: Bytes,
}

impl CodeChange {
    /// Creates a new code change with the given block access index and new code.
    pub fn new(block_access_index: u16, new_code: Bytes) -> Self {
        Self {
            block_access_index,
            new_code,
        }
    }
}

impl RLPEncode for CodeChange {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        structs::Encoder::new(buf)
            .encode_field(&self.block_access_index)
            .encode_field(&self.new_code)
            .finish();
    }
}

impl RLPDecode for CodeChange {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = structs::Decoder::new(rlp)?;
        let (block_access_index, decoder) = decoder.decode_field("block_access_index")?;
        let (new_code, decoder) = decoder.decode_field("new_code")?;
        let remaining = decoder.finish()?;
        Ok((
            Self {
                block_access_index,
                new_code,
            },
            remaining,
        ))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct AccountChanges {
    pub address: Address,
    pub storage_changes: Vec<SlotChange>,
    pub storage_reads: Vec<U256>,
    pub balance_changes: Vec<BalanceChange>,
    pub nonce_changes: Vec<NonceChange>,
    pub code_changes: Vec<CodeChange>,
}

impl AccountChanges {
    /// Creates a new account changes struct for the given address.
    pub fn new(address: Address) -> Self {
        Self {
            address,
            storage_changes: Vec::new(),
            storage_reads: Vec::new(),
            balance_changes: Vec::new(),
            nonce_changes: Vec::new(),
            code_changes: Vec::new(),
        }
    }

    pub fn with_storage_changes(mut self, changes: Vec<SlotChange>) -> Self {
        self.storage_changes = changes;
        self
    }

    pub fn with_storage_reads(mut self, reads: Vec<U256>) -> Self {
        self.storage_reads = reads;
        self
    }

    pub fn with_balance_changes(mut self, changes: Vec<BalanceChange>) -> Self {
        self.balance_changes = changes;
        self
    }

    pub fn with_nonce_changes(mut self, changes: Vec<NonceChange>) -> Self {
        self.nonce_changes = changes;
        self
    }

    pub fn with_code_changes(mut self, changes: Vec<CodeChange>) -> Self {
        self.code_changes = changes;
        self
    }

    /// Adds a slot change (storage write) to this account.
    pub fn add_storage_change(&mut self, slot_change: SlotChange) {
        self.storage_changes.push(slot_change);
    }

    /// Adds a storage read (slot that was only read, not written) to this account.
    pub fn add_storage_read(&mut self, slot: U256) {
        self.storage_reads.push(slot);
    }

    /// Adds a balance change to this account.
    pub fn add_balance_change(&mut self, change: BalanceChange) {
        self.balance_changes.push(change);
    }

    /// Adds a nonce change to this account.
    pub fn add_nonce_change(&mut self, change: NonceChange) {
        self.nonce_changes.push(change);
    }

    /// Adds a code change to this account.
    pub fn add_code_change(&mut self, change: CodeChange) {
        self.code_changes.push(change);
    }

    /// Returns an iterator over all storage slots that need prefetching
    /// (both reads and writes need their pre-state loaded).
    pub fn all_storage_slots(&self) -> impl Iterator<Item = U256> + '_ {
        self.storage_reads
            .iter()
            .copied()
            .chain(self.storage_changes.iter().map(|sc| sc.slot))
    }

    /// Returns whether this account has any changes or reads.
    pub fn is_empty(&self) -> bool {
        self.storage_changes.is_empty()
            && self.storage_reads.is_empty()
            && self.balance_changes.is_empty()
            && self.nonce_changes.is_empty()
            && self.code_changes.is_empty()
    }
}

impl RLPEncode for AccountChanges {
    fn encode(&self, buf: &mut dyn BufMut) {
        let payload_len = self.address.length()
            + sorted_list_length(&self.storage_changes)
            + sorted_list_length(&self.storage_reads)
            + sorted_list_length(&self.balance_changes)
            + sorted_list_length(&self.nonce_changes)
            + sorted_list_length(&self.code_changes);

        encode_length(payload_len, buf);
        self.address.encode(buf);
        encode_sorted_by(&self.storage_changes, buf, |s| s.slot);
        encode_sorted_by(&self.storage_reads, buf, |s| *s);
        encode_sorted_by(&self.balance_changes, buf, |b| b.block_access_index);
        encode_sorted_by(&self.nonce_changes, buf, |n| n.block_access_index);
        encode_sorted_by(&self.code_changes, buf, |c| c.block_access_index);
    }
}

impl RLPDecode for AccountChanges {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let decoder = structs::Decoder::new(rlp)?;
        let (address, decoder) = decoder.decode_field("address")?;
        let (storage_changes, decoder) = decoder.decode_field("storage_changes")?;
        let (storage_reads, decoder) = decoder.decode_field("storage_reads")?;
        let (balance_changes, decoder) = decoder.decode_field("balance_changes")?;
        let (nonce_changes, decoder) = decoder.decode_field("nonce_changes")?;
        let (code_changes, decoder) = decoder.decode_field("code_changes")?;
        let remaining = decoder.finish()?;
        Ok((
            Self {
                address,
                storage_changes,
                storage_reads,
                balance_changes,
                nonce_changes,
                code_changes,
            },
            remaining,
        ))
    }
}

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct BlockAccessList {
    inner: Vec<AccountChanges>,
}

impl BlockAccessList {
    /// Creates a new empty block access list.
    pub fn new() -> Self {
        Self { inner: Vec::new() }
    }

    /// Creates a block access list from a vector of account changes.
    pub fn from_accounts(accounts: Vec<AccountChanges>) -> Self {
        Self { inner: accounts }
    }

    /// Creates a new block access list with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Vec::with_capacity(capacity),
        }
    }

    /// Adds an account changes entry to the block access list.
    pub fn add_account_changes(&mut self, changes: AccountChanges) {
        self.inner.push(changes);
    }

    /// Returns true if the BAL is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns an iterator over account changes.
    pub fn accounts(&self) -> &[AccountChanges] {
        &self.inner
    }

    /// Computes the hash of the block access list (sorts accounts by address per EIP-7928).
    /// Use this when hashing a BAL constructed locally from execution.
    pub fn compute_hash(&self) -> H256 {
        if self.inner.is_empty() {
            return *EMPTY_BLOCK_ACCESS_LIST_HASH;
        }

        let buf = self.encode_to_vec();
        keccak(buf)
    }
}

impl RLPEncode for BlockAccessList {
    fn encode(&self, buf: &mut dyn BufMut) {
        encode_sorted_by(&self.inner, buf, |a| a.address);
    }
}

impl RLPDecode for BlockAccessList {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let (inner, remaining) = RLPDecode::decode_unfinished(rlp)?;
        Ok((Self { inner }, remaining))
    }
}

/// A checkpoint of the BAL recorder state that can be restored on revert.
///
/// Per EIP-7928: "State changes from reverted calls are discarded, but all accessed
/// addresses must be included." This checkpoint captures the state change data
/// (storage, balance, nonce, code changes) but NOT touched_addresses, which persist
/// across reverts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockAccessListCheckpoint {
    /// Number of promoted reads per address at checkpoint time.
    /// Reads that became writes are tracked in a Vec for ordered truncation.
    reads_promoted_len: BTreeMap<Address, usize>,
    /// For each address+slot, the number of writes at checkpoint time.
    storage_writes_len: BTreeMap<Address, BTreeMap<U256, usize>>,
    /// Number of balance changes per address at checkpoint time.
    balance_changes_len: BTreeMap<Address, usize>,
    /// Number of nonce changes per address at checkpoint time.
    nonce_changes_len: BTreeMap<Address, usize>,
    /// Number of code changes per address at checkpoint time.
    code_changes_len: BTreeMap<Address, usize>,
}

/// Records state accesses during block execution to build a Block Access List (EIP-7928).
///
/// The recorder accumulates all storage reads/writes, balance changes, nonce changes,
/// and code changes during execution. At the end, it can be converted into a `BlockAccessList`.
///
/// # Block Access Index Semantics
/// - 0: System contracts (pre-execution phase)
/// - 1..n: Transaction indices (1-indexed)
/// - n+1: Post-execution phase (withdrawals)
#[derive(Debug, Default, Clone)]
pub struct BlockAccessListRecorder {
    /// Current block access index per EIP-7928 spec (uint16).
    /// 0=pre-exec, 1..n=tx indices, n+1=post-exec.
    current_index: u16,
    /// All addresses that must be in BAL (touched during execution).
    touched_addresses: BTreeSet<Address>,
    /// Storage reads per address (slot -> set of slots read but not written).
    storage_reads: BTreeMap<Address, BTreeSet<U256>>,
    /// Storage writes per address (slot -> list of (index, post_value) pairs).
    storage_writes: BTreeMap<Address, BTreeMap<U256, Vec<(u16, U256)>>>,
    /// Initial balances for detecting balance round-trips.
    /// Used as the starting point for per-transaction round-trip detection in build().
    initial_balances: BTreeMap<Address, U256>,
    /// Per-transaction initial storage values for net-zero filtering.
    /// Per EIP-7928: "If a storage slot's value is changed but its post-transaction value
    /// is equal to its pre-transaction value, the slot MUST NOT be recorded as modified."
    /// Key is (address, slot), value is the pre-transaction value.
    tx_initial_storage: BTreeMap<(Address, U256), U256>,
    /// Per-transaction initial code for net-zero filtering.
    /// Per EIP-7928: similar to storage, if code changes but post-transaction code equals
    /// pre-transaction code (e.g., delegate then reset), it MUST NOT be recorded.
    tx_initial_code: BTreeMap<Address, Bytes>,
    /// Balance changes per address (list of (index, post_balance) pairs).
    balance_changes: BTreeMap<Address, Vec<(u16, U256)>>,
    /// Nonce changes per address (list of (index, post_nonce) pairs).
    nonce_changes: BTreeMap<Address, Vec<(u16, u64)>>,
    /// Code changes per address (list of (index, new_code) pairs).
    code_changes: BTreeMap<Address, Vec<(u16, Bytes)>>,
    /// Addresses that had non-empty code at the start (before any code changes).
    /// Used to distinguish CREATE-with-empty-code (no initial code → empty = no change)
    /// from delegation-clear (had code → empty = actual change).
    addresses_with_initial_code: BTreeSet<Address>,
    /// Tracks reads that were promoted to writes, in insertion order per address.
    /// Used for efficient checkpoint/restore without cloning storage_reads.
    /// On restore, we truncate this Vec and the slots go back to being reads.
    reads_promoted_to_writes: BTreeMap<Address, Vec<U256>>,
    /// When true, SYSTEM_ADDRESS balance/nonce/touch changes are filtered out.
    /// Set during system contract calls (EIP-2935, EIP-4788, etc.) where the
    /// system address account is backed up and restored, so changes are transient.
    in_system_call: bool,
}

impl BlockAccessListRecorder {
    /// Creates a new empty recorder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the current block access index per EIP-7928 spec (uint16).
    /// Call this before each transaction (index 1..n) and for withdrawals (n+1).
    ///
    /// Filters net-zero storage writes and code changes for the current transaction
    /// before switching to a new transaction index.
    pub fn set_block_access_index(&mut self, index: u16) {
        // Filter net-zero changes and clear per-transaction initial values when switching transactions
        if self.current_index != index {
            // Filter net-zero storage writes and code changes for the current transaction before switching
            self.filter_net_zero_storage();
            self.filter_net_zero_code();
            self.tx_initial_storage.clear();
            self.tx_initial_code.clear();
        }
        self.current_index = index;
    }

    /// Filters net-zero storage writes for the current transaction.
    /// Per EIP-7928: "If a storage slot's value is changed but its post-transaction value
    /// is equal to its pre-transaction value, the slot MUST NOT be recorded as modified."
    /// Net-zero writes are converted to reads instead.
    fn filter_net_zero_storage(&mut self) {
        let current_idx = self.current_index;

        // Collect slots that need to be converted from writes to reads
        let mut slots_to_convert: Vec<(Address, U256)> = Vec::new();

        for ((addr, slot), pre_value) in &self.tx_initial_storage {
            // Check if there are writes for this slot in the current transaction
            if let Some(slots) = self.storage_writes.get(addr)
                && let Some(changes) = slots.get(slot)
            {
                // Find the final value for this transaction
                // (last entry with current_idx, or no entry means no change in this tx)
                let final_value = changes
                    .iter()
                    .filter(|(idx, _)| *idx == current_idx)
                    .next_back()
                    .map(|(_, val)| *val);

                if let Some(final_val) = final_value
                    && final_val == *pre_value
                {
                    // Net-zero: final value equals pre-transaction value
                    slots_to_convert.push((*addr, *slot));
                }
            }
        }

        // Convert net-zero writes to reads
        for (addr, slot) in slots_to_convert {
            // Remove the write entries for the current transaction
            if let Some(slots) = self.storage_writes.get_mut(&addr) {
                if let Some(changes) = slots.get_mut(&slot) {
                    changes.retain(|(idx, _)| *idx != current_idx);
                    // If no changes remain for this slot, remove the slot entry
                    if changes.is_empty() {
                        slots.remove(&slot);
                    }
                }
                // If no slots remain for this address, remove the address entry
                if slots.is_empty() {
                    self.storage_writes.remove(&addr);
                }
            }

            // If this slot was promoted from read to write, undo the promotion
            // so build() doesn't skip it from storage_reads.
            if let Some(promoted) = self.reads_promoted_to_writes.get_mut(&addr) {
                promoted.retain(|s| *s != slot);
                if promoted.is_empty() {
                    self.reads_promoted_to_writes.remove(&addr);
                }
            }

            // Add as a read instead
            self.storage_reads.entry(addr).or_default().insert(slot);
        }
    }

    /// Filters net-zero code changes for the current transaction.
    /// Per EIP-7928: similar to storage, if code changes but post-transaction code equals
    /// pre-transaction code (e.g., delegate then reset in same tx), it should not be recorded.
    fn filter_net_zero_code(&mut self) {
        let current_idx = self.current_index;

        // Collect addresses with net-zero code changes
        let mut addrs_to_remove: Vec<Address> = Vec::new();

        for (addr, pre_code) in &self.tx_initial_code {
            // Check if there are code changes for this address in the current transaction
            if let Some(changes) = self.code_changes.get(addr) {
                // Find the final code for this transaction
                let final_code = changes
                    .iter()
                    .filter(|(idx, _)| *idx == current_idx)
                    .last()
                    .map(|(_, code)| code);

                if let Some(final_code) = final_code
                    && final_code == pre_code
                {
                    // Net-zero: final code equals pre-transaction code
                    addrs_to_remove.push(*addr);
                }
            }
        }

        // Remove net-zero code changes
        for addr in addrs_to_remove {
            if let Some(changes) = self.code_changes.get_mut(&addr) {
                changes.retain(|(idx, _)| *idx != current_idx);
                // If no changes remain for this address, remove the address entry
                if changes.is_empty() {
                    self.code_changes.remove(&addr);
                }
            }
        }
    }

    /// Returns the current block access index per EIP-7928 spec (uint16).
    pub fn current_index(&self) -> u16 {
        self.current_index
    }

    /// Marks the recorder as being inside a system contract call.
    /// While in this mode, SYSTEM_ADDRESS balance/nonce/touch changes are filtered out
    /// because system calls backup and restore the system address account state.
    pub fn enter_system_call(&mut self) {
        self.in_system_call = true;
    }

    /// Marks the recorder as no longer inside a system contract call.
    pub fn exit_system_call(&mut self) {
        self.in_system_call = false;
    }

    /// Records an address as touched during execution.
    /// The address will appear in the BAL even if it has no state changes.
    ///
    /// Note: SYSTEM_ADDRESS is excluded during system contract calls.
    pub fn record_touched_address(&mut self, address: Address) {
        if address == SYSTEM_ADDRESS && self.in_system_call {
            return;
        }
        self.touched_addresses.insert(address);
    }

    /// Records multiple addresses as touched during execution.
    /// More efficient than calling `record_touched_address` in a loop.
    ///
    /// Note: SYSTEM_ADDRESS is filtered out during system contract calls.
    pub fn extend_touched_addresses(&mut self, addresses: impl Iterator<Item = Address>) {
        if self.in_system_call {
            self.touched_addresses
                .extend(addresses.filter(|addr| *addr != SYSTEM_ADDRESS));
        } else {
            self.touched_addresses.extend(addresses);
        }
    }

    /// Records a storage slot read.
    /// If the slot is later written, the read will be removed (it becomes a write).
    pub fn record_storage_read(&mut self, address: Address, slot: U256) {
        // Don't record as a read if it's already been written
        if self
            .storage_writes
            .get(&address)
            .is_some_and(|slots| slots.contains_key(&slot))
        {
            return;
        }
        self.storage_reads.entry(address).or_default().insert(slot);
        // Also mark the address as touched
        self.touched_addresses.insert(address);
    }

    /// Records a storage slot write.
    /// If the slot was previously recorded as a read, it is tracked as promoted
    /// (for efficient checkpoint/restore) but kept in storage_reads until build().
    ///
    /// Per EIP-7928: Multiple writes to the same slot within the same transaction
    /// (same block_access_index) only keep the final value.
    pub fn record_storage_write(&mut self, address: Address, slot: U256, post_value: U256) {
        // Track if this read is being promoted to a write (for checkpoint/restore)
        // We don't remove from storage_reads here - filtering happens in build()
        if self
            .storage_reads
            .get(&address)
            .is_some_and(|reads| reads.contains(&slot))
        {
            // Only track promotion if not already tracked
            let promoted = self.reads_promoted_to_writes.entry(address).or_default();
            if !promoted.contains(&slot) {
                promoted.push(slot);
            }
        }

        // Always push a new entry instead of updating in-place.
        // This is necessary for correct checkpoint/restore semantics:
        // restore() truncates the vector by length, so in-place updates
        // would corrupt values that should be preserved after a revert.
        let changes = self
            .storage_writes
            .entry(address)
            .or_default()
            .entry(slot)
            .or_default();

        changes.push((self.current_index, post_value));
        // Mark address as touched (include SYSTEM_ADDRESS for actual state changes)
        self.touched_addresses.insert(address);
    }

    /// Captures the pre-storage value for net-zero filtering.
    /// Should be called BEFORE writing to a storage slot, with the current value.
    /// Uses first-write-wins semantics: only the first call for a given (address, slot)
    /// within a transaction will be recorded.
    pub fn capture_pre_storage(&mut self, address: Address, slot: U256, value: U256) {
        // First-write-wins: only capture if not already captured for this transaction
        self.tx_initial_storage
            .entry((address, slot))
            .or_insert(value);
    }

    /// Records a balance change.
    /// Should be called after every balance modification.
    /// Per EIP-7928, only the final balance per (address, block_access_index) is recorded.
    /// If multiple balance changes occur within the same transaction, only the last one matters.
    /// Note: SYSTEM_ADDRESS balance changes are excluded during system contract calls
    /// (system calls backup/restore the system address account state).
    ///
    /// IMPORTANT: We always push new entries (never update in-place) to support checkpoint/restore.
    /// The checkpoint mechanism captures lengths, not values. If we updated in-place, the restored
    /// value would be the updated one, not the original at checkpoint time.
    /// At build() time, we take only the last entry per transaction for each address.
    pub fn record_balance_change(&mut self, address: Address, post_balance: U256) {
        // SYSTEM_ADDRESS balance changes from system contract calls should not be recorded
        // (system calls backup and restore SYSTEM_ADDRESS state)
        if address == SYSTEM_ADDRESS && self.in_system_call {
            return;
        }

        // Always push new entries to support checkpoint/restore.
        // The last entry for each transaction will be used in build().
        let changes = self.balance_changes.entry(address).or_default();
        changes.push((self.current_index, post_balance));

        // Mark address as touched
        self.touched_addresses.insert(address);
    }

    /// Sets the initial balance for an address before any changes.
    /// This should be called when first accessing an account to enable round-trip detection.
    ///
    /// Per EIP-7928: "If an account's balance changes during a transaction, but its
    /// post-transaction balance is equal to its pre-transaction balance, then the
    /// change MUST NOT be recorded." The initial balance is used in build() to detect
    /// such round-trips on a per-transaction basis.
    pub fn set_initial_balance(&mut self, address: Address, balance: U256) {
        self.initial_balances.entry(address).or_insert(balance);
    }

    /// Records a nonce change.
    /// Per EIP-7928, only record nonces for:
    /// - EOA senders
    /// - Contracts performing CREATE/CREATE2
    /// - Deployed contracts
    /// - EIP-7702 authorities
    ///
    /// Note: SYSTEM_ADDRESS nonce changes from system calls are excluded.
    pub fn record_nonce_change(&mut self, address: Address, post_nonce: u64) {
        // SYSTEM_ADDRESS nonce changes from system contract calls should not be recorded
        if address == SYSTEM_ADDRESS && self.in_system_call {
            return;
        }
        self.nonce_changes
            .entry(address)
            .or_default()
            .push((self.current_index, post_nonce));
        // Mark address as touched
        self.touched_addresses.insert(address);
    }

    /// Records a code change (contract deployment or EIP-7702 delegation).
    /// Marks that an address has non-empty code at the start (before any code changes).
    /// This is used to distinguish:
    /// - CREATE with empty code: no initial code → empty = no change (skip)
    /// - Delegation clear: had code → empty = actual change (record)
    pub fn capture_initial_code_presence(&mut self, address: Address, has_code: bool) {
        if has_code {
            self.addresses_with_initial_code.insert(address);
        }
    }

    /// Captures the initial code for an address before any code changes in the current transaction.
    /// Used for net-zero code change detection (e.g., delegate then reset in same tx).
    /// Only the first call per address per transaction is stored.
    pub fn set_initial_code(&mut self, address: Address, code: Bytes) {
        self.tx_initial_code.entry(address).or_insert(code);
    }

    /// Records a code change (contract deployment or EIP-7702 delegation).
    /// Per EIP-7928:
    /// - Empty code on CREATE (no initial code → empty) is NOT recorded (test_bal_create_transaction_empty_code)
    /// - Empty code on delegation clear (had code → empty) IS recorded (test_bal_7702_delegation_clear)
    pub fn record_code_change(&mut self, address: Address, new_code: Bytes) {
        // If new code is empty, only record if the address had initial code
        // (i.e., this is an actual code change like delegation clear, not just CREATE empty)
        // No initial code and setting to empty = no change, skip
        // Had initial code and setting to empty = delegation clear, record it
        if new_code.is_empty() && !self.addresses_with_initial_code.contains(&address) {
            self.touched_addresses.insert(address);
            return;
        }

        self.code_changes
            .entry(address)
            .or_default()
            .push((self.current_index, new_code));
        // Mark address as touched (include SYSTEM_ADDRESS for actual state changes)
        self.touched_addresses.insert(address);
    }

    /// Merges additional touched addresses from an iterator.
    pub fn merge_touched_addresses(&mut self, addresses: impl Iterator<Item = Address>) {
        for address in addresses {
            self.record_touched_address(address);
        }
    }

    /// Builds the final BlockAccessList from accumulated data.
    ///
    /// This method:
    /// 1. Filters net-zero storage writes for the current transaction
    /// 2. Filters out balance changes per-transaction where the final balance equals the initial balance
    /// 3. Creates AccountChanges entries for all touched addresses
    /// 4. Includes addresses even if they have no state changes (per EIP-7928)
    ///
    /// Per EIP-7928: "If an account's balance changes during a transaction, but its
    /// post-transaction balance is equal to its pre-transaction balance, then the
    /// change MUST NOT be recorded."
    pub fn build(mut self) -> BlockAccessList {
        // Filter net-zero storage writes and code changes for the current (last) transaction
        self.filter_net_zero_storage();
        self.filter_net_zero_code();
        let mut bal = BlockAccessList::with_capacity(self.touched_addresses.len());

        // Process all touched addresses
        for address in &self.touched_addresses {
            let mut account_changes = AccountChanges::new(*address);

            // Add storage writes (slot changes)
            // Deduplicate entries per block_access_index (keep last per idx),
            // since record_storage_write always pushes for correct checkpoint/restore.
            if let Some(slots) = self.storage_writes.get(address) {
                for (slot, changes) in slots {
                    let mut slot_change = SlotChange::new(*slot);
                    let mut deduped: BTreeMap<u16, U256> = BTreeMap::new();
                    for (index, post_value) in changes {
                        deduped.insert(*index, *post_value);
                    }
                    for (index, post_value) in deduped {
                        slot_change.add_change(StorageChange::new(index, post_value));
                    }
                    account_changes.add_storage_change(slot_change);
                }
            }

            // Add storage reads (excluding slots that were promoted to writes)
            if let Some(reads) = self.storage_reads.get(address) {
                let promoted = self.reads_promoted_to_writes.get(address);
                for slot in reads {
                    // Skip if this read was promoted to a write
                    if promoted.is_some_and(|p| p.contains(slot)) {
                        continue;
                    }
                    account_changes.add_storage_read(*slot);
                }
            }

            // Add balance changes (filtered for round-trips per-transaction)
            // Per EIP-7928: "If an account's balance changes during a transaction, but its
            // post-transaction balance is equal to its pre-transaction balance, then the
            // change MUST NOT be recorded."
            if let Some(changes) = self.balance_changes.get(address) {
                // Group balance changes by transaction index
                let mut changes_by_tx: BTreeMap<u16, Vec<U256>> = BTreeMap::new();
                for (index, post_balance) in changes {
                    changes_by_tx.entry(*index).or_default().push(*post_balance);
                }

                // For each transaction, check if balance round-tripped
                // Per EIP-7928: only the FINAL balance per transaction is recorded
                let mut prev_balance = self.initial_balances.get(address).copied();
                for (index, tx_changes) in &changes_by_tx {
                    let initial_for_tx = prev_balance;
                    let final_for_tx = tx_changes.last().copied();

                    // Check if this transaction's balance round-tripped
                    let is_round_trip = match (initial_for_tx, final_for_tx) {
                        (Some(initial), Some(final_bal)) => initial == final_bal,
                        _ => false, // Include if we can't determine
                    };

                    // Only include the FINAL balance change if NOT a round-trip
                    if !is_round_trip && let Some(final_balance) = final_for_tx {
                        account_changes
                            .add_balance_change(BalanceChange::new(*index, final_balance));
                    }

                    // Update prev_balance for next transaction
                    prev_balance = final_for_tx;
                }
            }

            // Add nonce changes (only FINAL nonce per transaction)
            // Per EIP-7928, similar to balance changes, we only record the final nonce per tx.
            if let Some(changes) = self.nonce_changes.get(address) {
                // Group nonce changes by transaction index
                let mut changes_by_tx: BTreeMap<u16, u64> = BTreeMap::new();
                for (index, post_nonce) in changes {
                    // Only keep the final nonce for each transaction (last write wins)
                    changes_by_tx.insert(*index, *post_nonce);
                }

                for (index, post_nonce) in changes_by_tx {
                    account_changes.add_nonce_change(NonceChange::new(index, post_nonce));
                }
            }

            // Add code changes (only FINAL code per transaction)
            // Per EIP-7928, similar to nonce/balance, we only record the final code per tx.
            if let Some(changes) = self.code_changes.get(address) {
                // Group code changes by transaction index, keeping only the final one
                let mut changes_by_tx: BTreeMap<u16, Bytes> = BTreeMap::new();
                for (index, new_code) in changes {
                    // Only keep the final code for each transaction (last write wins)
                    changes_by_tx.insert(*index, new_code.clone());
                }

                for (index, new_code) in changes_by_tx {
                    account_changes.add_code_change(CodeChange::new(index, new_code));
                }
            }

            // Add account to BAL (even if empty - per EIP-7928, touched addresses must appear)
            bal.add_account_changes(account_changes);
        }

        bal
    }

    /// Returns true if the recorder has no recorded data.
    pub fn is_empty(&self) -> bool {
        self.touched_addresses.is_empty()
            && self.storage_reads.is_empty()
            && self.storage_writes.is_empty()
            && self.balance_changes.is_empty()
            && self.nonce_changes.is_empty()
            && self.code_changes.is_empty()
    }

    /// Creates a checkpoint of the current state (excluding touched_addresses which persist).
    ///
    /// Per EIP-7928: "State changes from reverted calls are discarded, but all accessed
    /// addresses must be included." The checkpoint captures state change data so it can
    /// be restored on revert, while touched_addresses are preserved.
    pub fn checkpoint(&self) -> BlockAccessListCheckpoint {
        BlockAccessListCheckpoint {
            reads_promoted_len: self
                .reads_promoted_to_writes
                .iter()
                .map(|(addr, promoted)| (*addr, promoted.len()))
                .collect(),
            storage_writes_len: self
                .storage_writes
                .iter()
                .map(|(addr, slots)| {
                    (
                        *addr,
                        slots
                            .iter()
                            .map(|(slot, changes)| (*slot, changes.len()))
                            .collect(),
                    )
                })
                .collect(),
            balance_changes_len: self
                .balance_changes
                .iter()
                .map(|(addr, changes)| (*addr, changes.len()))
                .collect(),
            nonce_changes_len: self
                .nonce_changes
                .iter()
                .map(|(addr, changes)| (*addr, changes.len()))
                .collect(),
            code_changes_len: self
                .code_changes
                .iter()
                .map(|(addr, changes)| (*addr, changes.len()))
                .collect(),
        }
    }

    /// Restores state to a checkpoint, keeping touched_addresses intact.
    ///
    /// Per EIP-7928: "State changes from reverted calls are discarded, but all accessed
    /// addresses must be included." This means:
    /// - Storage reads from reverted calls PERSIST (reads are accesses, not state changes)
    /// - Storage writes from reverted calls become READS (slot was accessed but value unchanged)
    /// - Balance/nonce/code changes are discarded
    pub fn restore(&mut self, checkpoint: BlockAccessListCheckpoint) {
        // Step 1: Truncate reads_promoted_to_writes (undo promotions after checkpoint)
        // Reads are never removed from storage_reads, so truncating promotions
        // means those slots will be treated as reads again in build().
        self.reads_promoted_to_writes.retain(|addr, promoted| {
            if let Some(&len) = checkpoint.reads_promoted_len.get(addr) {
                promoted.truncate(len);
                len > 0
            } else {
                false
            }
        });

        // Step 2: Find slots that were written after checkpoint but were NOT reads
        // (fresh writes need to become reads on revert)
        for (addr, slots) in &self.storage_writes {
            let checkpoint_lens = checkpoint.storage_writes_len.get(addr);
            for (slot, changes) in slots {
                let checkpoint_len = checkpoint_lens
                    .and_then(|m| m.get(slot))
                    .copied()
                    .unwrap_or(0);
                if changes.len() > checkpoint_len {
                    // This slot had writes after checkpoint - ensure it's recorded as a read
                    // (Reads that became writes are already in storage_reads since we don't remove them)
                    self.storage_reads.entry(*addr).or_default().insert(*slot);
                }
            }
        }

        // Step 3: Truncate storage_writes (keep only writes from before checkpoint)
        self.storage_writes.retain(|addr, slots| {
            if let Some(slot_lens) = checkpoint.storage_writes_len.get(addr) {
                slots.retain(|slot, changes| {
                    if let Some(&len) = slot_lens.get(slot) {
                        changes.truncate(len);
                        len > 0
                    } else {
                        false
                    }
                });
                !slots.is_empty()
            } else {
                false
            }
        });

        // Restore balance_changes: truncate change vectors
        self.balance_changes.retain(|addr, changes| {
            if let Some(&len) = checkpoint.balance_changes_len.get(addr) {
                changes.truncate(len);
                len > 0
            } else {
                false
            }
        });

        // Restore nonce_changes: truncate change vectors
        self.nonce_changes.retain(|addr, changes| {
            if let Some(&len) = checkpoint.nonce_changes_len.get(addr) {
                changes.truncate(len);
                len > 0
            } else {
                false
            }
        });

        // Restore code_changes: truncate change vectors
        self.code_changes.retain(|addr, changes| {
            if let Some(&len) = checkpoint.code_changes_len.get(addr) {
                changes.truncate(len);
                len > 0
            } else {
                false
            }
        });

        // Note: touched_addresses is intentionally NOT restored - per EIP-7928,
        // accessed addresses must be included even from reverted calls
    }

    /// Handles BAL cleanup for a self-destructed account per EIP-7928/EIP-6780.
    /// Called after destroy_account for contracts created and destroyed in the same tx.
    /// Removes nonce/code changes, converts storage writes to reads.
    /// Matches EELS `track_selfdestruct` in state_tracker.py:315.
    pub fn track_selfdestruct(&mut self, address: Address) {
        let idx = self.current_index;

        // 1. Remove nonce changes for this address at current tx index
        if let Some(changes) = self.nonce_changes.get_mut(&address) {
            changes.retain(|(i, _)| *i != idx);
            if changes.is_empty() {
                self.nonce_changes.remove(&address);
            }
        }

        // 2. Remove balance changes if pre-balance was 0 (round-trip: 0→X→0)
        // If initial_balance was never set, treat it as 0 (contract created with no value)
        let pre_balance = self
            .initial_balances
            .get(&address)
            .copied()
            .unwrap_or_default();
        if pre_balance.is_zero()
            && let Some(changes) = self.balance_changes.get_mut(&address)
        {
            changes.retain(|(i, _)| *i != idx);
            if changes.is_empty() {
                self.balance_changes.remove(&address);
            }
        }

        // 3. Remove code changes for this address at current tx index
        if let Some(changes) = self.code_changes.get_mut(&address) {
            changes.retain(|(i, _)| *i != idx);
            if changes.is_empty() {
                self.code_changes.remove(&address);
            }
        }

        // 4. Convert storage writes from current tx to reads
        if let Some(slots) = self.storage_writes.get_mut(&address) {
            let mut slots_to_read: Vec<U256> = Vec::new();
            for (slot, changes) in slots.iter_mut() {
                if changes.iter().any(|(i, _)| *i == idx) {
                    slots_to_read.push(*slot);
                }
                changes.retain(|(i, _)| *i != idx);
            }
            slots.retain(|_, changes| !changes.is_empty());
            if slots.is_empty() {
                self.storage_writes.remove(&address);
            }

            for slot in slots_to_read {
                self.storage_reads.entry(address).or_default().insert(slot);
                // Undo read-to-write promotion for these slots
                if let Some(promoted) = self.reads_promoted_to_writes.get_mut(&address) {
                    promoted.retain(|s| *s != slot);
                    if promoted.is_empty() {
                        self.reads_promoted_to_writes.remove(&address);
                    }
                }
            }
        }
    }
}
