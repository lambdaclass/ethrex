use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Mutex;

use ethereum_types::Address;
use ethrex_common::H256;
use ethrex_common::types::{BlockHeader, BlockNumber};
use lru::LruCache;

use super::extra_data::VoteAttestation;
use super::seal::SealError;

/// Number of snapshots to keep in the in-memory LRU cache.
const SNAPSHOT_CACHE_SIZE: usize = 128;

/// Difficulty value for an in-turn block producer.
pub const DIFF_IN_TURN: u64 = 2;
/// Difficulty value for an out-of-turn block producer.
pub const DIFF_NO_TURN: u64 = 1;

// ── ValidatorInfo ─────────────────────────────────────────────────────────────

/// A validator entry in the BSC Parlia validator set.
///
/// Validators are sorted ascending by `address` when determining turn order,
/// matching the Go implementation's `validatorsAscending` sort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorInfo {
    pub address: Address,
    /// 48-byte BLS public key used for fast-finality vote attestation.
    pub bls_public_key: [u8; 48],
}

// ── Snapshot ──────────────────────────────────────────────────────────────────

/// Snapshot of the validator set at a specific block.
///
/// Tracks the authorised set of validators and recent signers used for
/// difficulty calculation, proposer rotation, and spam protection.
///
/// Reference: BSC `consensus/parlia/snapshot.go` `Snapshot` struct.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Block number this snapshot was created at.
    pub number: BlockNumber,
    /// Block hash this snapshot was created from.
    pub hash: H256,
    /// Current authorised validator set, sorted ascending by address.
    pub validators: Vec<ValidatorInfo>,
    /// Recent block signers keyed by block number.  Used to prevent a
    /// validator from signing more than `turn_length` times within the
    /// recent-history window.
    pub recents: BTreeMap<BlockNumber, Address>,
    /// Number of consecutive blocks each validator is permitted to produce
    /// in a single turn.  Defaults to 1; updated at epoch boundaries.
    pub turn_length: u8,
    /// Current epoch length (blocks per validator-election cycle).
    pub epoch_length: u64,
    /// Target block interval in **milliseconds**.
    pub block_interval: u64,
    /// Latest fast-finality attestation observed, if any.
    pub attestation: Option<VoteAttestation>,
}

// ── Error types ───────────────────────────────────────────────────────────────

/// Error types for snapshot operations.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("seal error: {0}")]
    Seal(#[from] SealError),
    #[error("empty validator set at block {0}")]
    EmptyValidatorSet(BlockNumber),
    #[error("no snapshot found for block {0}")]
    NotFound(BlockNumber),
    #[error("signer {0:?} is not in the current validator set")]
    UnauthorizedSigner(Address),
    #[error("signer {0:?} has signed too recently")]
    RecentlySigned(Address),
    #[error("headers are not contiguous or do not follow this snapshot")]
    OutOfRange,
}

// ── Snapshot implementation ───────────────────────────────────────────────────

impl Snapshot {
    /// Create a new snapshot at the genesis or a trusted checkpoint.
    pub fn new(
        number: BlockNumber,
        hash: H256,
        validators: Vec<ValidatorInfo>,
        turn_length: u8,
        epoch_length: u64,
        block_interval: u64,
    ) -> Self {
        Self {
            number,
            hash,
            validators,
            recents: BTreeMap::new(),
            turn_length,
            epoch_length,
            block_interval,
            attestation: None,
        }
    }

    // ── Validator ordering ────────────────────────────────────────────────────

    /// Returns addresses sorted ascending — the canonical order for turn
    /// calculation.
    pub fn sorted_addresses(&self) -> Vec<Address> {
        let mut addrs: Vec<Address> = self.validators.iter().map(|v| v.address).collect();
        addrs.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        addrs
    }

