//! How far the snap/2 range download has advanced through the state key space.
//!
//! From devp2p `caps/snap.md`, "Synchronization algorithm":
//!
//! > The download starts at state root `R₀` of the initial pivot block and all
//! > responses are verified against `R₀`. As the pivot block advances, the
//! > current root is updated to `R₁`, ... `Rₙ` from the pivot. The state
//! > iteration does not restart when the pivot moves, i.e. it always advances
//! > the key until the end of state is reached.
//!
//! The resulting flat state therefore mixes leaves from `R₀ … Rₙ` and is
//! patched back into consistency by applying block access lists. A patch is
//! only correct for a key the download has already passed: a key still ahead of
//! the cursor will be served later, at a newer root, and already carries the
//! change the access list describes. Applying it twice — or applying it to a
//! key that is not there yet — corrupts the flat state, so every BAL write is
//! gated on these predicates.

use std::collections::{BTreeMap, BTreeSet};

use ethrex_common::{BigEndianHash, H256, U256};

/// A half-open-at-the-front range of hashes still to be served: `next` is the
/// first hash not yet covered, `last` the final hash the range owns.
///
/// A range is drained when `next` passes `last`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashRange {
    pub next: H256,
    pub last: H256,
}

impl HashRange {
    pub fn new(next: H256, last: H256) -> Self {
        Self { next, last }
    }

    /// Whether every hash this range owns has been served.
    pub fn is_drained(&self) -> bool {
        self.next > self.last
    }
}

/// Tracks which accounts, and which of their storage slots, the range download
/// has already served.
///
/// Account ranges are kept sorted by `last` and together cover the whole hash
/// space, so the first range whose `last` reaches a given hash is the one that
/// owns it.
#[derive(Debug, Default, Clone)]
pub struct DownloadCursor {
    /// Account-hash ranges, sorted by `last`, covering the unserved space.
    /// Drained ranges are removed.
    account_ranges: Vec<HashRange>,
    /// Slot ranges outstanding for a contract too large to be served in one
    /// go. An account is listed here only while its storage is partly served.
    storage_ranges: BTreeMap<H256, Vec<HashRange>>,
    /// Accounts whose storage has been served in full.
    storage_completed: BTreeSet<H256>,
}

impl DownloadCursor {
    /// A cursor with nothing served yet, split into `chunks` account ranges.
    ///
    /// Splitting only decides how the work is handed to peers; the predicates
    /// treat the ranges as one contiguous frontier either way.
    pub fn new(chunks: usize) -> Self {
        let chunks = chunks.max(1);
        let span = U256::MAX / U256::from(chunks);
        let mut account_ranges = Vec::with_capacity(chunks);
        let mut next = U256::zero();
        for i in 0..chunks {
            let last = if i + 1 == chunks {
                U256::MAX
            } else {
                next + span
            };
            account_ranges.push(HashRange::new(
                H256::from_uint(&next),
                H256::from_uint(&last),
            ));
            // The final range ends at the top of the space, where there is no
            // next hash to start from.
            let Some(following) = last.checked_add(U256::one()) else {
                break;
            };
            next = following;
        }
        Self {
            account_ranges,
            ..Default::default()
        }
    }

    /// The account ranges still to be served.
    pub fn pending_account_ranges(&self) -> &[HashRange] {
        &self.account_ranges
    }

    /// Whether the whole state key space has been served.
    pub fn is_complete(&self) -> bool {
        self.account_ranges.is_empty() && self.storage_ranges.is_empty()
    }

    /// Whether this account's leaf has already been served.
    ///
    /// An account range advances only once the accounts it covers *and* their
    /// storage and code are all in, so a true answer here also means the
    /// account's storage is complete. Anything still inside a pending range is
    /// re-requested at the current pivot, which is why a partly-served range
    /// answers false for keys it has already delivered.
    pub fn is_account_fetched(&self, account_hash: H256) -> bool {
        match self
            .account_ranges
            .iter()
            .find(|range| account_hash <= range.last)
        {
            Some(range) => account_hash < range.next,
            // Past the last pending range: the tail of the space is done.
            None => true,
        }
    }

