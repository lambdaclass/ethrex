#![allow(dead_code)]

use bytes::Bytes;
use ethereum_types::{Address, H160, H256, U256};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, structs};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::constants::EMPTY_BLOCK_ACCESS_LIST_HASH;
use crate::utils::keccak;

/// SYSTEM_ADDRESS is excluded from the BAL unless it has actual state changes.
/// 0xfffffffffffffffffffffffffffffffffffffffe
pub const SYSTEM_ADDRESS: Address = H160([
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFE,
]);

#[derive(Default, Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct StorageChange {
    /// Block access index per EIP-7928 spec (uint16).
    block_access_index: u16,
    post_value: U256,
}

impl StorageChange {
    /// Creates a new storage change with the given block access index and post value.
    pub fn new(block_access_index: u16, post_value: U256) -> Self {
        Self {
            block_access_index,
            post_value,
        }
    }

    /// Returns the block access index for this storage change.
    pub fn block_access_index(&self) -> u16 {
        self.block_access_index
    }

    /// Returns the post value for this storage change.
    pub fn post_value(&self) -> U256 {
        self.post_value
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
    slot: U256,
    slot_changes: Vec<StorageChange>,
}

impl SlotChange {
    /// Creates a new slot change for the given slot.
    pub fn new(slot: U256) -> Self {
        Self {
            slot,
            slot_changes: Vec::new(),
        }
    }

    /// Returns the slot for this slot change.
    pub fn slot(&self) -> U256 {
        self.slot
    }

    /// Adds a storage change to this slot.
    pub fn add_change(&mut self, change: StorageChange) {
        self.slot_changes.push(change);
    }

    /// Returns an iterator over the storage changes.
    pub fn changes(&self) -> &[StorageChange] {
        &self.slot_changes
    }
}

impl RLPEncode for SlotChange {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        // Check if already sorted (common case when built from BlockAccessListRecorder)
        let is_sorted = self
            .slot_changes
            .windows(2)
            .all(|w| w[0].block_access_index <= w[1].block_access_index);

        if is_sorted {
            // Encode directly without cloning
            structs::Encoder::new(buf)
                .encode_field(&self.slot)
                .encode_field(&self.slot_changes)
                .finish();
        } else {
            // Clone and sort only when necessary
            let mut slot_changes = self.slot_changes.clone();
            slot_changes.sort_by(|a, b| a.block_access_index.cmp(&b.block_access_index));
            structs::Encoder::new(buf)
                .encode_field(&self.slot)
                .encode_field(&slot_changes)
                .finish();
        }
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
    block_access_index: u16,
    post_balance: U256,
}

impl BalanceChange {
    /// Creates a new balance change with the given block access index and post balance.
    pub fn new(block_access_index: u16, post_balance: U256) -> Self {
        Self {
            block_access_index,
            post_balance,
        }
    }

    /// Returns the block access index for this balance change.
    pub fn block_access_index(&self) -> u16 {
        self.block_access_index
    }

    /// Returns the post balance for this balance change.
    pub fn post_balance(&self) -> U256 {
        self.post_balance
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
    block_access_index: u16,
    post_nonce: u64,
}

impl NonceChange {
    /// Creates a new nonce change with the given block access index and post nonce.
    pub fn new(block_access_index: u16, post_nonce: u64) -> Self {
        Self {
            block_access_index,
            post_nonce,
        }
    }

    /// Returns the block access index for this nonce change.
    pub fn block_access_index(&self) -> u16 {
        self.block_access_index
    }

    /// Returns the post nonce for this nonce change.
    pub fn post_nonce(&self) -> u64 {
        self.post_nonce
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
    block_access_index: u16,
    new_code: Bytes,
}

impl CodeChange {
    /// Creates a new code change with the given block access index and new code.
    pub fn new(block_access_index: u16, new_code: Bytes) -> Self {
        Self {
            block_access_index,
            new_code,
        }
    }

    /// Returns the block access index for this code change.
    pub fn block_access_index(&self) -> u16 {
        self.block_access_index
    }

