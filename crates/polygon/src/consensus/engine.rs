use std::cmp::Ordering;
use std::sync::Mutex;

use ethereum_types::Address;
use ethrex_common::U256;
use ethrex_common::types::{BlockHeader, BlockNumber, InvalidBlockHeaderError};

use crate::bor_config::BorConfig;
use crate::heimdall::{HeimdallClient, HeimdallError, Milestone};
use crate::system_calls::{
    self, MAX_SYSTEM_CALL_GAS, STATE_RECEIVER_CONTRACT, SYSTEM_ADDRESS, SystemCallContext,
    VALIDATOR_CONTRACT,
};
use crate::validation::validate_bor_header;

use super::seal::{SealError, recover_signer};
use super::snapshot::{Snapshot, SnapshotCache, SnapshotError};

/// Errors from BorEngine operations.
#[derive(Debug, thiserror::Error)]
pub enum BorEngineError {
    #[error("seal error: {0}")]
    Seal(#[from] SealError),
    #[error("snapshot error: {0}")]
    Snapshot(#[from] SnapshotError),
    #[error("invalid header: {0}")]
    InvalidHeader(#[from] InvalidBlockHeaderError),
    #[error("heimdall error: {0}")]
    Heimdall(#[from] HeimdallError),
    #[error("signer {0:?} not in validator set at block {1}")]
    UnauthorizedSigner(Address, BlockNumber),
    #[error("wrong difficulty at block {block}: expected {expected}, got {actual}")]
    WrongDifficulty {
        block: BlockNumber,
        expected: u64,
        actual: U256,
    },
    #[error("no snapshot available for parent block {0}")]
    MissingSnapshot(BlockNumber),
    #[error("reorg blocked by milestone: cannot revert past block {0}")]
    MilestoneReorgBlocked(BlockNumber),
}

/// Central orchestrator for Polygon Bor consensus.
///
/// Combines the Heimdall client, snapshot cache, Bor config, and milestone
/// tracker to provide the main consensus entry points: header verification,
/// seal verification, block finalization, and fork choice.
pub struct BorEngine {
    /// Bor consensus configuration (sprint sizes, fork blocks, contract addresses).
    pub config: BorConfig,
    /// HTTP client for the Heimdall sidecar (spans, state sync, milestones).
    pub heimdall: HeimdallClient,
    /// In-memory LRU cache of validator set snapshots.
    pub snapshots: SnapshotCache,
    /// Latest known milestone from Heimdall, used for reorg protection.
    latest_milestone: Mutex<Option<Milestone>>,
}

impl std::fmt::Debug for BorEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BorEngine")
            .field("config", &"<BorConfig>")
            .finish()
    }
}

impl BorEngine {
    /// Creates a new BorEngine.
    ///
    /// # Arguments
    /// * `config` — Bor consensus parameters (parsed from genesis or chain config)
    /// * `heimdall_url` — Base URL of the Heimdall REST API (e.g., "http://localhost:1317")
    pub fn new(config: BorConfig, heimdall_url: &str) -> Self {
        Self {
            config,
            heimdall: HeimdallClient::new(heimdall_url),
            snapshots: SnapshotCache::new(),
            latest_milestone: Mutex::new(None),
        }
    }

    /// Recover the block signer from the header's seal signature.
    pub fn recover_signer(header: &BlockHeader) -> Result<Address, SealError> {
        recover_signer(header)
    }

    /// Returns the latest known milestone, if any.
    pub fn latest_milestone(&self) -> Option<Milestone> {
        self.latest_milestone.lock().unwrap().clone()
    }

    /// Updates the latest milestone (called by the Heimdall poller).
    pub fn set_milestone(&self, milestone: Milestone) {
        *self.latest_milestone.lock().unwrap() = Some(milestone);
    }

    /// Look up or create the snapshot for a given block hash.
    ///
    /// Returns the cached snapshot if available.
    pub fn get_snapshot(&self, hash: &ethrex_common::H256) -> Option<Snapshot> {
        self.snapshots.get(hash)
    }

    /// Insert a snapshot into the cache.
    pub fn put_snapshot(&self, snapshot: Snapshot) {
        self.snapshots.insert(snapshot);
    }

    /// Returns whether a reorg past the given block number is allowed
    /// by the current milestone.
    ///
    /// If a milestone covers blocks up to `end_block`, we must not
    /// revert any block <= `end_block`.
    pub fn is_reorg_allowed(&self, revert_to: BlockNumber) -> bool {
        match self.latest_milestone() {
            Some(milestone) => revert_to > milestone.end_block,
            None => true, // No milestone known yet — allow all reorgs
        }
    }

    /// Verify a block header against its parent.
    ///
    /// Performs:
    /// 1. Structural validation (gas, timestamps, Bor-specific field checks)
    /// 2. Seal verification — recovers the signer from the signature
    /// 3. Authorization check — verifies the signer is in the validator set
    /// 4. Difficulty validation — checks difficulty matches the signer's succession
    /// 5. Advances the snapshot (records signer in recents, prunes old entries)
    ///
    /// The `parent_snapshot` must be the snapshot at the parent block. On success,
    /// the snapshot is mutated to reflect this block and the signer address is returned.
    pub fn verify_header(
        &self,
        header: &BlockHeader,
        parent_header: &BlockHeader,
        parent_snapshot: &mut Snapshot,
    ) -> Result<Address, BorEngineError> {
        // 1. Structural validation
        validate_bor_header(header, parent_header)?;

        // 2 & 3. Recover signer and check authorization via snapshot.
        // apply_header recovers the signer, checks they're in the validator set,
        // records the signer in recents, and prunes old entries.
        let sprint_size = self.config.get_sprint_size(header.number);
        let signer = parent_snapshot.apply_header(header, sprint_size)?;

        // 4. Difficulty validation.
        // difficulty = total_validators - succession, where succession is the
        // ring distance from proposer to signer. In-turn proposer gets the
        // highest difficulty (= total_validators), farthest gets 1.
        if let Some(expected) = parent_snapshot.expected_difficulty(&signer)
            && header.difficulty != U256::from(expected)
        {
            return Err(BorEngineError::WrongDifficulty {
                block: header.number,
                expected,
                actual: header.difficulty,
            });
        }

        // 5. Proposer rotation at sprint boundaries.
        // Must happen AFTER the difficulty check since difficulty is computed
        // against the pre-rotation proposer.
        if header.number > 0 && (header.number + 1).is_multiple_of(sprint_size) {
            parent_snapshot.increment_proposer_priority(1);
        }

        Ok(signer)
    }

    /// Build the list of system calls to execute during block finalization.
    ///
    /// System calls are executed AFTER all regular transactions but before state
    /// root computation. Each call uses `state.Finalise(true)` after execution.
    /// Gas is NOT counted toward block gasUsed.
    ///
    /// **Execution order** (matching Bor's `Finalize`):
    /// 1. Span commit (`checkAndCommitSpan`) — at sprint-start of last sprint in span
    /// 2. State sync (`CommitStates`) — at sprint-start blocks
    /// 3. Contract code upgrades (`changeContractCodeIfNeeded`) — any matching block
    ///
    /// # Arguments
    /// * `block_number` — the block being finalized
    /// * `header_timestamp` — current block's timestamp (for state sync time window)
    /// * `last_state_id` — the last committed state ID from the state receiver contract
    ///   (obtained by calling `lastStateId()` on 0x1001)
    ///
    /// Returns the list of `SystemCallContext` to execute against the EVM in order.
    pub async fn get_system_calls(
        &self,
        block_number: BlockNumber,
        header_timestamp: u64,
        last_state_id: u64,
    ) -> Result<Vec<SystemCallContext>, BorEngineError> {
        let mut calls = Vec::new();

        // 1. Span commit at span boundaries (checked at sprint-start blocks)
        if self.config.need_to_commit_span(block_number) {
            let span_call = self.build_span_commit_call(block_number).await?;
            calls.push(span_call);
        }

        // 2. State sync at sprint-start blocks
        if self.config.is_sprint_start(block_number) && block_number > 0 {
            let state_sync_calls = self
                .build_state_sync_calls(block_number, header_timestamp, last_state_id)
                .await?;
            calls.extend(state_sync_calls);
        }

        // 3. Contract code upgrades are handled separately by the caller
        // via get_block_alloc_updates(), since they modify code/storage directly
        // rather than going through the EVM.

        Ok(calls)
    }

    /// Returns the block_alloc updates for a given block number, if any.
    ///
    /// These are direct code/storage modifications to system contracts
    /// (e.g., hard fork upgrades) that are applied during finalization
    /// WITHOUT going through the EVM. The caller should apply these
    /// after executing all system calls.
    pub fn get_block_alloc_updates(
        &self,
        block_number: BlockNumber,
    ) -> Option<&std::collections::HashMap<Address, ethrex_common::types::GenesisAccount>> {
        self.config.block_alloc.get(&block_number)
    }

    /// Build commitState system calls for state sync events.
    ///
    /// Fetches pending state sync events from Heimdall for the time window
    /// ending at `header_timestamp - confirmation_delay`, starting from
    /// `last_state_id + 1`.
    ///
    /// Note: EVM reverts in individual commitState calls are non-fatal.
    /// The caller should log them but continue to the next event.
    async fn build_state_sync_calls(
        &self,
        block_number: BlockNumber,
        header_timestamp: u64,
        last_state_id: u64,
    ) -> Result<Vec<SystemCallContext>, BorEngineError> {
        let delay = self.config.get_state_sync_delay(block_number);
        let to_time = header_timestamp - delay;
        let from_id = last_state_id + 1;

        // Heimdall returns events ordered by ID.
        // The limit is generous — Bor uses 100 by default.
        let events = self
            .heimdall
            .fetch_state_sync_events(from_id, to_time, 100)
            .await?;

        let calls = events
            .into_iter()
            .map(|event| {
                // Each event's `data` field is hex-encoded RLP bytes for the record.
                let record_bytes = hex_decode_data(&event.data);
                let sync_time = to_time;
                let data = system_calls::encode_commit_state(sync_time, &record_bytes);
                SystemCallContext {
                    from: SYSTEM_ADDRESS,
                    to: STATE_RECEIVER_CONTRACT,
                    data,
                    gas_limit: MAX_SYSTEM_CALL_GAS,
                    gas_price: ethereum_types::U256::zero(),
                    value: ethereum_types::U256::zero(),
                    revert_ok: true, // commitState reverts are non-fatal
                }
            })
            .collect();

        Ok(calls)
    }

    /// Build the commitSpan system call for a span boundary.
    ///
    /// Fetches the next span from Heimdall and encodes the commitSpan call
    /// with the new validator and producer sets.
    async fn build_span_commit_call(
        &self,
        block_number: BlockNumber,
    ) -> Result<SystemCallContext, BorEngineError> {
        let current_span_id = self.config.span_id_at(block_number);
        let next_span = self.heimdall.fetch_span(current_span_id + 1).await?;

        // Encode validators and producers as Bor-format bytes (40 bytes each:
        // 20-byte address + 20-byte big-endian padded voting power).
        let validator_bytes = encode_validator_bytes(&next_span.validators);
        let producer_bytes = encode_validator_bytes(&next_span.selected_producers);

        let data = system_calls::encode_commit_span(
            next_span.id,
            next_span.start_block,
            next_span.end_block,
            &validator_bytes,
            &producer_bytes,
        );

        Ok(SystemCallContext {
            from: SYSTEM_ADDRESS,
            to: VALIDATOR_CONTRACT,
            data,
            gas_limit: MAX_SYSTEM_CALL_GAS,
            gas_price: ethereum_types::U256::zero(),
            value: ethereum_types::U256::zero(),
            revert_ok: false, // commitSpan reverts ARE fatal
        })
    }

    // ---- Snapshot bootstrapping ----

    /// Bootstrap the validator set snapshot after snap sync.
    ///
    /// When a node snap-syncs to a pivot block, it doesn't have historical headers
    /// to reconstruct the validator snapshot. This method fetches the current span
    /// from Heimdall and creates an initial snapshot at the pivot block.
    ///
    /// The resulting snapshot has an empty `recents` map — recent signers will be
    /// populated as new blocks are processed. This means the first few blocks after
    /// bootstrap may have relaxed difficulty checks, which is acceptable since the
    /// node is syncing and not producing blocks.
    ///
    /// # Arguments
    /// * `block_number` — the snap-sync pivot block number
    /// * `block_hash` — the pivot block's hash
    pub async fn bootstrap_snapshot(
        &self,
        block_number: BlockNumber,
        block_hash: ethrex_common::H256,
    ) -> Result<Snapshot, BorEngineError> {
        // Determine which span covers this block and fetch it.
        let span_id = self.config.span_id_at(block_number);
        let span = self.heimdall.fetch_span(span_id).await?;

        // Convert Heimdall validators to snapshot validator entries.
        let validator_set = span_to_validator_set(&span);

        // Create the snapshot at the pivot block.
        let snapshot = Snapshot::new(block_number, block_hash, validator_set);

        // Cache it for immediate use by verify_header().
        self.snapshots.insert(snapshot.clone());

        tracing::info!(
            block_number,
            span_id = span.id,
            validators = span.validators.len(),
            producers = span.selected_producers.len(),
            "Bootstrapped validator snapshot from Heimdall"
        );

        Ok(snapshot)
    }

    // ---- Fork choice ----

    /// Compare two chain heads by total difficulty for fork choice.
    ///
    /// Bor uses cumulative difficulty (total difficulty = sum of all block difficulties)
    /// to determine the canonical chain. The chain with higher TD wins.
    /// On tie, the chain with the higher block number wins.
    /// On further tie, the chain with the lower block hash wins (deterministic tiebreak).
    ///
    /// Returns `Ordering::Greater` if chain A should be preferred over chain B.
    pub fn compare_td(
        td_a: &U256,
        number_a: BlockNumber,
        hash_a: &ethrex_common::H256,
        td_b: &U256,
        number_b: BlockNumber,
        hash_b: &ethrex_common::H256,
    ) -> Ordering {
        match td_a.cmp(td_b) {
            Ordering::Equal => {
                // Tiebreak 1: higher block number
                match number_a.cmp(&number_b) {
                    Ordering::Equal => {
                        // Tiebreak 2: lower hash wins (deterministic)
                        hash_b.cmp(hash_a)
                    }
                    ord => ord,
                }
            }
            ord => ord,
        }
    }

    /// Check if a reorg to a new chain tip is allowed, considering milestone protection.
    ///
    /// Returns `Ok(())` if the reorg is allowed, or an error if a milestone prevents it.
    /// `fork_block_number` is the block number where the new chain diverges from the
    /// current canonical chain.
    pub fn check_reorg_allowed(
        &self,
        fork_block_number: BlockNumber,
    ) -> Result<(), BorEngineError> {
        if self.is_reorg_allowed(fork_block_number) {
            Ok(())
        } else {
            let milestone = self.latest_milestone().expect("milestone must exist");
            Err(BorEngineError::MilestoneReorgBlocked(milestone.end_block))
        }
    }
}

/// Convert a Heimdall span's validator set to snapshot `ValidatorInfo` entries.
///
/// Uses the span's full `validators` list (not just `selected_producers`), since
/// all validators need to be tracked for signer authorization and difficulty
/// calculation.
fn span_to_validator_set(span: &crate::heimdall::Span) -> Vec<super::snapshot::ValidatorInfo> {
    span.validators
        .iter()
        .map(|v| super::snapshot::ValidatorInfo {
            address: v.signer,
            voting_power: v.voting_power,
            proposer_priority: v.proposer_priority,
        })
        .collect()
}

/// Encode a list of Heimdall validators as Bor-format bytes.
///
/// Each validator is 40 bytes: [20-byte address][20-byte big-endian padded voting power].
pub fn encode_validator_bytes(validators: &[crate::heimdall::Validator]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(validators.len() * 40);
    for v in validators {
        bytes.extend_from_slice(v.signer.as_bytes());
        let mut power = [0u8; 20];
        power[12..].copy_from_slice(&v.voting_power.to_be_bytes());
        bytes.extend_from_slice(&power);
    }
    bytes
}

/// Decode a hex string (with or without 0x prefix) into bytes.
/// Returns empty vec on invalid input.
fn hex_decode_data(s: &str) -> Vec<u8> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(s).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::H256;

    /// Build a minimal BorConfig for testing (doesn't need real chain params).
    fn test_config() -> BorConfig {
        serde_json::from_str(
            r#"{
                "period": {"0": 2},
                "producerDelay": {"0": 6},
                "sprint": {"0": 16},
                "backupMultiplier": {"0": 2},
                "validatorContract": "0x0000000000000000000000000000000000001000",
                "stateReceiverContract": "0x0000000000000000000000000000000000001001",
                "jaipurBlock": 0,
                "delhiBlock": 0,
                "indoreBlock": 0
            }"#,
        )
        .expect("valid test config")
    }

