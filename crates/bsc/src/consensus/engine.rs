use ethereum_types::Address;
use ethrex_common::types::{BlockHeader, BlockNumber};
use ethrex_common::{H256, U256};

use crate::parlia_config::ParliaConfig;
use crate::validation::{ParliaValidationError, validate_parlia_header};

use super::seal::{SealError, recover_signer};
use super::snapshot::{Snapshot, SnapshotCache, SnapshotError};

// ── Error types ───────────────────────────────────────────────────────────────

/// Errors from ParliaEngine operations.
#[derive(Debug, thiserror::Error)]
pub enum ParliaEngineError {
    #[error("seal error: {0}")]
    Seal(#[from] SealError),
    #[error("snapshot error: {0}")]
    Snapshot(#[from] SnapshotError),
    #[error("validation error: {0}")]
    Validation(#[from] ParliaValidationError),
    #[error("signer {0:?} not in validator set at block {1}")]
    UnauthorizedSigner(Address, BlockNumber),
    #[error("signer {0:?} has signed too recently at block {1}")]
    RecentlySigned(Address, BlockNumber),
    #[error("wrong difficulty at block {block}: expected {expected}, got {actual}")]
    WrongDifficulty {
        block: BlockNumber,
        expected: u64,
        actual: U256,
    },
    #[error(
        "block timestamp {block_ts} is before parent timestamp {parent_ts} + interval {interval}"
    )]
    TimestampTooEarly {
        block_ts: u64,
        parent_ts: u64,
        interval: u64,
    },
    #[error("no snapshot available for parent block {0}")]
    MissingSnapshot(BlockNumber),
}

// ── ParliaEngine ──────────────────────────────────────────────────────────────

/// Central orchestrator for BSC Parlia consensus.
pub struct ParliaEngine {
    /// Parlia consensus configuration (epoch, block interval, turn length).
    pub config: ParliaConfig,
    /// In-memory LRU cache of validator set snapshots.
    pub snapshots: SnapshotCache,
    /// BSC chain ID (56 for mainnet, 97 for Chapel testnet).
    pub chain_id: u64,
}

impl std::fmt::Debug for ParliaEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParliaEngine")
            .field("config", &self.config)
            .field("chain_id", &self.chain_id)
            .finish()
    }
}

impl ParliaEngine {
    /// Creates a new ParliaEngine with the given configuration and chain ID.
    pub fn new(config: ParliaConfig, chain_id: u64) -> Self {
        Self {
            config,
            chain_id,
            snapshots: SnapshotCache::new(),
        }
    }

    // ── Signer recovery ───────────────────────────────────────────────────────

    /// Recover the block signer from the header's seal signature.
    pub fn recover_signer(&self, header: &BlockHeader) -> Result<Address, SealError> {
        recover_signer(header, self.chain_id)
    }

    // ── Structural header validation ──────────────────────────────────────────

    /// Validate the structural properties of a block header against its parent.
    ///
    /// The `is_lorentz` flag enables Lorentz-fork milli-timestamp validation.
    /// This delegates to [`validate_parlia_header`] in `validation.rs`.
    pub fn verify_header(
        &self,
        header: &BlockHeader,
        parent: &BlockHeader,
        is_lorentz: bool,
    ) -> Result<(), ParliaEngineError> {
        validate_parlia_header(header, parent, &self.config, is_lorentz)?;
        Ok(())
    }

    // ── Consensus-level header verification ───────────────────────────────────