    /// Returns the validator that is in-turn for the block *after* this
    /// snapshot's block number (i.e. for block `number + 1`).
    ///
    /// Formula: `validators[(number + 1) / turn_length % len(validators)]`
    ///
    /// Reference: BSC `snapshot.go` `inturnValidator`.
    pub fn inturn_validator(&self) -> Address {
        let validators = self.sorted_addresses();
        let n = validators.len() as u64;
        debug_assert!(n > 0, "validator set must be non-empty");
        let offset = (self.number + 1) / (self.turn_length as u64) % n;
        validators[offset as usize]
    }

    // ── History window helpers ─────────────────────────────────────────────────

    /// Number of recent blocks to check for miner history spam protection.
    ///
    /// Formula: `(num_validators / 2 + 1) * turn_length - 1`
    ///
    /// Reference: BSC `snapshot.go` `minerHistoryCheckLen`.
    pub fn miner_history_check_len(&self) -> u64 {
        let n = self.validators.len() as u64;
        (n / 2 + 1) * (self.turn_length as u64) - 1
    }

    /// Number of recent blocks used for fork-hash version tracking.
    ///
    /// Formula: `num_validators * turn_length`
    pub fn version_history_check_len(&self) -> u64 {
        self.validators.len() as u64 * self.turn_length as u64
    }

    /// Count how many times each address appears as a signer within the
    /// active history window `(number - miner_history_check_len, number]`.
    fn count_recents(&self) -> std::collections::HashMap<Address, u8> {
        let check_len = self.miner_history_check_len();
        let left_bound = self.number.saturating_sub(check_len);

        let mut counts: std::collections::HashMap<Address, u8> =
            std::collections::HashMap::with_capacity(self.validators.len());

        for (&block_num, &signer) in &self.recents {
            // Entries with a zero address are epoch sentinel values
            // (epochKey pattern from the Go implementation); skip those.
            if block_num <= left_bound || signer == Address::zero() {
                continue;
            }
            *counts.entry(signer).or_insert(0) += 1;
        }
        counts
    }

    /// Returns `true` if `validator` has signed `>= turn_length` times in the
    /// recent history window.
    ///
    /// Reference: BSC `snapshot.go` `signRecentlyByCounts` / `SignRecently`.
    pub fn sign_recently_by_counts(
        &self,
        validator: Address,
        counts: &std::collections::HashMap<Address, u8>,
    ) -> bool {
        counts.get(&validator).copied().unwrap_or(0) >= self.turn_length
    }

    /// Convenience wrapper: computes recents counts and checks `validator`.
    pub fn sign_recently(&self, validator: Address) -> bool {
        self.sign_recently_by_counts(validator, &self.count_recents())
    }

    // ── Difficulty ────────────────────────────────────────────────────────────

    /// Expected difficulty for a block produced by `signer`.
    ///
    /// Returns `DIFF_IN_TURN` (2) if signer equals the in-turn validator,
    /// otherwise `DIFF_NO_TURN` (1).
    pub fn difficulty_for(&self, signer: Address) -> u64 {
        if signer == self.inturn_validator() {
            DIFF_IN_TURN
        } else {
            DIFF_NO_TURN
        }
    }

    // ── Attestation ───────────────────────────────────────────────────────────

    /// Returns the highest justified (finalised source) block number known to
    /// this snapshot, or 0 if no attestation has been recorded.
    pub fn finalized_number(&self) -> u64 {
        self.attestation
            .as_ref()
            .map(|a| a.data.source_number)
            .unwrap_or(0)
    }

    /// Update the snapshot's attestation from the new block header.
    ///
    /// If the attestation spans more than one block (source + 1 != target)
    /// and we already have an attestation, only advance the target pointer —
    /// but only if the new attestation's source equals the existing target
    /// (i.e. the chain is contiguous).  This matches the Go implementation's
    /// partial-update path in `snapshot.go` `updateAttestation`.
    ///
    /// Passing `None` is a no-op.
    pub fn update_attestation(&mut self, attestation: Option<VoteAttestation>) {
        let Some(attest) = attestation else { return };

        if attest.data.source_number + 1 != attest.data.target_number {
            // Partial update: advance target on existing attestation only when
            // the new attestation's source chains from the existing target.
            if let Some(ref mut existing) = self.attestation
                && attest.data.source_number == existing.data.target_number
                && attest.data.source_hash == existing.data.target_hash
            {
                existing.data.target_number = attest.data.target_number;
                existing.data.target_hash = attest.data.target_hash;
                // else: new attestation doesn't chain from existing — ignore.
            }
            return;
        }
        self.attestation = Some(attest);
    }