    fn test_engine() -> BorEngine {
        BorEngine::new(test_config(), "http://localhost:1317")
    }

    // ---- compare_td tests ----

    #[test]
    fn compare_td_higher_td_wins() {
        let td_a = U256::from(200);
        let td_b = U256::from(100);
        let hash = H256::zero();
        assert_eq!(
            BorEngine::compare_td(&td_a, 10, &hash, &td_b, 10, &hash),
            Ordering::Greater
        );
    }

    #[test]
    fn compare_td_lower_td_loses() {
        let td_a = U256::from(100);
        let td_b = U256::from(200);
        let hash = H256::zero();
        assert_eq!(
            BorEngine::compare_td(&td_a, 10, &hash, &td_b, 10, &hash),
            Ordering::Less
        );
    }

    #[test]
    fn compare_td_equal_td_higher_number_wins() {
        let td = U256::from(100);
        let hash = H256::zero();
        assert_eq!(
            BorEngine::compare_td(&td, 20, &hash, &td, 10, &hash),
            Ordering::Greater
        );
    }

    #[test]
    fn compare_td_equal_td_lower_number_loses() {
        let td = U256::from(100);
        let hash = H256::zero();
        assert_eq!(
            BorEngine::compare_td(&td, 10, &hash, &td, 20, &hash),
            Ordering::Less
        );
    }