    /// Verify a block header against its parent using the current validator
    /// snapshot.
    ///
    /// Performs consensus-level checks on top of structural validation:
    /// 1. Timestamp: `header.timestamp >= parent.timestamp + block_interval_secs`.
    /// 2. Signer is in the current validator set.
    /// 3. Signer has not signed too recently.
    /// 4. Difficulty matches expected (in-turn = 2, out-of-turn = 1).
    ///
    /// Reference: BSC `parlia.go` `verifySeal` and `verifyCascadingFields`.
    pub fn verify_header_with_snap(
        &self,
        header: &BlockHeader,
        parent: &BlockHeader,
        snap: &Snapshot,
    ) -> Result<(), ParliaEngineError> {
        let number = header.number;

        // 1. Timestamp check.
        // snap.block_interval is in milliseconds; header timestamps are in
        // seconds.
        let interval_secs = snap.block_interval / 1_000;
        if header.timestamp < parent.timestamp + interval_secs {
            return Err(ParliaEngineError::TimestampTooEarly {
                block_ts: header.timestamp,
                parent_ts: parent.timestamp,
                interval: interval_secs,
            });
        }

        // 2–4. Recover signer and validate against snapshot.
        let signer = self.recover_signer(header)?;

        // 2. Signer in validator set.
        if !snap.validators.iter().any(|v| v.address == signer) {
            return Err(ParliaEngineError::UnauthorizedSigner(signer, number));
        }

        // 3. Signer has not signed recently.
        if snap.sign_recently(signer) {
            return Err(ParliaEngineError::RecentlySigned(signer, number));
        }

        // 4. Difficulty.
        let expected = snap.difficulty_for(signer);
        let expected_u256 = U256::from(expected);
        if header.difficulty != expected_u256 {
            return Err(ParliaEngineError::WrongDifficulty {
                block: number,
                expected,
                actual: header.difficulty,
            });
        }

        Ok(())
    }

    // ── Fork choice ───────────────────────────────────────────────────────────

    /// Returns `true` if the chain should reorg to `new_header` instead of
    /// staying on `current_header`.
    ///
    /// Algorithm (mirrors BSC `core/forkchoice.go`
    /// `ReorgNeededWithFastFinality`):
    ///
    /// 1. If justified numbers differ, prefer the higher one.
    /// 2. Otherwise compare total difficulties; higher wins.
    /// 3. If TDs are equal, prefer the lower block number (less wasted work).
    /// 4. If block numbers are equal, prefer lower hash (deterministic
    ///    tie-break).
    ///
    /// `current_justified` / `new_justified` are the highest justified
    /// (fast-finality source) block numbers on each branch, obtained from
    /// [`Snapshot::finalized_number`].
    pub fn reorg_needed(
        current: &BlockHeader,
        new_header: &BlockHeader,
        current_td: U256,
        new_td: U256,
        current_justified: u64,
        new_justified: u64,
    ) -> bool {
        // 1. Fast-finality justified number wins.
        if new_justified != current_justified {
            return new_justified > current_justified;
        }

        // 2. Total difficulty.
        if new_td != current_td {
            return new_td > current_td;
        }

        // 3. Block number (lower = less wasted work when TDs are equal).
        if new_header.number != current.number {
            return new_header.number < current.number;
        }

        // 4. Hash (deterministic tie-break).
        let new_hash = new_header.hash.get().copied().unwrap_or(H256::zero());
        let cur_hash = current.hash.get().copied().unwrap_or(H256::zero());
        new_hash.as_bytes() < cur_hash.as_bytes()
    }

    // ── Snapshot bootstrap ────────────────────────────────────────────────────