    // ── apply_header ──────────────────────────────────────────────────────────

    /// Advance the snapshot by one header.
    ///
    /// Steps:
    /// 1. Prune the oldest recents entry so the signer can sign again later.
    /// 2. Verify the signer is authorised and hasn't signed recently.
    /// 3. Record the signer for this block.
    /// 4. Update the attestation.
    ///
    /// Epoch-boundary validator set rotation and Maxwell-era finality pruning
    /// require chain access and are handled in the engine layer.
    ///
    /// Reference: BSC `snapshot.go` `apply` inner loop.
    pub fn apply_header(
        &mut self,
        header: &BlockHeader,
        signer: Address,
        attestation: Option<VoteAttestation>,
    ) -> Result<(), SnapshotError> {
        let number = header.number;

        // Prune old recents entry so the signer can sign again in the future.
        let limit = self.miner_history_check_len() + 1;
        if number >= limit {
            self.recents.remove(&(number - limit));
        }

        // Check the signer is in the current validator set.
        if !self.validators.iter().any(|v| v.address == signer) {
            return Err(SnapshotError::UnauthorizedSigner(signer));
        }

        // Spam protection: check the signer hasn't signed too recently.
        if self.sign_recently(signer) {
            return Err(SnapshotError::RecentlySigned(signer));
        }

        // Record this signer.
        self.recents.insert(number, signer);

        // Update fast-finality attestation.
        self.update_attestation(attestation);

        Ok(())
    }
}

// ── SnapshotCache ─────────────────────────────────────────────────────────────

/// Thread-safe LRU cache for Parlia validator snapshots.
pub struct SnapshotCache {
    inner: Mutex<LruCache<H256, Snapshot>>,
}

impl SnapshotCache {
    /// Creates a new snapshot cache.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(LruCache::new(
                NonZeroUsize::new(SNAPSHOT_CACHE_SIZE).expect("non-zero cache size"),
            )),
        }
    }

    /// Insert a snapshot into the cache.
    pub fn insert(&self, hash: H256, snapshot: Snapshot) {
        self.inner
            .lock()
            .expect("snapshot cache lock")
            .put(hash, snapshot);
    }

    /// Retrieve a snapshot from the cache by block hash.
    pub fn get(&self, hash: &H256) -> Option<Snapshot> {
        self.inner
            .lock()
            .expect("snapshot cache lock")
            .get(hash)
            .cloned()
    }
}

impl Default for SnapshotCache {
    fn default() -> Self {
        Self::new()
    }
}

// ── Validator parsing helpers ─────────────────────────────────────────────────

/// Parse the initial validator set from an epoch block's `extra_data` using
/// the post-Luban format: `[count: 1 byte][N * (20-byte addr + 48-byte BLS)]`.
pub fn parse_validators_luban(extra: &[u8]) -> Result<Vec<ValidatorInfo>, SnapshotError> {
    use super::extra_data::{ExtraDataError, parse_validators};

    parse_validators(extra, false)
        .map_err(|e| match e {
            ExtraDataError::TooShort(_, _)
            | ExtraDataError::InvalidValidatorCount(_)
            | ExtraDataError::MissingTurnLength
            | ExtraDataError::Rlp(_) => SnapshotError::EmptyValidatorSet(0),
        })
        .and_then(|pairs| {
            if pairs.is_empty() {
                return Err(SnapshotError::EmptyValidatorSet(0));
            }
            Ok(pairs
                .into_iter()
                .map(|(addr, bls)| ValidatorInfo {
                    address: addr,
                    bls_public_key: bls,
                })
                .collect())
        })
}

