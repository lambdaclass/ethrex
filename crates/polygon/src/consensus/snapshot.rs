use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Mutex;

use ethereum_types::Address;
use ethrex_common::H256;
use ethrex_common::types::BlockNumber;
use lru::LruCache;

use super::seal::{SealError, recover_signer};

/// Number of snapshots to keep in the in-memory LRU cache.
const SNAPSHOT_CACHE_SIZE: usize = 128;

/// How often to persist snapshots to disk (every N blocks).
pub const SNAPSHOT_PERSIST_INTERVAL: u64 = 1024;

/// A validator entry with address, voting power, and proposer priority.
///
/// Proposer priority is used by Bor's Tendermint-based proposer rotation algorithm.
/// Full rotation logic will be implemented in Wave 3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorInfo {
    pub address: Address,
    pub voting_power: u64,
    pub proposer_priority: i64,
}

/// Snapshot of the validator set at a specific block.
///
/// Tracks the authorized set of validators and recent signers for difficulty
/// calculation and proposer rotation.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Block number this snapshot applies to.
    pub number: BlockNumber,
    /// Block hash this snapshot was created from.
    pub hash: H256,
    /// Current authorized validator set.
    pub validator_set: Vec<ValidatorInfo>,
    /// Recent block signers: block_number -> signer address.
    /// Used for difficulty calculation and proposer rotation (NOT for authorization).
    pub recents: BTreeMap<BlockNumber, Address>,
}

/// Error types for snapshot operations.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("seal error: {0}")]
    Seal(#[from] SealError),
    #[error("unauthorized signer: {0:?} is not in the validator set")]
    UnauthorizedSigner(Address),
}

impl Snapshot {
    /// Create a new snapshot at the given block.
    pub fn new(number: BlockNumber, hash: H256, validator_set: Vec<ValidatorInfo>) -> Self {
        Self {
            number,
            hash,
            validator_set,
            recents: BTreeMap::new(),
        }
    }

    /// Apply a block header to advance the snapshot.
    ///
    /// Recovers the signer from the header, validates they're in the validator set,
    /// and records the signer in recents. Prunes recents older than `sprint_size` blocks.
    ///
    /// Note: Bor does NOT reject signers found in recents — recents are used only
    /// for difficulty calculation and proposer rotation.
    pub fn apply_header(
        &mut self,
        header: &ethrex_common::types::BlockHeader,
        sprint_size: u64,
    ) -> Result<Address, SnapshotError> {
        let signer = recover_signer(header)?;

        // Verify signer is in the validator set.
        if !self.validator_set.iter().any(|v| v.address == signer) {
            return Err(SnapshotError::UnauthorizedSigner(signer));
        }

        // Record this signer.
        self.recents.insert(header.number, signer);
        self.number = header.number;
        self.hash = header.hash();

        // Prune: remove the entry that's now outside the sprint window.
        if header.number >= sprint_size {
            self.recents.remove(&(header.number - sprint_size));
        }

        Ok(signer)
    }

    /// Update the validator set (e.g., at sprint-end or span boundary).
    pub fn update_validator_set(&mut self, new_set: Vec<ValidatorInfo>) {
        self.validator_set = new_set;
    }

    /// Whether this snapshot should be persisted to disk.
    pub fn should_persist(&self) -> bool {
        self.number.is_multiple_of(SNAPSHOT_PERSIST_INTERVAL)
    }

    /// Returns the index of the current proposer (the validator with the
    /// highest proposer_priority). On ties, the validator with the higher
    /// address (lexicographic) wins, matching Bor/Tendermint behavior.
    ///
    /// Returns `None` if the validator set is empty.
    pub fn get_proposer_index(&self) -> Option<usize> {
        if self.validator_set.is_empty() {
            return None;
        }
        let mut best_idx = 0;
        for (i, v) in self.validator_set.iter().enumerate().skip(1) {
            let best = &self.validator_set[best_idx];
            if v.proposer_priority > best.proposer_priority
                || (v.proposer_priority == best.proposer_priority && v.address > best.address)
            {
                best_idx = i;
            }
        }
        Some(best_idx)
    }

    /// Returns the index of the given signer in the validator set, if present.
    pub fn get_signer_index(&self, signer: &Address) -> Option<usize> {
        self.validator_set.iter().position(|v| &v.address == signer)
    }

