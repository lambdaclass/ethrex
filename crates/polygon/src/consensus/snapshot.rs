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
/// Proposer priority is used by Bor's Tendermint-based proposer rotation algorithm
/// (see `Snapshot::increment_proposer_priority`).
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

    /// Sum of all validators' voting power (saturating).
    pub fn total_voting_power(&self) -> i64 {
        self.validator_set
            .iter()
            .fold(0i64, |acc, v| acc.saturating_add(v.voting_power as i64))
    }

    /// Tendermint-based proposer rotation: rescale, center, increment, select, reduce.
    ///
    /// Called at sprint boundaries to advance the proposer schedule.
    /// `times` is typically 1 (one rotation per sprint boundary).
    pub fn increment_proposer_priority(&mut self, times: u32) {
        if self.validator_set.is_empty() {
            return;
        }
        let total_vp = self.total_voting_power();
        if total_vp == 0 {
            return;
        }

        for _ in 0..times {
            // 1. Rescale if priorities have diverged too far.
            self.rescale_priorities(total_vp);

            // 2. Center priorities around zero.
            self.center_priorities();

            // 3. Increment each validator's priority by their voting power.
            for v in &mut self.validator_set {
                v.proposer_priority = v.proposer_priority.saturating_add(v.voting_power as i64);
            }

            // 4. Select proposer (highest priority, smallest address on tie).
            if let Some(proposer_idx) = self.get_proposer_index() {
                // 5. Reduce proposer's priority by total voting power.
                self.validator_set[proposer_idx].proposer_priority = self.validator_set
                    [proposer_idx]
                    .proposer_priority
                    .saturating_sub(total_vp);
            }
        }
    }

    /// Rescale all priorities if the spread exceeds 2 * total_voting_power.
    fn rescale_priorities(&mut self, total_vp: i64) {
        let (min_p, max_p) = self
            .validator_set
            .iter()
            .fold((i64::MAX, i64::MIN), |(min, max), v| {
                (min.min(v.proposer_priority), max.max(v.proposer_priority))
            });
        let spread = max_p.saturating_sub(min_p);
        let threshold = (total_vp).saturating_mul(2);

        if spread > threshold {
            // Integer ceil division: ceil(spread / threshold)
            let divisor = (spread + threshold - 1) / threshold;
            for v in &mut self.validator_set {
                v.proposer_priority /= divisor;
            }
        }
    }

    /// Center priorities by subtracting the average from all validators.
    fn center_priorities(&mut self) {
        if self.validator_set.is_empty() {
            return;
        }
        let n = self.validator_set.len() as i128;
        let sum: i128 = self
            .validator_set
            .iter()
            .map(|v| v.proposer_priority as i128)
            .sum();
        let avg = (sum / n) as i64;
        for v in &mut self.validator_set {
            v.proposer_priority = v.proposer_priority.saturating_sub(avg);
        }
    }

    /// Returns the index of the current proposer (the validator with the
    /// highest proposer_priority). On ties, the validator with the smallest
    /// address wins, matching Bor/Tendermint behavior.
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
                || (v.proposer_priority == best.proposer_priority && v.address < best.address)
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
        // Same priority — smallest address wins (Bor/Tendermint tie-break)
        let validators = vec![
            make_validator_with_priority(1, 10, 10), // smallest address
            make_validator_with_priority(3, 10, 10),
            make_validator_with_priority(2, 10, 10),
        ];
        let snap = Snapshot::new(0, H256::zero(), validators);
        assert_eq!(snap.get_proposer_index(), Some(0)); // addr 1 is smallest
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

    // ---- Proposer rotation tests ----

    #[test]
    fn total_voting_power() {
        let validators = vec![
            make_validator(1, 10),
            make_validator(2, 20),
            make_validator(3, 30),
        ];
        let snap = Snapshot::new(0, H256::zero(), validators);
        assert_eq!(snap.total_voting_power(), 60);
    }

    #[test]
    fn total_voting_power_empty() {
        let snap = Snapshot::new(0, H256::zero(), vec![]);
        assert_eq!(snap.total_voting_power(), 0);
    }

    #[test]
    fn increment_proposer_priority_single_round() {
        // 3 validators with equal power, all starting at priority 0
        let mut snap = Snapshot::new(
            0,
            H256::zero(),
            vec![
                make_validator_with_priority(1, 10, 0),
                make_validator_with_priority(2, 10, 0),
                make_validator_with_priority(3, 10, 0),
            ],
        );

        // total_vp = 30
        // After increment: all get +10 → [10, 10, 10]
        // Proposer = index 0 (smallest address on tie)
        // After reduce: [10-30, 10, 10] = [-20, 10, 10]
        snap.increment_proposer_priority(1);

        assert_eq!(snap.validator_set[0].proposer_priority, -20);
        assert_eq!(snap.validator_set[1].proposer_priority, 10);
        assert_eq!(snap.validator_set[2].proposer_priority, 10);
    }

    #[test]
    fn increment_proposer_priority_unequal_power() {
        // Validator A has 3x more power than B
        let mut snap = Snapshot::new(
            0,
            H256::zero(),
            vec![
                make_validator_with_priority(1, 30, 0), // A
                make_validator_with_priority(2, 10, 0), // B
            ],
        );

        // total_vp = 40
        // Round 1: increment → [30, 10], proposer = A (idx 0, highest priority)
        //          reduce A: [30-40, 10] = [-10, 10]
        snap.increment_proposer_priority(1);
        assert_eq!(snap.validator_set[0].proposer_priority, -10);
        assert_eq!(snap.validator_set[1].proposer_priority, 10);

        // Round 2: center avg = (-10+10)/2 = 0, no change
        //          increment → [-10+30, 10+10] = [20, 20]
        //          proposer = idx 0 (tie, smallest address)
        //          reduce: [20-40, 20] = [-20, 20]
        snap.increment_proposer_priority(1);
        assert_eq!(snap.validator_set[0].proposer_priority, -20);
        assert_eq!(snap.validator_set[1].proposer_priority, 20);

        // Round 3: center avg = (-20+20)/2 = 0, no change
        //          increment → [-20+30, 20+10] = [10, 30]
        //          proposer = idx 1 (priority 30 > 10)
        //          reduce: [10, 30-40] = [10, -10]
        snap.increment_proposer_priority(1);
        assert_eq!(snap.validator_set[0].proposer_priority, 10);
        assert_eq!(snap.validator_set[1].proposer_priority, -10);
    }

    #[test]
    fn increment_proposer_priority_empty_set_noop() {
        let mut snap = Snapshot::new(0, H256::zero(), vec![]);
        snap.increment_proposer_priority(1); // should not panic
    }

    #[test]
    fn increment_proposer_priority_multiple_times() {
        let mut snap = Snapshot::new(
            0,
            H256::zero(),
            vec![
                make_validator_with_priority(1, 10, 0),
                make_validator_with_priority(2, 10, 0),
            ],
        );

        // Run 2 rounds at once
        snap.increment_proposer_priority(2);

        // Same as calling it twice individually
        let mut snap2 = Snapshot::new(
            0,
            H256::zero(),
            vec![
                make_validator_with_priority(1, 10, 0),
                make_validator_with_priority(2, 10, 0),
            ],
        );
        snap2.increment_proposer_priority(1);
        snap2.increment_proposer_priority(1);

        for (a, b) in snap.validator_set.iter().zip(snap2.validator_set.iter()) {
            assert_eq!(a.proposer_priority, b.proposer_priority);
        }
    }

    #[test]
    fn rescale_prevents_priority_explosion() {
        // Set up extreme priorities that exceed 2 * total_vp
        let mut snap = Snapshot::new(
            0,
            H256::zero(),
            vec![
                make_validator_with_priority(1, 10, 1000),
                make_validator_with_priority(2, 10, -1000),
            ],
        );

        // total_vp = 20, threshold = 40, spread = 2000
        // divisor = ceil(2000/40) = 50
        // After rescale: [1000/50, -1000/50] = [20, -20]
        // After center: avg = 0, no change
        // After increment: [20+10, -20+10] = [30, -10]
        // Proposer = idx 0 (priority 30), reduce: [30-20, -10] = [10, -10]
        snap.increment_proposer_priority(1);

        assert_eq!(snap.validator_set[0].proposer_priority, 10);
        assert_eq!(snap.validator_set[1].proposer_priority, -10);
    }

    #[test]
    fn center_adjusts_average_to_zero() {
        let mut snap = Snapshot::new(
            0,
            H256::zero(),
            vec![
                make_validator_with_priority(1, 10, 100),
                make_validator_with_priority(2, 10, 200),
            ],
        );

        snap.center_priorities();

        // avg = (100+200)/2 = 150
        assert_eq!(snap.validator_set[0].proposer_priority, -50);
        assert_eq!(snap.validator_set[1].proposer_priority, 50);
    }
}