    /// Returns the new code for this code change.
    pub fn new_code(&self) -> &Bytes {
        &self.new_code
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
    address: Address,
    storage_changes: Vec<SlotChange>,
    storage_reads: Vec<U256>,
    balance_changes: Vec<BalanceChange>,
    nonce_changes: Vec<NonceChange>,
    code_changes: Vec<CodeChange>,
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

    /// Returns the address for this account changes.
    pub fn address(&self) -> Address {
        self.address
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

    /// Returns whether this account has any changes or reads.
    pub fn is_empty(&self) -> bool {
        self.storage_changes.is_empty()
            && self.storage_reads.is_empty()
            && self.balance_changes.is_empty()
            && self.nonce_changes.is_empty()
            && self.code_changes.is_empty()
    }

    /// Returns the storage changes.
    pub fn storage_changes(&self) -> &[SlotChange] {
        &self.storage_changes
    }

    /// Returns the storage reads.
    pub fn storage_reads(&self) -> &[U256] {
        &self.storage_reads
    }

    /// Returns the balance changes.
    pub fn balance_changes(&self) -> &[BalanceChange] {
        &self.balance_changes
    }

    /// Returns the nonce changes.
    pub fn nonce_changes(&self) -> &[NonceChange] {
        &self.nonce_changes
    }

    /// Returns the code changes.
    pub fn code_changes(&self) -> &[CodeChange] {
        &self.code_changes
    }
}

impl RLPEncode for AccountChanges {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        // Check if vectors are already sorted (common case when built from BlockAccessListRecorder)
        let storage_changes_sorted = self
            .storage_changes
            .windows(2)
            .all(|w| w[0].slot <= w[1].slot);
        let storage_reads_sorted = self.storage_reads.windows(2).all(|w| w[0] <= w[1]);
        let balance_changes_sorted = self
            .balance_changes
            .windows(2)
            .all(|w| w[0].block_access_index <= w[1].block_access_index);
        let nonce_changes_sorted = self
            .nonce_changes
            .windows(2)
            .all(|w| w[0].block_access_index <= w[1].block_access_index);
        let code_changes_sorted = self
            .code_changes
            .windows(2)
            .all(|w| w[0].block_access_index <= w[1].block_access_index);

        // If all are sorted, encode directly without any cloning
        if storage_changes_sorted
            && storage_reads_sorted
            && balance_changes_sorted
            && nonce_changes_sorted
            && code_changes_sorted
        {
            structs::Encoder::new(buf)
                .encode_field(&self.address)
                .encode_field(&self.storage_changes)
                .encode_field(&self.storage_reads)
                .encode_field(&self.balance_changes)
                .encode_field(&self.nonce_changes)
                .encode_field(&self.code_changes)
                .finish();
            return;
        }

        // Clone and sort only the vectors that need it
        let mut storage_changes = self.storage_changes.clone();
        if !storage_changes_sorted {
            storage_changes.sort_by(|a, b| a.slot.cmp(&b.slot));
        }

        let mut storage_reads = self.storage_reads.clone();
        if !storage_reads_sorted {
            storage_reads.sort();
        }

        let mut balance_changes = self.balance_changes.clone();
        if !balance_changes_sorted {
            balance_changes.sort_by(|a, b| a.block_access_index.cmp(&b.block_access_index));
        }

        let mut nonce_changes = self.nonce_changes.clone();
        if !nonce_changes_sorted {
            nonce_changes.sort_by(|a, b| a.block_access_index.cmp(&b.block_access_index));
        }

        let mut code_changes = self.code_changes.clone();
        if !code_changes_sorted {
            code_changes.sort_by(|a, b| a.block_access_index.cmp(&b.block_access_index));
        }

        structs::Encoder::new(buf)
            .encode_field(&self.address)
            .encode_field(&storage_changes)
            .encode_field(&storage_reads)
            .encode_field(&balance_changes)
            .encode_field(&nonce_changes)
            .encode_field(&code_changes)
            .finish();
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

    /// Computes the hash of the block access list.
    pub fn compute_hash(&self) -> H256 {
        if self.inner.is_empty() {
            return *EMPTY_BLOCK_ACCESS_LIST_HASH;
        }

        let buf = self.encode_to_vec();
        keccak(buf)
    }
}

impl RLPEncode for BlockAccessList {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        // Check if already sorted by address (common case when built from BlockAccessListRecorder)
        let is_sorted = self.inner.windows(2).all(|w| w[0].address <= w[1].address);

        if is_sorted {
            // Encode directly without cloning
            self.inner.encode(buf);
        } else {
            // Clone and sort only when necessary
            let mut sorted = self.inner.clone();
            sorted.sort_by(|a, b| a.address.cmp(&b.address));
            sorted.encode(buf);
        }
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
    /// Snapshot of storage reads at checkpoint time.
    /// We need to store the actual slots because when a write is reverted, it must
    /// be converted back to a read if it was originally a read.
    storage_reads_snapshot: BTreeMap<Address, BTreeSet<U256>>,
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
    /// Initial balances for detecting balance round-trips (per-block, used for general reference).
    initial_balances: BTreeMap<Address, U256>,
    /// Per-transaction initial balances for round-trip detection.
    /// Per EIP-7928: "If an account's balance changes during a transaction, but its
    /// post-transaction balance is equal to its pre-transaction balance, then the
    /// change MUST NOT be recorded."
    tx_initial_balances: BTreeMap<Address, U256>,
    /// Balance changes per address (list of (index, post_balance) pairs).
    balance_changes: BTreeMap<Address, Vec<(u16, U256)>>,
    /// Nonce changes per address (list of (index, post_nonce) pairs).
    nonce_changes: BTreeMap<Address, Vec<(u16, u64)>>,
    /// Code changes per address (list of (index, new_code) pairs).
    code_changes: BTreeMap<Address, Vec<(u16, Bytes)>>,
}

impl BlockAccessListRecorder {
    /// Creates a new empty recorder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the current block access index per EIP-7928 spec (uint16).
    /// Call this before each transaction (index 1..n) and for withdrawals (n+1).
    ///
    /// Per EIP-7928: "If an account's balance changes during a transaction, but its
    /// post-transaction balance is equal to its pre-transaction balance, then the
    /// change MUST NOT be recorded."
    /// Clears per-transaction initial balances when switching to a new transaction.
    pub fn set_block_access_index(&mut self, index: u16) {
        // Clear per-transaction initial balances when switching transactions
        // This enables per-transaction round-trip detection as required by EIP-7928
        if self.current_index != index {
            self.tx_initial_balances.clear();
        }
        self.current_index = index;
    }

    /// Returns the current block access index per EIP-7928 spec (uint16).
    pub fn current_index(&self) -> u16 {
        self.current_index
    }

    /// Records an address as touched during execution.
    /// The address will appear in the BAL even if it has no state changes.
    ///
    /// Note: SYSTEM_ADDRESS is excluded unless it has actual state changes.
    pub fn record_touched_address(&mut self, address: Address) {
        // SYSTEM_ADDRESS is only included if it has actual state changes
        if address != SYSTEM_ADDRESS {
            self.touched_addresses.insert(address);
        }
    }

    /// Records multiple addresses as touched during execution.
    /// More efficient than calling `record_touched_address` in a loop.
    ///
    /// Note: SYSTEM_ADDRESS is filtered out.
    pub fn extend_touched_addresses(&mut self, addresses: impl Iterator<Item = Address>) {
        self.touched_addresses
            .extend(addresses.filter(|addr| *addr != SYSTEM_ADDRESS));
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
    /// If the slot was previously recorded as a read, it is removed from reads.
    pub fn record_storage_write(&mut self, address: Address, slot: U256, post_value: U256) {
        // Remove from reads if present (reads that become writes are writes)
        if let Some(reads) = self.storage_reads.get_mut(&address) {
            reads.remove(&slot);
        }
        // Add to writes
        self.storage_writes
            .entry(address)
            .or_default()
            .entry(slot)
            .or_default()
            .push((self.current_index, post_value));
        // Mark address as touched (include SYSTEM_ADDRESS for actual state changes)
        self.touched_addresses.insert(address);
    }

    /// Records a balance change.
    /// Should be called after every balance modification.
    pub fn record_balance_change(&mut self, address: Address, post_balance: U256) {
        // Track initial balance for round-trip detection
        self.initial_balances.entry(address).or_insert(post_balance);

        self.balance_changes
            .entry(address)
            .or_default()
            .push((self.current_index, post_balance));

        // Mark address as touched (include SYSTEM_ADDRESS for actual state changes)
        self.touched_addresses.insert(address);
    }

    /// Sets the initial balance for an address before any changes.
    /// This should be called when first accessing an account to enable round-trip detection.
    ///
    /// Tracks both per-block initial (for general reference) and per-transaction initial
    /// (for EIP-7928 round-trip detection).
    pub fn set_initial_balance(&mut self, address: Address, balance: U256) {
        // Track per-block initial (for overall reference)
        self.initial_balances.entry(address).or_insert(balance);
        // Track per-transaction initial (for EIP-7928 round-trip detection)
        self.tx_initial_balances.entry(address).or_insert(balance);
    }

    /// Records a nonce change.
    /// Per EIP-7928, only record nonces for:
    /// - EOA senders
    /// - Contracts performing CREATE/CREATE2
    /// - Deployed contracts
    /// - EIP-7702 authorities
    pub fn record_nonce_change(&mut self, address: Address, post_nonce: u64) {
        self.nonce_changes
            .entry(address)
            .or_default()
            .push((self.current_index, post_nonce));
        // Mark address as touched (include SYSTEM_ADDRESS for actual state changes)
        self.touched_addresses.insert(address);
    }

    /// Records a code change (contract deployment or EIP-7702 delegation).
    pub fn record_code_change(&mut self, address: Address, new_code: Bytes) {
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
    /// 1. Filters out balance changes per-transaction where the final balance equals the initial balance
    /// 2. Creates AccountChanges entries for all touched addresses
    /// 3. Includes addresses even if they have no state changes (per EIP-7928)
    ///
    /// Per EIP-7928: "If an account's balance changes during a transaction, but its
    /// post-transaction balance is equal to its pre-transaction balance, then the
    /// change MUST NOT be recorded."
    pub fn build(self) -> BlockAccessList {
        let mut bal = BlockAccessList::with_capacity(self.touched_addresses.len());

        // Process all touched addresses
        for address in &self.touched_addresses {
            let mut account_changes = AccountChanges::new(*address);

            // Add storage writes (slot changes)
            if let Some(slots) = self.storage_writes.get(address) {
                for (slot, changes) in slots {
                    let mut slot_change = SlotChange::new(*slot);
                    for (index, post_value) in changes {
                        slot_change.add_change(StorageChange::new(*index, *post_value));
                    }
                    account_changes.add_storage_change(slot_change);
                }
            }

            // Add storage reads
            if let Some(reads) = self.storage_reads.get(address) {
                for slot in reads {
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
                let mut prev_balance = self.initial_balances.get(address).copied();
                for (index, tx_changes) in &changes_by_tx {
                    let initial_for_tx = prev_balance;
                    let final_for_tx = tx_changes.last().copied();

                    // Check if this transaction's balance round-tripped
                    let is_round_trip = match (initial_for_tx, final_for_tx) {
                        (Some(initial), Some(final_bal)) => initial == final_bal,
                        _ => false, // Include if we can't determine
                    };

                    // Only include changes if NOT a round-trip within this transaction
                    if !is_round_trip {
                        for post_balance in tx_changes {
                            account_changes
                                .add_balance_change(BalanceChange::new(*index, *post_balance));
                        }
                    }

                    // Update prev_balance for next transaction
                    prev_balance = final_for_tx;
                }
            }

            // Add nonce changes
            if let Some(changes) = self.nonce_changes.get(address) {
                for (index, post_nonce) in changes {
                    account_changes.add_nonce_change(NonceChange::new(*index, *post_nonce));
                }
            }

            // Add code changes
            if let Some(changes) = self.code_changes.get(address) {
                for (index, new_code) in changes {
                    account_changes.add_code_change(CodeChange::new(*index, new_code.clone()));
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
            storage_reads_snapshot: self.storage_reads.clone(),
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
    /// This truncates all state change vectors back to their lengths at checkpoint time,
    /// removing any changes made after the checkpoint was created.
    /// Storage reads are fully restored from the snapshot, which correctly handles
    /// the case where a slot that was read is later written (removed from reads)
    /// and then reverted (must reappear as a read).
    pub fn restore(&mut self, checkpoint: BlockAccessListCheckpoint) {
        // Restore storage_reads from snapshot.
        // This correctly handles reverted writes: if a slot was originally a read,
        // then written (removing it from reads), and then reverted, it will be
        // restored as a read.
        self.storage_reads = checkpoint.storage_reads_snapshot;

        // Restore storage_writes: truncate change vectors
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
}

#[cfg(test)]
mod tests {
    use ethereum_types::{H160, U256};
    use ethrex_rlp::decode::RLPDecode;
    use ethrex_rlp::encode::RLPEncode;

    use crate::types::block_access_list::{
        AccountChanges, BalanceChange, NonceChange, SlotChange, StorageChange,
    };

    use super::BlockAccessList;

    const ALICE_ADDR: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10]); //0xA
    const BOB_ADDR: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 11]); //0xB
    const CHARLIE_ADDR: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 12]); //0xC
    const CONTRACT_ADDR: H160 = H160([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 12]); //0xC

    #[test]
    fn test_encode_decode_empty_list_validation() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: ALICE_ADDR,
                ..Default::default()
            }],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(&buf);
        assert_eq!(
            &encoded_rlp,
            "dbda94000000000000000000000000000000000000000ac0c0c0c0c0"
        );

        let decoded_bal = BlockAccessList::decode(&buf).unwrap();
        assert_eq!(decoded_bal, actual_bal);
    }

    #[test]
    fn test_encode_decode_partial_validation() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: ALICE_ADDR,
                storage_reads: vec![U256::from(1), U256::from(2)],
                balance_changes: vec![BalanceChange {
                    block_access_index: 1,
                    post_balance: U256::from(100),
                }],
                nonce_changes: vec![NonceChange {
                    block_access_index: 1,
                    post_nonce: 1,
                }],
                ..Default::default()
            }],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(&buf);
        assert_eq!(
            &encoded_rlp,
            "e3e294000000000000000000000000000000000000000ac0c20102c3c20164c3c20101c0"
        );