    /// Compute the expected difficulty for a signer based on their succession
    /// number relative to the current proposer.
    ///
    /// Formula: `difficulty = total_validators - succession`
    /// where `succession = (signer_index - proposer_index) mod total_validators`
    ///
    /// The in-turn proposer (succession=0) gets the highest difficulty (= total_validators).
    /// The validator farthest from the proposer gets difficulty = 1.
    ///
    /// Returns `None` if the signer is not in the validator set or the set is empty.
    pub fn expected_difficulty(&self, signer: &Address) -> Option<u64> {
        let total = self.validator_set.len();
        if total == 0 {
            return None;
        }
        let proposer_idx = self.get_proposer_index()?;
        let signer_idx = self.get_signer_index(signer)?;

        // Succession: distance from proposer to signer going forward in the ring
        let succession = if signer_idx >= proposer_idx {
            signer_idx - proposer_idx
        } else {
            total + signer_idx - proposer_idx
        };

        Some((total - succession) as u64)
    }
}

/// In-memory LRU cache of snapshots, keyed by block hash.
pub struct SnapshotCache {
    cache: Mutex<LruCache<H256, Snapshot>>,
}

impl SnapshotCache {
    /// Create a new snapshot cache with the default capacity.
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(SNAPSHOT_CACHE_SIZE).expect("cache size > 0"),
            )),
        }
    }

    /// Get a snapshot by block hash, if cached.
    pub fn get(&self, hash: &H256) -> Option<Snapshot> {
        self.cache.lock().unwrap().get(hash).cloned()
    }

    /// Insert a snapshot into the cache.
    pub fn insert(&self, snapshot: Snapshot) {
        self.cache.lock().unwrap().put(snapshot.hash, snapshot);
    }
}