/// Parse validators from a pre-Luban epoch extra-data field.
///
/// Pre-Luban format: `vanity[32] + N*address[20] + seal[65]`.
/// BLS keys are not present; returned `ValidatorInfo` entries have
/// `bls_public_key` zero-filled.
pub fn parse_validators_pre_luban(extra: &[u8]) -> Result<Vec<ValidatorInfo>, SnapshotError> {
    use super::extra_data::{EXTRA_SEAL_LENGTH, EXTRA_VANITY_LENGTH};

    if extra.len() <= EXTRA_VANITY_LENGTH + EXTRA_SEAL_LENGTH {
        return Err(SnapshotError::EmptyValidatorSet(0));
    }

    let body = &extra[EXTRA_VANITY_LENGTH..extra.len() - EXTRA_SEAL_LENGTH];

    const ADDR_LEN: usize = 20;
    if !body.len().is_multiple_of(ADDR_LEN) || body.is_empty() {
        return Err(SnapshotError::EmptyValidatorSet(0));
    }

    let validators = body
        .chunks_exact(ADDR_LEN)
        .map(|chunk| {
            let mut addr = [0u8; 20];
            addr.copy_from_slice(chunk);
            ValidatorInfo {
                address: Address::from(addr),
                bls_public_key: [0u8; 48],
            }
        })
        .collect();

    Ok(validators)
}

// ── bootstrap_snapshot ───────────────────────────────────────────────────────

