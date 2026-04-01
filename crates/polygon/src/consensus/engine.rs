use std::cmp::Ordering;
use std::sync::Mutex;

use ethereum_types::Address;
use ethrex_common::U256;
use ethrex_common::types::{BlockHeader, BlockNumber, InvalidBlockHeaderError};
use tokio_util::sync::CancellationToken;

use crate::bor_config::BorConfig;
use crate::heimdall::{HeimdallClient, HeimdallError, Milestone};
use crate::validation::validate_bor_header;

use super::extra_data::{
    EXTRA_SEAL_LENGTH, EXTRA_VANITY_LENGTH, parse_extra_data, parse_validators,
};
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
    /// Current span from Heimdall, used for accurate span boundary detection.
    /// Set during bootstrap and updated after each commitSpan.
    current_span: Mutex<Option<crate::heimdall::Span>>,
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
    /// * `cancel_token` — Token to signal shutdown and cancel in-flight Heimdall retries
    pub fn new(config: BorConfig, heimdall_url: &str, cancel_token: CancellationToken) -> Self {
        Self {
            config,
            heimdall: HeimdallClient::new(heimdall_url, cancel_token),
            snapshots: SnapshotCache::new(),
            latest_milestone: Mutex::new(None),
            current_span: Mutex::new(None),
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

    /// Returns the current span's ID, if known.
    pub fn current_span_id(&self) -> Option<u64> {
        self.current_span.lock().unwrap().as_ref().map(|s| s.id)
    }

    /// Updates the stored current span (called after bootstrap or commitSpan).
    pub fn set_current_span(&self, span: crate::heimdall::Span) {
        *self.current_span.lock().unwrap() = Some(span);
    }

    /// Returns true if a span commit system call is needed at this block.
    ///
    /// Uses the actual span boundaries from Heimdall instead of the formula-based
    /// `span_id_at()` which can be inaccurate on networks like Amoy.
    ///
    /// Trigger condition: this block is a sprint start, and this sprint's end
    /// reaches or crosses the current span's end_block.
    /// Falls back to the formula-based check if no span is stored yet.
    pub fn need_to_commit_span(&self, block_number: BlockNumber) -> bool {
        if !self.config.is_sprint_start(block_number) || block_number == 0 {
            return false;
        }
        let span = self.current_span.lock().unwrap();
        match span.as_ref() {
            Some(span) => {
                let sprint = self.config.get_sprint_size(block_number);
                let sprint_end = block_number + sprint - 1;
                sprint_end >= span.end_block
            }
            // No span stored yet — fall back to formula.
            None => self.config.need_to_commit_span(block_number),
        }
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
        validate_bor_header(header, parent_header, &self.config)?;

        // 2 & 3. Recover signer and check authorization via snapshot.
        // apply_header recovers the signer, checks they're in the validator set,
        // records the signer in recents, and prunes old entries.
        let sprint_size = self.config.get_sprint_size(header.number);
        let signer = parent_snapshot.apply_header(header, sprint_size)?;

        // 4. Difficulty validation.
        // difficulty = total_validators - succession, where succession is the
        // ring distance from proposer to signer. In-turn proposer gets the
        // highest difficulty (= total_validators), farthest gets 1.
        // Post-Rio: skip difficulty check — snapshot uses full validator set (25) for
        // authorization but difficulty is computed from selected_producers only.
        if !self.config.is_rio_active(header.number)
            && let Some(expected) = parent_snapshot.expected_difficulty(&signer)
            && header.difficulty != U256::from(expected)
        {
            return Err(BorEngineError::WrongDifficulty {
                block: header.number,
                expected,
                actual: header.difficulty,
            });
        }

        // 5. Validator bytes verification at sprint-end.
        // At sprint-end blocks, verify that the validator set in the header's extra data
        // matches the snapshot's validator set (sorted by address).
        // Skipped post-Rio (same as difficulty check — different validator set semantics).
        if !self.config.is_rio_active(header.number)
            && header.number > 0
            && (header.number + 1).is_multiple_of(sprint_size)
        {
            self.verify_sprint_end_validators(header, parent_snapshot)?;
        }

        // 6. Proposer rotation at sprint boundaries.
        // Must happen AFTER the difficulty check since difficulty is computed
        // against the pre-rotation proposer.
        if header.number > 0 && (header.number + 1).is_multiple_of(sprint_size) {
            parent_snapshot.increment_proposer_priority(1);
        }

        Ok(signer)
    }

    /// Verify that a sprint-end block's extra data contains the correct validator set.
    ///
    /// Extracts validator bytes from the header and compares them against the
    /// snapshot's validator set (sorted by address). This matches Bor's
    /// `verifyCascadingFields` check at `bor.go:614-656`.
    fn verify_sprint_end_validators(
        &self,
        header: &BlockHeader,
        snapshot: &Snapshot,
    ) -> Result<(), BorEngineError> {
        // Extract validator bytes from the header's extra data.
        // Pre-Lisovo: raw bytes between vanity and seal.
        // Post-Lisovo: RLP-decoded BlockExtraData.validator_bytes.
        let validator_bytes = if self.config.is_lisovo_active(header.number) {
            let (_vanity, block_extra, _sig) = parse_extra_data(&header.extra_data)
                .map_err(|_| InvalidBlockHeaderError::PolygonInvalidSprintEndValidators)?;
            block_extra.validator_bytes
        } else {
            let extra = &header.extra_data[..];
            if extra.len() < EXTRA_VANITY_LENGTH + EXTRA_SEAL_LENGTH {
                return Err(InvalidBlockHeaderError::PolygonInvalidSprintEndValidators.into());
            }
            extra[EXTRA_VANITY_LENGTH..extra.len() - EXTRA_SEAL_LENGTH].to_vec()
        };

        let header_vals = parse_validators(&validator_bytes)
            .map_err(|_| InvalidBlockHeaderError::PolygonInvalidSprintEndValidators)?;

        // Build expected validator set from snapshot, sorted by address.
        let mut expected: Vec<_> = snapshot
            .validator_set
            .iter()
            .map(|v| (v.address, v.voting_power))
            .collect();
        expected.sort_by_key(|(addr, _)| *addr);

        if header_vals.len() != expected.len() {
            tracing::warn!(
                block = header.number,
                header_count = header_vals.len(),
                snapshot_count = expected.len(),
                "Validator set length mismatch at sprint-end"
            );
            return Err(InvalidBlockHeaderError::PolygonInvalidSprintEndValidators.into());
        }

        for (i, (header_val, (exp_addr, exp_power))) in
            header_vals.iter().zip(expected.iter()).enumerate()
        {
            if header_val.address != *exp_addr || header_val.voting_power != U256::from(*exp_power)
            {
                tracing::warn!(
                    block = header.number,
                    index = i,
                    header_addr = ?header_val.address,
                    expected_addr = ?exp_addr,
                    "Validator mismatch at sprint-end"
                );
                return Err(InvalidBlockHeaderError::PolygonInvalidSprintEndValidators.into());
            }
        }

        Ok(())
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

    // ---- Snapshot validator set update at span boundaries ----

    /// Update the snapshot's validator set if this block is the last block of a span.
    ///
    /// In Bor, the validator set changes at span boundaries. The `commitSpan` system call
    /// (executed at the sprint-start of the last sprint in the span) writes the next span's
    /// validators to the ValidatorSet contract and updates `current_span`. At the sprint-end
    /// block (= span end), the snapshot must be updated so subsequent blocks in the new span
    /// are verified against the correct validator set.
    ///
    /// Two cases are handled:
    /// 1. `commitSpan` already ran → `current_span` is the next span, use it directly.
    /// 2. `commitSpan` didn't run (e.g., node bootstrapped after the commitSpan block) →
    ///    fetch the next span from Heimdall.
    pub async fn update_snapshot_at_span_boundary(
        &self,
        block_number: BlockNumber,
        snapshot: &mut Snapshot,
    ) -> Result<(), BorEngineError> {
        // Only sprint-end blocks can be span boundaries.
        if !self.config.is_sprint_end(block_number) {
            return Ok(());
        }

        let next_block = block_number + 1;

        // Check if the next block enters a new span.
        let (needs_update, have_next_span) = {
            let span = self.current_span.lock().unwrap();
            match span.as_ref() {
                // commitSpan already updated current_span to the next span.
                Some(s) if next_block == s.start_block => (true, true),
                // current_span was NOT updated (bootstrapped after commitSpan block).
                Some(s) if block_number == s.end_block => (true, false),
                _ => (false, false),
            }
        };

        if !needs_update {
            return Ok(());
        }

        let next_span = if have_next_span {
            self.current_span.lock().unwrap().clone().unwrap()
        } else {
            // Fetch the span covering the next block from Heimdall.
            self.fetch_span_for_block(next_block).await?
        };

        let is_rio = self.config.is_rio_active(next_block);
        let new_validators = span_to_validator_set(&next_span, is_rio);

        tracing::info!(
            block_number,
            new_span_id = next_span.id,
            new_span_start = next_span.start_block,
            new_span_end = next_span.end_block,
            validators = new_validators.len(),
            "Updating snapshot validator set at span boundary"
        );

        snapshot.update_validator_set(new_validators);

        if !have_next_span {
            self.set_current_span(next_span);
        }

        Ok(())
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
        // Fetch the span covering this block from Heimdall.
        // We use fetch_latest_span() and walk back instead of span_id_at() because
        // the formula-based span ID calculation can be off on networks like Amoy
        // where spans don't follow the uniform-from-genesis pattern.
        let span = self.fetch_span_for_block(block_number).await?;

        // Convert Heimdall span to snapshot validator entries.
        // Always uses full validator set for signer authorization.
        // Post-Rio: sorted by address for deterministic ordering.
        let is_rio = self.config.is_rio_active(block_number);
        let validator_set = span_to_validator_set(&span, is_rio);

        // Create the snapshot at the pivot block.
        let mut snapshot = Snapshot::new(block_number, block_hash, validator_set);

        // Fast-forward proposer rotation to match current block position.
        // Heimdall returns priorities from span start, but Bor rotates at each sprint boundary.
        // Post-Rio: Bor creates fresh snapshots from sorted producers without rotation.
        if !is_rio {
            let span_start = span.start_block;
            let sprint_size = self.config.get_sprint_size(block_number);
            let sprints_elapsed = if block_number > span_start && sprint_size > 0 {
                (block_number - span_start) / sprint_size
            } else {
                0
            };
            if sprints_elapsed > 0 {
                snapshot.increment_proposer_priority(sprints_elapsed as u32);
                tracing::info!(
                    sprints_elapsed,
                    "Fast-forwarded proposer priority from span start"
                );
            }
        }

        // Cache it for immediate use by verify_header().
        self.snapshots.insert(snapshot.clone());

        // Store the current span for accurate boundary detection.
        tracing::info!(
            block_number,
            span_id = span.id,
            span_start = span.start_block,
            span_end = span.end_block,
            validators = span.validators.len(),
            producers = span.selected_producers.len(),
            "Bootstrapped validator snapshot from Heimdall"
        );
        self.set_current_span(span);

        Ok(snapshot)
    }

    /// Fetch the span that covers the given block number from Heimdall.
    ///
    /// Starts from the formula-estimated span ID and walks in the correct
    /// direction until finding one where `start_block <= block_number`.
    /// The formula is typically off by a bounded amount (~200 on Amoy,
    /// ~234 on mainnet), so this converges quickly.
    async fn fetch_span_for_block(
        &self,
        block_number: BlockNumber,
    ) -> Result<crate::heimdall::Span, BorEngineError> {
        // Span 0 covers blocks 0..=255 — fetch directly.
        if block_number <= 255 {
            return Ok(self.heimdall.fetch_span(0).await?);
        }

        // Start from the formula estimate and search from there.
        let estimated_id = self.config.span_id_at(block_number);
        let mut span = self.heimdall.fetch_span(estimated_id).await?;

        // Walk back if the estimate is too high.
        while span.start_block > block_number && span.id > 0 {
            tracing::debug!(
                span_id = span.id,
                span_start = span.start_block,
                block_number,
                "Span too new, fetching previous"
            );
            span = self.heimdall.fetch_span(span.id - 1).await?;
        }

        // Walk forward if the estimate is too low.
        while span.end_block < block_number {
            tracing::debug!(
                span_id = span.id,
                span_end = span.end_block,
                block_number,
                "Span too old, fetching next"
            );
            span = self.heimdall.fetch_span(span.id + 1).await?;
        }

        tracing::info!(
            block_number,
            span_id = span.id,
            span_start = span.start_block,
            span_end = span.end_block,
            "Found span for bootstrap block"
        );

        Ok(span)
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

/// Convert a Heimdall span to snapshot `ValidatorInfo` entries.
///
/// Pre-Rio: uses the full `validators` list for signer authorization and difficulty.
/// Post-Rio: uses `selected_producers` (sorted by address), matching Bor's behavior
/// where only block producers participate in proposer rotation and difficulty.
fn span_to_validator_set(
    span: &crate::heimdall::Span,
    is_rio: bool,
) -> Vec<super::snapshot::ValidatorInfo> {
    // Always use the full validator set for signer authorization.
    // Post-Rio, all 25 validators can sign blocks (not just selected_producers).
    // selected_producers is only used for difficulty/proposer selection.
    let mut set: Vec<super::snapshot::ValidatorInfo> = span
        .validators
        .iter()
        .map(|v| super::snapshot::ValidatorInfo {
            address: v.signer,
            voting_power: v.voting_power,
            proposer_priority: v.proposer_priority,
        })
        .collect();
    if is_rio {
        set.sort_by_key(|v| v.address);
    }
    set
}

/// Encode a list of Heimdall validators as RLP bytes for commitSpan.
///
/// Matches Bor's `rlp.EncodeToBytes([]MinimalVal{{ID, VotingPower, Signer}})`:
/// an RLP list of `[id: u64, voting_power: u64, signer: Address]` lists.
pub fn encode_validator_bytes(validators: &[crate::heimdall::Validator]) -> Vec<u8> {
    use ethrex_rlp::encode::encode_length;

    // Each validator is RLP-encoded as a list [id, power, signer]
    let encoded_vals: Vec<Vec<u8>> = validators
        .iter()
        .map(|v| {
            let mut buf = Vec::new();
            ethrex_rlp::structs::Encoder::new(&mut buf)
                .encode_field(&v.id)
                .encode_field(&v.voting_power)
                .encode_field(&v.signer)
                .finish();
            buf
        })
        .collect();

    // Wrap as outer RLP list
    let payload_len: usize = encoded_vals.iter().map(|v| v.len()).sum();
    let mut out = Vec::with_capacity(payload_len + 5);
    encode_length(payload_len, &mut out);
    for val in &encoded_vals {
        out.extend_from_slice(val);
    }
    out
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
        BorEngine::new(
            test_config(),
            "http://localhost:1317",
            CancellationToken::new(),
        )
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