impl Default for SnapshotCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_validator(addr_byte: u8, power: u64) -> ValidatorInfo {
        ValidatorInfo {
            address: Address::from_low_u64_be(addr_byte as u64),
            voting_power: power,
            proposer_priority: 0,
        }
    }

    #[test]
    fn snapshot_new() {
        let validators = vec![make_validator(1, 10), make_validator(2, 20)];
        let snap = Snapshot::new(100, H256::zero(), validators.clone());
        assert_eq!(snap.number, 100);
        assert_eq!(snap.validator_set, validators);
        assert!(snap.recents.is_empty());
    }

    #[test]
    fn validator_info_has_proposer_priority() {
        let v = ValidatorInfo {
            address: Address::zero(),
            voting_power: 10,
            proposer_priority: -5,
        };
        assert_eq!(v.proposer_priority, -5);
    }

    #[test]
    fn should_persist() {
        let snap = Snapshot::new(1024, H256::zero(), vec![]);
        assert!(snap.should_persist());

        let snap = Snapshot::new(1025, H256::zero(), vec![]);
        assert!(!snap.should_persist());

        let snap = Snapshot::new(0, H256::zero(), vec![]);
        assert!(snap.should_persist());
    }

    #[test]
    fn update_validator_set() {
        let mut snap = Snapshot::new(0, H256::zero(), vec![make_validator(1, 10)]);
        assert_eq!(snap.validator_set.len(), 1);

        snap.update_validator_set(vec![make_validator(2, 20), make_validator(3, 30)]);
        assert_eq!(snap.validator_set.len(), 2);
        assert_eq!(snap.validator_set[0].address, Address::from_low_u64_be(2));
    }

    #[test]
    fn cache_insert_and_get() {
        let cache = SnapshotCache::new();
        let hash = H256::from_low_u64_be(42);
        let snap = Snapshot::new(100, hash, vec![make_validator(1, 10)]);

        assert!(cache.get(&hash).is_none());
        cache.insert(snap);

        let retrieved = cache.get(&hash).expect("should find cached snapshot");
        assert_eq!(retrieved.number, 100);
        assert_eq!(retrieved.hash, hash);
    }

    #[test]
    fn cache_eviction() {
        let cache = SnapshotCache::new();

        // Fill beyond capacity.
        for i in 0..SNAPSHOT_CACHE_SIZE + 10 {
            let hash = H256::from_low_u64_be(i as u64);
            cache.insert(Snapshot::new(i as u64, hash, vec![]));
        }

        // First entries should have been evicted.
        let first_hash = H256::from_low_u64_be(0);
        assert!(cache.get(&first_hash).is_none());

        // Latest entries should still be present.
        let latest_hash = H256::from_low_u64_be((SNAPSHOT_CACHE_SIZE + 9) as u64);
        assert!(cache.get(&latest_hash).is_some());
    }

    // ---- Difficulty calculation tests ----

    fn make_validator_with_priority(addr_byte: u8, power: u64, priority: i64) -> ValidatorInfo {
        ValidatorInfo {
            address: Address::from_low_u64_be(addr_byte as u64),
            voting_power: power,
            proposer_priority: priority,
        }
    }

    #[test]
    fn proposer_index_highest_priority() {
        let validators = vec![
            make_validator_with_priority(1, 10, 5),
            make_validator_with_priority(2, 10, 20), // highest
            make_validator_with_priority(3, 10, 10),
        ];
        let snap = Snapshot::new(0, H256::zero(), validators);
        assert_eq!(snap.get_proposer_index(), Some(1));
    }

    #[test]
    fn proposer_index_tie_breaks_by_address() {
        // Same priority — higher address wins
        let validators = vec![
            make_validator_with_priority(1, 10, 10),
            make_validator_with_priority(3, 10, 10), // higher address
            make_validator_with_priority(2, 10, 10),
        ];
        let snap = Snapshot::new(0, H256::zero(), validators);
        assert_eq!(snap.get_proposer_index(), Some(1)); // addr 3 is highest
    }

    #[test]
    fn proposer_index_empty_set() {
        let snap = Snapshot::new(0, H256::zero(), vec![]);
        assert_eq!(snap.get_proposer_index(), None);
    }

    #[test]
    fn expected_difficulty_in_turn_proposer() {
        // 3 validators, proposer is index 1 (highest priority)
        let validators = vec![
            make_validator_with_priority(1, 10, 0),
            make_validator_with_priority(2, 10, 100), // proposer
            make_validator_with_priority(3, 10, 50),
        ];
        let snap = Snapshot::new(0, H256::zero(), validators);
        let proposer = Address::from_low_u64_be(2);
        // In-turn proposer: succession=0, difficulty = 3 - 0 = 3
        assert_eq!(snap.expected_difficulty(&proposer), Some(3));
    }

    #[test]
    fn expected_difficulty_succession_ring() {
        // 4 validators: [A, B, C, D] with proposer = B (index 1)
        let validators = vec![
            make_validator_with_priority(1, 10, 0),   // A, idx 0
            make_validator_with_priority(2, 10, 100), // B, idx 1 (proposer)
            make_validator_with_priority(3, 10, 50),  // C, idx 2
            make_validator_with_priority(4, 10, 25),  // D, idx 3
        ];
        let snap = Snapshot::new(0, H256::zero(), validators);

        let addr_a = Address::from_low_u64_be(1);
        let addr_b = Address::from_low_u64_be(2);
        let addr_c = Address::from_low_u64_be(3);
        let addr_d = Address::from_low_u64_be(4);

        // B is proposer (idx=1), succession=0, difficulty=4
        assert_eq!(snap.expected_difficulty(&addr_b), Some(4));
        // C: idx=2, succession=(2-1)=1, difficulty=4-1=3
        assert_eq!(snap.expected_difficulty(&addr_c), Some(3));
        // D: idx=3, succession=(3-1)=2, difficulty=4-2=2
        assert_eq!(snap.expected_difficulty(&addr_d), Some(2));
        // A: idx=0, succession=(4+0-1)=3, difficulty=4-3=1
        assert_eq!(snap.expected_difficulty(&addr_a), Some(1));
    }

    #[test]
    fn expected_difficulty_single_validator() {
        let validators = vec![make_validator_with_priority(1, 10, 0)];
        let snap = Snapshot::new(0, H256::zero(), validators);
        let addr = Address::from_low_u64_be(1);
        // Single validator: succession=0, difficulty=1
        assert_eq!(snap.expected_difficulty(&addr), Some(1));
    }

    #[test]
    fn expected_difficulty_unknown_signer() {
        let validators = vec![make_validator_with_priority(1, 10, 0)];
        let snap = Snapshot::new(0, H256::zero(), validators);
        let unknown = Address::from_low_u64_be(99);
        assert_eq!(snap.expected_difficulty(&unknown), None);
    }

    #[test]
    fn expected_difficulty_empty_set() {
        let snap = Snapshot::new(0, H256::zero(), vec![]);
        let addr = Address::from_low_u64_be(1);
        assert_eq!(snap.expected_difficulty(&addr), None);
    }
}