    #[test]
    fn compare_td_equal_td_equal_number_lower_hash_wins() {
        let td = U256::from(100);
        // hash_a > hash_b numerically, so hash_b is "lower" → B wins → A is Less
        let hash_a = H256::from_low_u64_be(0xFF);
        let hash_b = H256::from_low_u64_be(0x01);
        assert_eq!(
            BorEngine::compare_td(&td, 10, &hash_a, &td, 10, &hash_b),
            Ordering::Less
        );
        // Reverse: hash_a < hash_b → A wins
        assert_eq!(
            BorEngine::compare_td(&td, 10, &hash_b, &td, 10, &hash_a),
            Ordering::Greater
        );
    }

    #[test]
    fn compare_td_all_equal() {
        let td = U256::from(100);
        let hash = H256::from_low_u64_be(0x42);
        assert_eq!(
            BorEngine::compare_td(&td, 10, &hash, &td, 10, &hash),
            Ordering::Equal
        );
    }

    #[test]
    fn compare_td_zero_td() {
        let zero = U256::zero();
        let one = U256::from(1);
        let hash = H256::zero();
        assert_eq!(
            BorEngine::compare_td(&zero, 0, &hash, &one, 0, &hash),
            Ordering::Less
        );
        assert_eq!(
            BorEngine::compare_td(&zero, 0, &hash, &zero, 0, &hash),
            Ordering::Equal
        );
    }