/// Walk backwards through `headers` to find the most recent epoch boundary,
/// parse the validator set from its `extra_data`, and return an initial
/// [`Snapshot`].
///
/// `headers` must be in ascending order (oldest first) and include at least
/// one epoch block (a block whose `number % epoch_length == 0`).
///
/// The `post_luban` flag selects the validator parsing format:
/// - `true`  → post-Luban format (count prefix + BLS keys)
/// - `false` → pre-Luban format (packed 20-byte addresses only)
///
/// `chain_id` is required for signer recovery on post-epoch headers.
///
/// Reference: BSC `parlia.go` `snapshot` genesis / checkpoint branch.
pub fn bootstrap_snapshot(
    headers: &[BlockHeader],
    epoch_length: u64,
    turn_length: u8,
    block_interval: u64,
    post_luban: bool,
    chain_id: u64,
) -> Result<Snapshot, SnapshotError> {
    // Find the latest epoch-boundary header in the slice.
    let epoch_header = headers
        .iter()
        .rev()
        .find(|h| h.number % epoch_length == 0)
        .ok_or(SnapshotError::NotFound(0))?;

    let validators = if post_luban {
        parse_validators_luban(&epoch_header.extra_data)?
    } else {
        parse_validators_pre_luban(&epoch_header.extra_data)?
    };

    let epoch_hash = epoch_header.hash.get().copied().unwrap_or(H256::zero());

    let mut snap = Snapshot::new(
        epoch_header.number,
        epoch_hash,
        validators,
        turn_length,
        epoch_length,
        block_interval,
    );

    // Advance snap.number / snap.hash for any headers that follow the epoch
    // block and record their signers in recents so that spam-protection is
    // correct when the snapshot is first used.
    for header in headers
        .iter()
        .skip_while(|h| h.number <= epoch_header.number)
    {
        snap.number = header.number;
        snap.hash = header.hash.get().copied().unwrap_or(H256::zero());

        // Recover the signer and populate recents.
        let signer = super::seal::recover_signer(header, chain_id)?;
        snap.recents.insert(header.number, signer);
    }

    Ok(snap)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::Address;
    use ethrex_common::H256;

    fn make_validator(byte: u8) -> ValidatorInfo {
        let mut addr = [0u8; 20];
        addr[19] = byte;
        ValidatorInfo {
            address: Address::from(addr),
            bls_public_key: [0u8; 48],
        }
    }

    fn make_snapshot(num_validators: usize, turn_length: u8, number: u64) -> Snapshot {
        let validators: Vec<ValidatorInfo> =
            (1..=(num_validators as u8)).map(make_validator).collect();
        Snapshot::new(number, H256::zero(), validators, turn_length, 200, 3_000)
    }

    #[test]
    fn inturn_validator_basic() {
        // 3 validators, turn_length=1, snapshot at block 10.
        // Next block = 11, offset = 11 / 1 % 3 = 2.
        let snap = make_snapshot(3, 1, 10);
        let addrs = snap.sorted_addresses();
        assert_eq!(snap.inturn_validator(), addrs[11 % 3]);
    }

    #[test]
    fn inturn_validator_turn_length_2() {
        // turn_length=2, snapshot at block 10.
        // offset = (10+1) / 2 % 3 = 5 % 3 = 2
        let snap = make_snapshot(3, 2, 10);
        let addrs = snap.sorted_addresses();
        assert_eq!(snap.inturn_validator(), addrs[(11 / 2) % 3]);
    }

    #[test]
    fn miner_history_check_len_values() {
        // 3 validators, turn_length=1: (3/2 + 1) * 1 - 1 = 1
        let snap = make_snapshot(3, 1, 10);
        assert_eq!(snap.miner_history_check_len(), 1);

        // 5 validators, turn_length=2: (5/2 + 1) * 2 - 1 = 3*2 - 1 = 5
        let snap = make_snapshot(5, 2, 10);
        assert_eq!(snap.miner_history_check_len(), 5);
    }

    #[test]
    fn difficulty_for_inturn_and_out_of_turn() {
        let snap = make_snapshot(3, 1, 10);
        let inturn = snap.inturn_validator();
        assert_eq!(snap.difficulty_for(inturn), DIFF_IN_TURN);

        let other = snap
            .sorted_addresses()
            .into_iter()
            .find(|&a| a != inturn)
            .unwrap();
        assert_eq!(snap.difficulty_for(other), DIFF_NO_TURN);
    }

    #[test]
    fn sign_recently_detects_overuse() {
        let mut snap = make_snapshot(3, 2, 10);
        let signer = snap.sorted_addresses()[0];

        // history check len = (3/2+1)*2 - 1 = 3, left_bound = 10-3 = 7
        // Both 9 and 10 are > 7, count = 2 >= turn_length 2.
        snap.recents.insert(9, signer);
        snap.recents.insert(10, signer);

        assert!(snap.sign_recently(signer));
    }

    #[test]
    fn sign_recently_below_threshold() {
        let mut snap = make_snapshot(3, 2, 10);
        let signer = snap.sorted_addresses()[0];

        // Only one entry: count = 1 < turn_length 2.
        snap.recents.insert(10, signer);

        assert!(!snap.sign_recently(signer));
    }

    #[test]
    fn apply_header_rejects_unknown_signer() {
        let mut snap = make_snapshot(3, 1, 10);
        let mut header = BlockHeader::default();
        header.number = 11;
        let unknown_addr = Address::from([0xde; 20]);
        let result = snap.apply_header(&header, unknown_addr, None);
        assert!(matches!(result, Err(SnapshotError::UnauthorizedSigner(_))));
    }

    #[test]
    fn apply_header_accepts_valid_signer() {
        let mut snap = make_snapshot(3, 1, 10);
        let mut header = BlockHeader::default();
        header.number = 11;
        let signer = snap.validators[0].address;
        let result = snap.apply_header(&header, signer, None);
        assert!(result.is_ok());
        assert_eq!(snap.recents[&11], signer);
    }

    #[test]
    fn finalized_number_without_attestation() {
        let snap = make_snapshot(3, 1, 10);
        assert_eq!(snap.finalized_number(), 0);
    }

    #[test]
    fn parse_validators_pre_luban_roundtrip() {
        // 32-byte vanity + 2*20-byte addrs + 65-byte seal
        let mut extra = vec![0u8; 32 + 40 + 65];
        extra[32..52].copy_from_slice(&[0x01; 20]);
        extra[52..72].copy_from_slice(&[0x02; 20]);
        let validators = parse_validators_pre_luban(&extra).unwrap();
        assert_eq!(validators.len(), 2);
        assert_eq!(validators[0].address, Address::from([0x01; 20]));
        assert_eq!(validators[1].address, Address::from([0x02; 20]));
    }
}