        let decoded_bal = BlockAccessList::decode(&buf).unwrap();
        assert_eq!(decoded_bal, actual_bal);
    }

    #[test]
    fn test_storage_changes_validation() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: CONTRACT_ADDR,
                storage_changes: vec![SlotChange {
                    slot: U256::from(0x1),
                    slot_changes: vec![StorageChange {
                        block_access_index: 1,
                        post_value: U256::from(0x42),
                    }],
                }],
                ..Default::default()
            }],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(buf);
        assert_eq!(
            &encoded_rlp,
            "e1e094000000000000000000000000000000000000000cc6c501c3c20142c0c0c0c0"
        );
    }

    #[test]
    fn test_expected_addresses_auto_sorted() {
        let actual_bal = BlockAccessList {
            inner: vec![
                AccountChanges {
                    address: CHARLIE_ADDR,
                    ..Default::default()
                },
                AccountChanges {
                    address: ALICE_ADDR,
                    ..Default::default()
                },
                AccountChanges {
                    address: BOB_ADDR,
                    ..Default::default()
                },
            ],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(buf);
        assert_eq!(
            &encoded_rlp,
            "f851da94000000000000000000000000000000000000000ac0c0c0c0c0da94000000000000000000000000000000000000000bc0c0c0c0c0da94000000000000000000000000000000000000000cc0c0c0c0c0"
        );
    }

    #[test]
    fn test_expected_storage_slots_ordering_correct_order_should_pass() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: ALICE_ADDR,
                storage_changes: vec![
                    SlotChange {
                        slot: U256::from(0x02),
                        slot_changes: vec![],
                    },
                    SlotChange {
                        slot: U256::from(0x01),
                        slot_changes: vec![],
                    },
                    SlotChange {
                        slot: U256::from(0x03),
                        slot_changes: vec![],
                    },
                ],
                ..Default::default()
            }],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(&buf);
        assert_eq!(
            &encoded_rlp,
            "e4e394000000000000000000000000000000000000000ac9c201c0c202c0c203c0c0c0c0c0"
        );
    }

    #[test]
    fn test_expected_storage_reads_ordering_correct_order_should_pass() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: ALICE_ADDR,
                storage_reads: vec![U256::from(0x02), U256::from(0x01), U256::from(0x03)],
                ..Default::default()
            }],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(buf);
        assert_eq!(
            &encoded_rlp,
            "dedd94000000000000000000000000000000000000000ac0c3010203c0c0c0"
        );
    }

    #[test]
    fn test_expected_tx_indices_ordering_correct_order_should_pass() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: ALICE_ADDR,
                nonce_changes: vec![
                    NonceChange {
                        block_access_index: 2,
                        post_nonce: 2,
                    },
                    NonceChange {
                        block_access_index: 3,
                        post_nonce: 3,
                    },
                    NonceChange {
                        block_access_index: 1,
                        post_nonce: 1,
                    },
                ],
                ..Default::default()
            }],
        };

        let mut buf = Vec::new();
        actual_bal.encode(&mut buf);

        let encoded_rlp = hex::encode(buf);
        assert_eq!(
            &encoded_rlp,
            "e4e394000000000000000000000000000000000000000ac0c0c0c9c20101c20202c20303c0"
        );
    }

    #[test]
    fn test_decode_storage_slots_ordering_correct_order_should_pass() {
        let actual_bal = BlockAccessList {
            inner: vec![AccountChanges {
                address: ALICE_ADDR,
                storage_changes: vec![
                    SlotChange {
                        slot: U256::from(0x01),
                        slot_changes: vec![],
                    },
                    SlotChange {
                        slot: U256::from(0x02),
                        slot_changes: vec![],
                    },
                    SlotChange {
                        slot: U256::from(0x03),
                        slot_changes: vec![],
                    },
                ],
                ..Default::default()
            }],
        };

        let encoded_rlp: Vec<u8> = hex::decode(
            "e4e394000000000000000000000000000000000000000ac9c201c0c202c0c203c0c0c0c0c0",
        )
        .unwrap();

        let decoded_bal = BlockAccessList::decode(&encoded_rlp).unwrap();
        assert_eq!(decoded_bal, actual_bal);
    }

    // ====================== BlockAccessListRecorder Tests ======================

    use super::BlockAccessListRecorder;
    use super::SYSTEM_ADDRESS;

    #[test]
    fn test_recorder_empty_build() {
        let recorder = BlockAccessListRecorder::new();
        let bal = recorder.build();
        assert!(bal.is_empty());
    }

    #[test]
    fn test_recorder_touched_address_only() {
        let mut recorder = BlockAccessListRecorder::new();
        recorder.record_touched_address(ALICE_ADDR);
        let bal = recorder.build();

        assert_eq!(bal.accounts().len(), 1);
        let account = &bal.accounts()[0];
        assert_eq!(account.address(), ALICE_ADDR);
        // Account with no changes should still appear (per EIP-7928)
        assert!(account.storage_changes().is_empty());
        assert!(account.balance_changes().is_empty());
    }

    #[test]
    fn test_recorder_storage_read_then_write_becomes_write() {
        let mut recorder = BlockAccessListRecorder::new();
        recorder.set_block_access_index(1);

        // First read a slot
        recorder.record_storage_read(ALICE_ADDR, U256::from(0x10));
        // Then write to the same slot
        recorder.record_storage_write(ALICE_ADDR, U256::from(0x10), U256::from(0x42));

        let bal = recorder.build();

        assert_eq!(bal.accounts().len(), 1);
        let account = &bal.accounts()[0];
        // The slot should appear in writes, not reads
        assert_eq!(account.storage_changes().len(), 1);
        assert!(account.storage_reads().is_empty());
        assert_eq!(account.storage_changes()[0].slot(), U256::from(0x10));
    }

    #[test]
    fn test_recorder_storage_read_only() {
        let mut recorder = BlockAccessListRecorder::new();
        recorder.set_block_access_index(1);

        recorder.record_storage_read(ALICE_ADDR, U256::from(0x10));
        recorder.record_storage_read(ALICE_ADDR, U256::from(0x20));

        let bal = recorder.build();

        assert_eq!(bal.accounts().len(), 1);
        let account = &bal.accounts()[0];
        assert!(account.storage_changes().is_empty());
        assert_eq!(account.storage_reads().len(), 2);
    }

    #[test]
    fn test_recorder_multiple_writes_same_slot() {
        let mut recorder = BlockAccessListRecorder::new();
        recorder.set_block_access_index(1);
        recorder.record_storage_write(ALICE_ADDR, U256::from(0x10), U256::from(0x01));
        recorder.set_block_access_index(2);
        recorder.record_storage_write(ALICE_ADDR, U256::from(0x10), U256::from(0x02));

        let bal = recorder.build();

        let account = &bal.accounts()[0];
        assert_eq!(account.storage_changes().len(), 1);
        let slot_change = &account.storage_changes()[0];
        // Should have two changes with different indices
        assert_eq!(slot_change.changes().len(), 2);
    }

    #[test]
    fn test_recorder_balance_roundtrip_filtered_within_tx() {
        // Per EIP-7928: "If an account's balance changes during a transaction, but its
        // post-transaction balance is equal to its pre-transaction balance, then the
        // change MUST NOT be recorded."
        // This is per-TRANSACTION filtering, not per-block.
        let mut recorder = BlockAccessListRecorder::new();
        recorder.set_block_access_index(1);

        // Set initial balance
        recorder.set_initial_balance(ALICE_ADDR, U256::from(1000));
        // Record changes within the SAME transaction that round-trip
        recorder.record_balance_change(ALICE_ADDR, U256::from(500)); // decrease
        recorder.record_balance_change(ALICE_ADDR, U256::from(1000)); // back to initial

        let bal = recorder.build();

        let account = &bal.accounts()[0];
        // Balance round-tripped within same TX, so balance_changes should be empty
        assert!(account.balance_changes().is_empty());
    }

    #[test]
    fn test_recorder_balance_changes_across_txs_not_filtered() {
        // Per EIP-7928: Per-transaction filtering means changes across different
        // transactions are evaluated independently.
        let mut recorder = BlockAccessListRecorder::new();
        recorder.set_block_access_index(1);

        // Set initial balance for TX 1
        recorder.set_initial_balance(ALICE_ADDR, U256::from(1000));
        // TX 1: decrease to 500 (NOT round-trip: 1000 -> 500)
        recorder.record_balance_change(ALICE_ADDR, U256::from(500));

        // TX 2: increase back to 1000 (NOT round-trip: 500 -> 1000)
        recorder.set_block_access_index(2);
        recorder.record_balance_change(ALICE_ADDR, U256::from(1000));

        let bal = recorder.build();

        let account = &bal.accounts()[0];
        // Both transactions have actual balance changes (not round-trips within their tx)
        // TX 1: 1000 -> 500, TX 2: 500 -> 1000
        assert_eq!(account.balance_changes().len(), 2);
    }

    #[test]
    fn test_recorder_balance_change_recorded() {
        let mut recorder = BlockAccessListRecorder::new();
        recorder.set_block_access_index(1);

        recorder.set_initial_balance(ALICE_ADDR, U256::from(1000));
        recorder.record_balance_change(ALICE_ADDR, U256::from(500));

        let bal = recorder.build();

        let account = &bal.accounts()[0];
        // Balance changed to different value, should be recorded
        assert_eq!(account.balance_changes().len(), 1);
        assert_eq!(account.balance_changes()[0].post_balance(), U256::from(500));
    }

    #[test]
    fn test_recorder_nonce_change() {
        let mut recorder = BlockAccessListRecorder::new();
        recorder.set_block_access_index(1);

        recorder.record_nonce_change(ALICE_ADDR, 1);

        let bal = recorder.build();

        let account = &bal.accounts()[0];
        assert_eq!(account.nonce_changes().len(), 1);
        assert_eq!(account.nonce_changes()[0].post_nonce(), 1);
    }

    #[test]
    fn test_recorder_code_change() {
        let mut recorder = BlockAccessListRecorder::new();
        recorder.set_block_access_index(1);

        recorder.record_code_change(ALICE_ADDR, bytes::Bytes::from_static(&[0x60, 0x00]));

        let bal = recorder.build();

        let account = &bal.accounts()[0];
        assert_eq!(account.code_changes().len(), 1);
        assert_eq!(
            account.code_changes()[0].new_code(),
            &bytes::Bytes::from_static(&[0x60, 0x00])
        );
    }

    #[test]
    fn test_recorder_system_address_excluded_when_only_touched() {
        let mut recorder = BlockAccessListRecorder::new();
        // Just touch SYSTEM_ADDRESS without actual state changes
        recorder.record_touched_address(SYSTEM_ADDRESS);

        let bal = recorder.build();
        // SYSTEM_ADDRESS should not appear if only touched
        assert!(bal.is_empty());
    }

    #[test]
    fn test_recorder_system_address_included_with_state_change() {
        let mut recorder = BlockAccessListRecorder::new();
        recorder.set_block_access_index(1);
        // Record an actual state change for SYSTEM_ADDRESS
        recorder.record_storage_write(SYSTEM_ADDRESS, U256::from(0x10), U256::from(0x42));

        let bal = recorder.build();
        // SYSTEM_ADDRESS should appear because it has actual state changes
        assert_eq!(bal.accounts().len(), 1);
        assert_eq!(bal.accounts()[0].address(), SYSTEM_ADDRESS);
    }

    #[test]
    fn test_recorder_multiple_addresses_sorted() {
        let mut recorder = BlockAccessListRecorder::new();
        recorder.record_touched_address(CHARLIE_ADDR);
        recorder.record_touched_address(ALICE_ADDR);
        recorder.record_touched_address(BOB_ADDR);

        let bal = recorder.build();

        // Addresses should be sorted lexicographically in the encoded output
        assert_eq!(bal.accounts().len(), 3);
        // BTreeSet maintains order, so the build() returns them in sorted order
        let addresses: Vec<_> = bal.accounts().iter().map(|a| a.address()).collect();
        // The set should be sorted
        let mut sorted = addresses.clone();
        sorted.sort();
        assert_eq!(addresses, sorted);
    }

    // ====================== EIP-7928 Execution Spec Tests ======================

    #[test]
    fn test_bal_self_transfer() {
        // Per EIP-7928: Self-transfers where an account sends value to itself
        // result in balance changes that round-trip within the same TX.
        let mut recorder = BlockAccessListRecorder::new();
        recorder.set_block_access_index(1);

        // Initial balance of 1000
        recorder.set_initial_balance(ALICE_ADDR, U256::from(1000));
        // Self-transfer: balance goes down then back up by same amount
        // (In a real self-transfer, the net effect is zero)
        recorder.record_balance_change(ALICE_ADDR, U256::from(1000)); // No net change

        let bal = recorder.build();

        let account = &bal.accounts()[0];
        // Self-transfer with no net balance change should result in empty balance_changes
        assert!(account.balance_changes().is_empty());
    }

    #[test]
    fn test_bal_zero_value_transfer() {
        // Per EIP-7928: Zero-value transfers touch accounts but don't change balances.
        // Both sender and recipient must appear in BAL even with no balance changes.
        let mut recorder = BlockAccessListRecorder::new();
        recorder.set_block_access_index(1);

        // Touch both addresses (simulating a zero-value transfer)
        recorder.record_touched_address(ALICE_ADDR); // sender
        recorder.record_touched_address(BOB_ADDR); // recipient

        // Set initial balances (no actual change occurs in zero-value transfer)
        recorder.set_initial_balance(ALICE_ADDR, U256::from(1000));
        recorder.set_initial_balance(BOB_ADDR, U256::from(500));

        // Record same balances (no change)
        recorder.record_balance_change(ALICE_ADDR, U256::from(1000));
        recorder.record_balance_change(BOB_ADDR, U256::from(500));

        let bal = recorder.build();

        // Both accounts should appear (they were touched)
        assert_eq!(bal.accounts().len(), 2);
        // Neither should have balance_changes (balances unchanged)
        for account in bal.accounts() {
            assert!(account.balance_changes().is_empty());
        }
    }

    #[test]
    fn test_bal_checkpoint_restore_preserves_touched_addresses() {
        // Per EIP-7928: "State changes from reverted calls are discarded, but all
        // accessed addresses must be included."
        let mut recorder = BlockAccessListRecorder::new();
        recorder.set_block_access_index(1);

        // Record some state before checkpoint
        recorder.record_touched_address(ALICE_ADDR);
        recorder.record_storage_write(ALICE_ADDR, U256::from(0x10), U256::from(0x01));

        // Take checkpoint (simulating entering a nested call)
        let checkpoint = recorder.checkpoint();

        // Record more state that will be reverted
        recorder.record_touched_address(BOB_ADDR);
        recorder.record_storage_write(BOB_ADDR, U256::from(0x20), U256::from(0x02));

        // Revert (simulating nested call failure)
        recorder.restore(checkpoint);

        let bal = recorder.build();

        // ALICE should have her storage write preserved
        // BOB's storage write should be reverted
        // BUT both addresses should still appear (touched_addresses persists)
        assert_eq!(bal.accounts().len(), 2);

        let alice = bal
            .accounts()
            .iter()
            .find(|a| a.address() == ALICE_ADDR)
            .unwrap();
        let bob = bal
            .accounts()
            .iter()
            .find(|a| a.address() == BOB_ADDR)
            .unwrap();

        // Alice's storage write survived
        assert_eq!(alice.storage_changes().len(), 1);
        // Bob's storage write was reverted
        assert!(bob.storage_changes().is_empty());
    }

    #[test]
    fn test_bal_reverted_write_restores_read() {
        // When a slot is read, then written (which removes it from reads), then
        // the write is reverted, the slot should be restored as a read.
        let mut recorder = BlockAccessListRecorder::new();
        recorder.set_block_access_index(1);

        // Read a slot
        recorder.record_storage_read(ALICE_ADDR, U256::from(0x10));

        // Take checkpoint
        let checkpoint = recorder.checkpoint();

        // Write to the same slot (this removes it from reads and adds to writes)
        recorder.record_storage_write(ALICE_ADDR, U256::from(0x10), U256::from(0x42));

        // At this point, the slot should be in writes, not reads
        // (verified by existing test test_recorder_storage_read_then_write_becomes_write)

        // Revert the write
        recorder.restore(checkpoint);

        let bal = recorder.build();

        let account = &bal.accounts()[0];
        // The write was reverted, so slot should be back in reads
        assert_eq!(account.storage_reads().len(), 1);
        assert!(account.storage_reads().contains(&U256::from(0x10)));
        // And not in writes
        assert!(account.storage_changes().is_empty());
    }
}