    // ---- is_reorg_allowed tests ----

    #[test]
    fn is_reorg_allowed_no_milestone() {
        let engine = test_engine();
        // No milestone set → all reorgs allowed
        assert!(engine.is_reorg_allowed(0));
        assert!(engine.is_reorg_allowed(100));
        assert!(engine.is_reorg_allowed(u64::MAX));
    }

    #[test]
    fn is_reorg_allowed_above_milestone() {
        let engine = test_engine();
        engine.set_milestone(Milestone {
            id: 1,
            start_block: 50,
            end_block: 100,
            hash: H256::zero(),
        });
        // revert_to 101 > end_block 100 → allowed
        assert!(engine.is_reorg_allowed(101));
        assert!(engine.is_reorg_allowed(200));
    }

    #[test]
    fn is_reorg_allowed_at_milestone_boundary_not_allowed() {
        let engine = test_engine();
        engine.set_milestone(Milestone {
            id: 1,
            start_block: 50,
            end_block: 100,
            hash: H256::zero(),
        });
        // revert_to 100 == end_block → strictly greater required → NOT allowed
        assert!(!engine.is_reorg_allowed(100));
    }

    #[test]
    fn is_reorg_allowed_below_milestone_not_allowed() {
        let engine = test_engine();
        engine.set_milestone(Milestone {
            id: 1,
            start_block: 50,
            end_block: 100,
            hash: H256::zero(),
        });
        assert!(!engine.is_reorg_allowed(50));
        assert!(!engine.is_reorg_allowed(0));
        assert!(!engine.is_reorg_allowed(99));
    }