    /// Whether this storage slot has already been served.
    pub fn is_storage_fetched(&self, account_hash: H256, slot_hash: H256) -> bool {
        let Some(range) = self
            .account_ranges
            .iter()
            .find(|range| account_hash <= range.last)
        else {
            return true;
        };
        if account_hash < range.next {
            return true;
        }
        if self.storage_completed.contains(&account_hash) {
            return true;
        }
        match self.storage_ranges.get(&account_hash) {
            // A contract being served in chunks: the slot is in whichever
            // chunk owns it, or past them all.
            Some(ranges) => match ranges.iter().find(|range| slot_hash <= range.last) {
                Some(range) => slot_hash < range.next,
                None => true,
            },
            // The account landed but its storage has not been scheduled yet.
            None => false,
        }
    }

    /// Record that every account up to and including `served_through` in the
    /// range owning it has been served, along with their storage and code.
    pub fn advance_accounts(&mut self, served_through: H256) {
        let Some(index) = self
            .account_ranges
            .iter()
            .position(|range| served_through <= range.last)
        else {
            return;
        };
        match served_through.into_uint().checked_add(U256::one()) {
            Some(next) => self.account_ranges[index].next = H256::from_uint(&next),
            // Served through the top of the hash space, which only the range
            // owning the tail can reach. There is no next hash to point at, so
            // drop the range rather than expressing the frontier past it.
            None => {
                self.account_ranges.remove(index);
            }
        }
        self.account_ranges.retain(|range| !range.is_drained());
        // Everything below the first pending range is served, so completion
        // flags for those accounts are redundant against the ranges. Mainnet
        // has millions of contracts; keeping every flag for the whole sync is
        // not affordable.
        match self.account_ranges.first() {
            Some(first) => self.storage_completed = self.storage_completed.split_off(&first.next),
            None => self.storage_completed.clear(),
        }
    }

    /// The slot ranges a contract still owes, if its storage is part-served.
    ///
    /// The download reads this to resume a large contract where it left off
    /// after a pivot move, rather than restarting its slot space.
    pub fn storage_ranges(&self, account_hash: H256) -> Option<&[HashRange]> {
        self.storage_ranges
            .get(&account_hash)
            .map(|ranges| ranges.as_slice())
    }

    /// Whether a contract's storage has been served in full.
    pub fn is_storage_complete(&self, account_hash: H256) -> bool {
        self.storage_completed.contains(&account_hash)
    }

    /// Open a set of slot ranges for a contract whose storage needs more than
    /// one request.
    pub fn open_storage_ranges(&mut self, account_hash: H256, ranges: Vec<HashRange>) {
        if ranges.is_empty() {
            self.complete_storage(account_hash);
            return;
        }
        self.storage_completed.remove(&account_hash);
        self.storage_ranges.insert(account_hash, ranges);
    }

    /// Record that a contract's slots up to and including `served_through` are
    /// in. Draining the last range completes the account's storage.
    pub fn advance_storage(&mut self, account_hash: H256, served_through: H256) {
        let Some(ranges) = self.storage_ranges.get_mut(&account_hash) else {
            return;
        };
        if let Some(index) = ranges.iter().position(|range| served_through <= range.last) {
            match served_through.into_uint().checked_add(U256::one()) {
                Some(next) => ranges[index].next = H256::from_uint(&next),
                // As in `advance_accounts`: the range owning the tail of the
                // slot space has no next hash to point at.
                None => {
                    ranges.remove(index);
                }
            }
        }
        ranges.retain(|range| !range.is_drained());
        if ranges.is_empty() {
            self.complete_storage(account_hash);
        }
    }

    /// Record that a contract's storage is served in full.
    pub fn complete_storage(&mut self, account_hash: H256) {
        self.storage_ranges.remove(&account_hash);
        self.storage_completed.insert(account_hash);
    }
}