    /// Walk `headers` (ascending) to find an epoch-aligned boundary, parse
    /// the validator set from its `extra_data`, and return an initial
    /// [`Snapshot`].
    ///
    /// The `post_luban` flag selects the parsing format:
    /// - `true`  → post-Luban (count prefix + 48-byte BLS keys)
    /// - `false` → pre-Luban (packed 20-byte addresses only)
    ///
    /// `chain_id` is required to recover signers for post-epoch headers so that
    /// the returned snapshot's `recents` map is fully populated.
    ///
    /// Reference: BSC `parlia.go` `snapshot` genesis / checkpoint branch.
    pub fn bootstrap_snapshot(
        headers: &[BlockHeader],
        epoch_length: u64,
        turn_length: u8,
        block_interval: u64,
        post_luban: bool,
        chain_id: u64,
    ) -> Result<Snapshot, ParliaEngineError> {
        super::snapshot::bootstrap_snapshot(
            headers,
            epoch_length,
            turn_length,
            block_interval,
            post_luban,
            chain_id,
        )
        .map_err(ParliaEngineError::Snapshot)
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::snapshot::ValidatorInfo;
    use crate::parlia_config::ParliaConfig;
    use ethrex_common::H256;

    fn make_engine() -> ParliaEngine {
        ParliaEngine::new(ParliaConfig::mainnet(), 56)
    }

    fn make_snap(n: usize, turn_length: u8, number: u64) -> Snapshot {
        let validators: Vec<ValidatorInfo> = (1..=(n as u8))
            .map(|b| {
                let mut addr = [0u8; 20];
                addr[19] = b;
                ValidatorInfo {
                    address: Address::from(addr),
                    bls_public_key: [0u8; 48],
                }
            })
            .collect();
        Snapshot::new(number, H256::zero(), validators, turn_length, 200, 3_000)
    }

    #[test]
    fn reorg_needed_by_justified_number() {
        let current = BlockHeader::default();
        let new_h = BlockHeader::default();
        // Higher justified on new chain → reorg.
        assert!(ParliaEngine::reorg_needed(
            &current,
            &new_h,
            U256::from(100),
            U256::from(50),
            5,
            10,
        ));
        // Lower justified on new chain → no reorg.
        assert!(!ParliaEngine::reorg_needed(
            &current,
            &new_h,
            U256::from(100),
            U256::from(200),
            10,
            5,
        ));
    }

    #[test]
    fn reorg_needed_by_total_difficulty() {
        let current = BlockHeader::default();
        let new_h = BlockHeader::default();
        // Equal justified, higher TD → reorg.
        assert!(ParliaEngine::reorg_needed(
            &current,
            &new_h,
            U256::from(100),
            U256::from(200),
            5,
            5,
        ));
        // Equal justified, lower TD → no reorg.
        assert!(!ParliaEngine::reorg_needed(
            &current,
            &new_h,
            U256::from(200),
            U256::from(100),
            5,
            5,
        ));
    }

    #[test]
    fn reorg_needed_by_block_number_when_td_equal() {
        let mut current = BlockHeader::default();
        let mut new_h = BlockHeader::default();
        current.number = 10;
        new_h.number = 9; // lower block number preferred
        assert!(ParliaEngine::reorg_needed(
            &current,
            &new_h,
            U256::from(100),
            U256::from(100),
            5,
            5,
        ));
        new_h.number = 11; // higher block number → no reorg
        assert!(!ParliaEngine::reorg_needed(
            &current,
            &new_h,
            U256::from(100),
            U256::from(100),
            5,
            5,
        ));
    }

    #[test]
    fn verify_header_with_snap_rejects_old_timestamp() {
        let engine = make_engine();
        let snap = make_snap(3, 1, 10);

        let mut parent = BlockHeader::default();
        parent.timestamp = 1_000;

        let mut header = BlockHeader::default();
        header.number = 11;
        // block_interval = 3000 ms → 3 s minimum; 1001 < 1000 + 3 = 1003
        header.timestamp = 1_001;

        let err = engine.verify_header_with_snap(&header, &parent, &snap);
        assert!(
            matches!(err, Err(ParliaEngineError::TimestampTooEarly { .. })),
            "unexpected: {:?}",
            err
        );
    }

    #[test]
    fn verify_header_with_snap_passes_timestamp_check() {
        let engine = make_engine();
        let snap = make_snap(3, 1, 10);

        let mut parent = BlockHeader::default();
        parent.timestamp = 1_000;

        let mut header = BlockHeader::default();
        header.number = 11;
        header.timestamp = 1_003; // exactly at the boundary → OK for timestamp

        // Will fail on seal recovery because extra_data is empty, which is
        // expected — we're only testing that the timestamp check passes.
        let err = engine.verify_header_with_snap(&header, &parent, &snap);
        assert!(
            !matches!(err, Err(ParliaEngineError::TimestampTooEarly { .. })),
            "should not fail with TimestampTooEarly: {:?}",
            err
        );
    }
}