    // ---- check_reorg_allowed tests ----

    #[test]
    fn check_reorg_allowed_returns_ok_when_allowed() {
        let engine = test_engine();
        engine.set_milestone(Milestone {
            id: 1,
            start_block: 50,
            end_block: 100,
            hash: H256::zero(),
        });
        assert!(engine.check_reorg_allowed(101).is_ok());
    }

    #[test]
    fn check_reorg_allowed_returns_err_when_blocked() {
        let engine = test_engine();
        engine.set_milestone(Milestone {
            id: 1,
            start_block: 50,
            end_block: 100,
            hash: H256::zero(),
        });
        let err = engine.check_reorg_allowed(100).unwrap_err();
        assert!(
            matches!(err, BorEngineError::MilestoneReorgBlocked(100)),
            "expected MilestoneReorgBlocked(100), got {err:?}"
        );
    }

    #[test]
    fn check_reorg_allowed_no_milestone_always_ok() {
        let engine = test_engine();
        assert!(engine.check_reorg_allowed(0).is_ok());
        assert!(engine.check_reorg_allowed(u64::MAX).is_ok());
    }

    // ---- set_milestone / latest_milestone tests ----

    #[test]
    fn milestone_starts_as_none() {
        let engine = test_engine();
        assert!(engine.latest_milestone().is_none());
    }

    #[test]
    fn set_milestone_updates_latest() {
        let engine = test_engine();
        let m1 = Milestone {
            id: 1,
            start_block: 0,
            end_block: 100,
            hash: H256::from_low_u64_be(0x1),
        };
        engine.set_milestone(m1.clone());
        let got = engine.latest_milestone().expect("should have milestone");
        assert_eq!(got.id, 1);
        assert_eq!(got.end_block, 100);

        // Update with a newer milestone
        let m2 = Milestone {
            id: 2,
            start_block: 101,
            end_block: 200,
            hash: H256::from_low_u64_be(0x2),
        };
        engine.set_milestone(m2);
        let got = engine.latest_milestone().expect("should have milestone");
        assert_eq!(got.id, 2);
        assert_eq!(got.end_block, 200);
    }
}
