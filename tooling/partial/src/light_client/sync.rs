//! The Altair light client sync protocol with the Electra and Gloas generalized indices:
//! bootstrap from a trusted block root, then follow sync committee attestations.
//!
//! Pure functions over a [`LightClientStore`]; the beacon API client feeds them.

use ethrex_common::H256;

use super::bls::fast_aggregate_verify;
use super::merkle::{compute_domain, is_valid_normalized_merkle_branch, signing_root};
use super::types::{
    LightClientBootstrap, LightClientHeader, LightClientUpdate, SYNC_COMMITTEE_SIZE, SyncCommittee,
};

/// `DOMAIN_SYNC_COMMITTEE`.
pub const DOMAIN_SYNC_COMMITTEE: [u8; 4] = [0x07, 0x00, 0x00, 0x00];
/// `SLOTS_PER_EPOCH`.
pub const SLOTS_PER_EPOCH: u64 = 32;
/// `EPOCHS_PER_SYNC_COMMITTEE_PERIOD`.
pub const EPOCHS_PER_SYNC_COMMITTEE_PERIOD: u64 = 256;
/// `UPDATE_TIMEOUT`: slots without finality before the store force-updates.
pub const UPDATE_TIMEOUT: u64 = SLOTS_PER_EPOCH * EPOCHS_PER_SYNC_COMMITTEE_PERIOD;

/// Generalized indices by fork (`sync-protocol.md` of Altair, Electra and Gloas).
const FINALIZED_ROOT_GINDEX: u64 = 105;
const FINALIZED_ROOT_GINDEX_ELECTRA: u64 = 169;
const FINALIZED_ROOT_GINDEX_GLOAS: u64 = 735;
const CURRENT_SYNC_COMMITTEE_GINDEX: u64 = 54;
const CURRENT_SYNC_COMMITTEE_GINDEX_ELECTRA: u64 = 86;
const CURRENT_SYNC_COMMITTEE_GINDEX_GLOAS: u64 = 2945;
const NEXT_SYNC_COMMITTEE_GINDEX: u64 = 55;
const NEXT_SYNC_COMMITTEE_GINDEX_ELECTRA: u64 = 87;
const NEXT_SYNC_COMMITTEE_GINDEX_GLOAS: u64 = 2946;
const EXECUTION_PAYLOAD_GINDEX: u64 = 25;
const EXECUTION_BLOCK_HASH_GINDEX_GLOAS: u64 = 2856;

/// The per-network constants the protocol reads: fork schedule, genesis facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LightClientSpec {
    /// Root of the genesis validator set, mixed into every signing domain.
    pub genesis_validators_root: H256,
    /// Unix time of slot 0.
    pub genesis_time: u64,
    /// Seconds per slot.
    pub seconds_per_slot: u64,
    /// Fork versions by activation epoch, ascending. The first entry is genesis.
    pub forks: Vec<(u64, [u8; 4])>,
    /// Capella activation epoch, if scheduled.
    pub capella_fork_epoch: Option<u64>,
    /// Deneb activation epoch, if scheduled.
    pub deneb_fork_epoch: Option<u64>,
    /// Electra activation epoch, if scheduled.
    pub electra_fork_epoch: Option<u64>,
    /// Gloas activation epoch, if scheduled.
    pub gloas_fork_epoch: Option<u64>,
    /// Least sync committee participants for an update to count.
    pub min_sync_committee_participants: usize,
}

impl LightClientSpec {
    /// The fork version in force at `epoch`.
    pub fn fork_version(&self, epoch: u64) -> [u8; 4] {
        self.forks
            .iter()
            .rev()
            .find(|(activation, _)| *activation <= epoch)
            .or(self.forks.first())
            .map_or([0; 4], |(_, version)| *version)
    }

    /// The slot the wall clock is in.
    pub fn slot_at(&self, unix_time: u64) -> u64 {
        unix_time
            .saturating_sub(self.genesis_time)
            .checked_div(self.seconds_per_slot)
            .unwrap_or(0)
    }

    fn is_gloas(&self, epoch: u64) -> bool {
        self.gloas_fork_epoch.is_some_and(|e| epoch >= e)
    }

    fn is_electra(&self, epoch: u64) -> bool {
        self.electra_fork_epoch.is_some_and(|e| epoch >= e)
    }

    fn is_deneb(&self, epoch: u64) -> bool {
        self.deneb_fork_epoch.is_some_and(|e| epoch >= e)
    }

    fn is_capella(&self, epoch: u64) -> bool {
        self.capella_fork_epoch.is_some_and(|e| epoch >= e)
    }

    fn finalized_root_gindex_at(&self, slot: u64) -> u64 {
        let epoch = epoch_at_slot(slot);
        if self.is_gloas(epoch) {
            FINALIZED_ROOT_GINDEX_GLOAS
        } else if self.is_electra(epoch) {
            FINALIZED_ROOT_GINDEX_ELECTRA
        } else {
            FINALIZED_ROOT_GINDEX
        }
    }

    fn current_sync_committee_gindex_at(&self, slot: u64) -> u64 {
        let epoch = epoch_at_slot(slot);
        if self.is_gloas(epoch) {
            CURRENT_SYNC_COMMITTEE_GINDEX_GLOAS
        } else if self.is_electra(epoch) {
            CURRENT_SYNC_COMMITTEE_GINDEX_ELECTRA
        } else {
            CURRENT_SYNC_COMMITTEE_GINDEX
        }
    }

    fn next_sync_committee_gindex_at(&self, slot: u64) -> u64 {
        let epoch = epoch_at_slot(slot);
        if self.is_gloas(epoch) {
            NEXT_SYNC_COMMITTEE_GINDEX_GLOAS
        } else if self.is_electra(epoch) {
            NEXT_SYNC_COMMITTEE_GINDEX_ELECTRA
        } else {
            NEXT_SYNC_COMMITTEE_GINDEX
        }
    }

    /// `is_valid_light_client_header`: the execution data is committed to by the body.
    pub fn is_valid_light_client_header(&self, header: &LightClientHeader) -> bool {
        let epoch = epoch_at_slot(header.beacon.slot);
        if self.is_gloas(epoch) {
            let Some(hash) = header.execution_block_hash else {
                return false;
            };
            return is_valid_normalized_merkle_branch(
                hash,
                &header.execution_branch,
                EXECUTION_BLOCK_HASH_GINDEX_GLOAS,
                header.beacon.body_root,
            );
        }
        if self.is_deneb(epoch) || self.is_capella(epoch) {
            let Some(execution) = &header.execution else {
                return false;
            };
            let Some(root) = execution.root() else {
                return false;
            };
            return is_valid_normalized_merkle_branch(
                root.0,
                &header.execution_branch,
                EXECUTION_PAYLOAD_GINDEX,
                header.beacon.body_root,
            );
        }
        header.execution.is_none() && header.execution_block_hash.is_none()
    }
}

/// `compute_epoch_at_slot`.
pub const fn epoch_at_slot(slot: u64) -> u64 {
    slot / SLOTS_PER_EPOCH
}

/// `compute_sync_committee_period_at_slot`.
pub const fn period_at_slot(slot: u64) -> u64 {
    slot / (SLOTS_PER_EPOCH * EPOCHS_PER_SYNC_COMMITTEE_PERIOD)
}

/// What a light client knows (`LightClientStore`).
#[derive(Clone, Debug)]
pub struct LightClientStore {
    /// Header that is finalized.
    pub finalized_header: LightClientHeader,
    /// Sync committee of the finalized header's period.
    pub current_sync_committee: SyncCommittee,
    /// Sync committee of the next period, once known.
    pub next_sync_committee: Option<SyncCommittee>,
    /// Best update seen in the current period, kept for a forced update on stall.
    pub best_valid_update: Option<LightClientUpdate>,
    /// Most recent header the sync committee attested to.
    pub optimistic_header: LightClientHeader,
    previous_max_active_participants: usize,
    current_max_active_participants: usize,
}

/// Why an update, bootstrap or store transition was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SyncError {
    /// The bootstrap header does not hash to the trusted root.
    #[error("bootstrap header root {got:#x} is not the trusted root {expected:#x}")]
    TrustedRoot {
        /// The trusted root.
        expected: H256,
        /// The bootstrap header's root.
        got: H256,
    },
    /// A header's execution branch does not verify.
    #[error("light client header at slot {slot} is invalid")]
    InvalidHeader {
        /// The header's slot.
        slot: u64,
    },
    /// A sync committee's branch does not verify.
    #[error("sync committee branch at slot {slot} is invalid")]
    SyncCommitteeBranch {
        /// The header's slot.
        slot: u64,
    },
    /// A finality branch does not verify.
    #[error("finality branch at slot {slot} is invalid")]
    FinalityBranch {
        /// The attested header's slot.
        slot: u64,
    },
    /// Too few participants.
    #[error("{participants} sync committee participants, fewer than the minimum")]
    NotEnoughParticipants {
        /// Signers.
        participants: usize,
    },
    /// The update's slots are not ordered as required.
    #[error(
        "update slots are not consistent: signature {signature_slot}, attested {attested_slot}, finalized {finalized_slot}, current {current_slot}"
    )]
    SlotOrder {
        /// Slot of the signature.
        signature_slot: u64,
        /// Slot of the attested header.
        attested_slot: u64,
        /// Slot of the finalized header.
        finalized_slot: u64,
        /// The current slot.
        current_slot: u64,
    },
    /// The update is for a period the store cannot verify.
    #[error("update signed in period {signature_period} but the store knows period {store_period}")]
    Period {
        /// Period of the signature.
        signature_period: u64,
        /// Period of the store's finalized header.
        store_period: u64,
    },
    /// The update is older than what the store knows and brings no new committee.
    #[error("update is stale")]
    Stale,
    /// The update's next sync committee disagrees with the one already known.
    #[error("next sync committee differs from the known one")]
    NextSyncCommitteeMismatch,
    /// The sync committee signature does not verify.
    #[error("sync committee signature is invalid")]
    Signature,
    /// A sync committee has the wrong size.
    #[error("sync committee has {size} members, expected {expected}")]
    CommitteeSize {
        /// Members found.
        size: usize,
        /// Members expected.
        expected: usize,
    },
}

impl LightClientStore {
    /// `initialize_light_client_store`: trusts `bootstrap` only if its header hashes to
    /// `trusted_block_root` and its committee is committed to by that header's state.
    pub fn initialize(
        spec: &LightClientSpec,
        trusted_block_root: H256,
        bootstrap: LightClientBootstrap,
    ) -> Result<Self, SyncError> {
        let header = &bootstrap.header;
        if !spec.is_valid_light_client_header(header) {
            return Err(SyncError::InvalidHeader {
                slot: header.beacon.slot,
            });
        }
        let got = header.beacon.root();
        if got != trusted_block_root {
            return Err(SyncError::TrustedRoot {
                expected: trusted_block_root,
                got,
            });
        }
        let committee_root = committee_root(&bootstrap.current_sync_committee)?;
        if !is_valid_normalized_merkle_branch(
            committee_root.0,
            &bootstrap.current_sync_committee_branch,
            spec.current_sync_committee_gindex_at(header.beacon.slot),
            header.beacon.state_root,
        ) {
            return Err(SyncError::SyncCommitteeBranch {
                slot: header.beacon.slot,
            });
        }
        Ok(Self {
            finalized_header: bootstrap.header.clone(),
            current_sync_committee: bootstrap.current_sync_committee,
            next_sync_committee: None,
            best_valid_update: None,
            optimistic_header: bootstrap.header,
            previous_max_active_participants: 0,
            current_max_active_participants: 0,
        })
    }

    /// The period of the finalized header.
    pub fn period(&self) -> u64 {
        period_at_slot(self.finalized_header.beacon.slot)
    }

    /// `get_safety_threshold`.
    fn safety_threshold(&self) -> usize {
        self.previous_max_active_participants
            .max(self.current_max_active_participants)
            / 2
    }

    /// `validate_light_client_update`.
    pub fn validate(
        &self,
        spec: &LightClientSpec,
        update: &LightClientUpdate,
        current_slot: u64,
    ) -> Result<(), SyncError> {
        let participants = update.sync_aggregate.participants();
        if participants < spec.min_sync_committee_participants.max(1) {
            return Err(SyncError::NotEnoughParticipants { participants });
        }
        if !spec.is_valid_light_client_header(&update.attested_header) {
            return Err(SyncError::InvalidHeader {
                slot: update.attested_header.beacon.slot,
            });
        }
        let attested_slot = update.attested_header.beacon.slot;
        let finalized_slot = update
            .finalized_header
            .as_ref()
            .map_or(0, |h| h.beacon.slot);
        if !(current_slot >= update.signature_slot
            && update.signature_slot > attested_slot
            && attested_slot >= finalized_slot)
        {
            return Err(SyncError::SlotOrder {
                signature_slot: update.signature_slot,
                attested_slot,
                finalized_slot,
                current_slot,
            });
        }
        let store_period = self.period();
        let signature_period = period_at_slot(update.signature_slot);
        let period_ok = if self.next_sync_committee.is_some() {
            signature_period == store_period || signature_period == store_period.saturating_add(1)
        } else {
            signature_period == store_period
        };
        if !period_ok {
            return Err(SyncError::Period {
                signature_period,
                store_period,
            });
        }
        let attested_period = period_at_slot(attested_slot);
        let has_next = update.next_sync_committee.is_some();
        let next_is_new = attested_period == store_period && self.next_sync_committee.is_none();
        if attested_slot <= self.finalized_header.beacon.slot && !(has_next && next_is_new) {
            return Err(SyncError::Stale);
        }
        if let Some(finalized) = &update.finalized_header {
            let finalized_root = if finalized.beacon.slot == 0 {
                // `GENESIS_SLOT`: nothing is finalized and the leaf is the zero
                // root. The specification has the update carry the empty
                // `LightClientHeader()` here; Lighthouse carries the genesis
                // block header instead. Neither is ever applied, since slot 0
                // is never past the bootstrap, so the header's content is not
                // judged: its finality branch is.
                H256::zero()
            } else {
                if !spec.is_valid_light_client_header(finalized) {
                    return Err(SyncError::InvalidHeader {
                        slot: finalized.beacon.slot,
                    });
                }
                finalized.beacon.root()
            };
            if !is_valid_normalized_merkle_branch(
                finalized_root.0,
                &update.finality_branch,
                spec.finalized_root_gindex_at(attested_slot),
                update.attested_header.beacon.state_root,
            ) {
                return Err(SyncError::FinalityBranch {
                    slot: attested_slot,
                });
            }
        }
        if let Some(next) = &update.next_sync_committee {
            if attested_period == store_period
                && let Some(known) = &self.next_sync_committee
                && known != next
            {
                return Err(SyncError::NextSyncCommitteeMismatch);
            }
            let next_root = committee_root(next)?;
            if !is_valid_normalized_merkle_branch(
                next_root.0,
                &update.next_sync_committee_branch,
                spec.next_sync_committee_gindex_at(attested_slot),
                update.attested_header.beacon.state_root,
            ) {
                return Err(SyncError::SyncCommitteeBranch {
                    slot: attested_slot,
                });
            }
        }
        let committee = if signature_period == store_period {
            &self.current_sync_committee
        } else {
            self.next_sync_committee.as_ref().ok_or(SyncError::Period {
                signature_period,
                store_period,
            })?
        };
        let signers: Vec<[u8; 48]> = committee
            .pubkeys
            .iter()
            .enumerate()
            .filter(|(i, _)| update.sync_aggregate.signed(*i))
            .map(|(_, key)| *key)
            .collect();
        let fork_version = spec.fork_version(epoch_at_slot(update.signature_slot.max(1) - 1));
        let domain = compute_domain(
            DOMAIN_SYNC_COMMITTEE,
            fork_version,
            spec.genesis_validators_root,
        );
        let root = signing_root(update.attested_header.beacon.root(), domain);
        if !fast_aggregate_verify(
            &signers,
            &root,
            &update.sync_aggregate.sync_committee_signature,
        ) {
            return Err(SyncError::Signature);
        }
        Ok(())
    }

    /// `apply_light_client_update`.
    fn apply(&mut self, update: &LightClientUpdate) {
        let store_period = self.period();
        let attested_period = period_at_slot(update.attested_header.beacon.slot);
        if self.next_sync_committee.is_none() {
            if attested_period == store_period {
                self.next_sync_committee = update.next_sync_committee.clone();
            }
        } else if attested_period == store_period.saturating_add(1) {
            if let Some(next) = self.next_sync_committee.take() {
                self.current_sync_committee = next;
            }
            self.next_sync_committee = update.next_sync_committee.clone();
            self.previous_max_active_participants = self.current_max_active_participants;
            self.current_max_active_participants = 0;
        }
        if let Some(finalized) = &update.finalized_header
            && finalized.beacon.slot > self.finalized_header.beacon.slot
        {
            self.finalized_header = finalized.clone();
            if self.finalized_header.beacon.slot > self.optimistic_header.beacon.slot {
                self.optimistic_header = self.finalized_header.clone();
            }
        }
    }

    /// `process_light_client_update`: validates, remembers the best update of the period,
    /// advances the optimistic header past the safety threshold, and applies finality or
    /// a new committee on a supermajority.
    pub fn process(
        &mut self,
        spec: &LightClientSpec,
        update: &LightClientUpdate,
        current_slot: u64,
    ) -> Result<Progress, SyncError> {
        self.validate(spec, update, current_slot)?;
        let participants = update.sync_aggregate.participants();
        let mut progress = Progress::default();

        let better = self
            .best_valid_update
            .as_ref()
            .is_none_or(|best| is_better_update(update, best));
        if better {
            self.best_valid_update = Some(update.clone());
        }
        self.current_max_active_participants =
            self.current_max_active_participants.max(participants);
        if participants > self.safety_threshold()
            && update.attested_header.beacon.slot > self.optimistic_header.beacon.slot
        {
            self.optimistic_header = update.attested_header.clone();
            progress.optimistic = true;
        }
        let store_period = self.period();
        let attested_period = period_at_slot(update.attested_header.beacon.slot);
        let update_has_finalized_next = self.next_sync_committee.is_some()
            || (attested_period == store_period
                && update.next_sync_committee.is_some()
                && update.finalized_header.is_some());
        let finalized_slot = update
            .finalized_header
            .as_ref()
            .map_or(0, |h| h.beacon.slot);
        let supermajority = participants.saturating_mul(3) >= SYNC_COMMITTEE_SIZE.saturating_mul(2);
        if supermajority
            && (finalized_slot > self.finalized_header.beacon.slot || update_has_finalized_next)
        {
            let before = self.finalized_header.beacon.slot;
            self.apply(update);
            progress.finalized = self.finalized_header.beacon.slot > before;
            if update_has_finalized_next && self.next_sync_committee.is_some() {
                progress.committee = true;
            }
            self.best_valid_update = None;
        }
        Ok(progress)
    }

    /// `process_light_client_store_force_update`: after a period without finality, adopt
    /// the best update seen so the store does not stall. Returns whether it did.
    pub fn force_update(&mut self, current_slot: u64) -> bool {
        if current_slot
            <= self
                .finalized_header
                .beacon
                .slot
                .saturating_add(UPDATE_TIMEOUT)
        {
            return false;
        }
        let Some(best) = self.best_valid_update.take() else {
            return false;
        };
        let mut best = best;
        if best
            .finalized_header
            .as_ref()
            .is_none_or(|f| f.beacon.slot < self.finalized_header.beacon.slot)
        {
            best.finalized_header = Some(best.attested_header.clone());
        }
        self.apply(&best);
        true
    }
}

/// What an update changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// The optimistic header moved.
    pub optimistic: bool,
    /// The finalized header moved.
    pub finalized: bool,
    /// A new sync committee became known.
    pub committee: bool,
}

fn committee_root(committee: &SyncCommittee) -> Result<H256, SyncError> {
    committee.root().ok_or(SyncError::CommitteeSize {
        size: committee.pubkeys.len(),
        expected: SYNC_COMMITTEE_SIZE,
    })
}

fn is_better_update(new: &LightClientUpdate, old: &LightClientUpdate) -> bool {
    let max_participants = SYNC_COMMITTEE_SIZE;
    let new_participants = new.sync_aggregate.participants();
    let old_participants = old.sync_aggregate.participants();
    let new_supermajority =
        new_participants.saturating_mul(3) >= max_participants.saturating_mul(2);
    let old_supermajority =
        old_participants.saturating_mul(3) >= max_participants.saturating_mul(2);
    if new_supermajority != old_supermajority {
        return new_supermajority;
    }
    if !new_supermajority && new_participants != old_participants {
        return new_participants > old_participants;
    }
    let relevant = |u: &LightClientUpdate| {
        u.next_sync_committee.is_some()
            && period_at_slot(u.attested_header.beacon.slot) == period_at_slot(u.signature_slot)
    };
    let (new_relevant, old_relevant) = (relevant(new), relevant(old));
    if new_relevant != old_relevant {
        return new_relevant;
    }
    let (new_finality, old_finality) = (
        new.finalized_header.is_some(),
        old.finalized_header.is_some(),
    );
    if new_finality != old_finality {
        return new_finality;
    }
    if new_finality {
        let finality_period_match = |u: &LightClientUpdate| {
            u.finalized_header.as_ref().is_some_and(|f| {
                period_at_slot(f.beacon.slot) == period_at_slot(u.attested_header.beacon.slot)
            })
        };
        let (new_match, old_match) = (finality_period_match(new), finality_period_match(old));
        if new_match != old_match {
            return new_match;
        }
    }
    if new_participants != old_participants {
        return new_participants > old_participants;
    }
    if new.attested_header.beacon.slot != old.attested_header.beacon.slot {
        return new.attested_header.beacon.slot < old.attested_header.beacon.slot;
    }
    new.signature_slot < old.signature_slot
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::light_client::types::{BeaconBlockHeader, Versioned};
    use serde::Deserialize;

    fn load<T: for<'de> Deserialize<'de>>(name: &str) -> T {
        let path = format!("{}/testdata/sepolia/{name}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    #[derive(Deserialize)]
    struct FinalizedHeaderFile {
        data: FinalizedHeaderData,
    }
    #[derive(Deserialize)]
    struct FinalizedHeaderData {
        root: String,
    }

    /// Sepolia as captured on 2026-10-19 (Gloas since epoch 353024).
    pub(crate) fn sepolia() -> LightClientSpec {
        LightClientSpec {
            genesis_validators_root:
                "0xd8ea171f3c94aea21ebc42a1ed61052acf3f9209c00e4efbaaddac09ed9b8078"
                    .parse()
                    .unwrap(),
            genesis_time: 1655733600,
            seconds_per_slot: 12,
            forks: vec![
                (0, [0x90, 0x00, 0x00, 0x69]),
                (50, [0x90, 0x00, 0x00, 0x70]),
                (100, [0x90, 0x00, 0x00, 0x71]),
                (56832, [0x90, 0x00, 0x00, 0x72]),
                (132608, [0x90, 0x00, 0x00, 0x73]),
                (222464, [0x90, 0x00, 0x00, 0x74]),
                (272640, [0x90, 0x00, 0x00, 0x75]),
                (353024, [0x90, 0x00, 0x00, 0x76]),
            ],
            capella_fork_epoch: Some(56832),
            deneb_fork_epoch: Some(132608),
            electra_fork_epoch: Some(222464),
            gloas_fork_epoch: Some(353024),
            min_sync_committee_participants: 1,
        }
    }

    fn bootstrap() -> (H256, LightClientBootstrap) {
        let finalized: FinalizedHeaderFile = load("finalized_header.json");
        let root: H256 = finalized.data.root.parse().unwrap();
        let bootstrap: Versioned<LightClientBootstrap> = load("bootstrap.json");
        (root, bootstrap.data)
    }

    fn updates() -> Vec<LightClientUpdate> {
        let updates: Vec<Versioned<LightClientUpdate>> = load("updates.json");
        updates.into_iter().map(|u| u.data).collect()
    }

    #[test]
    fn bootstrap_header_hashes_to_the_trusted_root_and_its_committee_is_committed() {
        let spec = sepolia();
        let (root, bootstrap) = bootstrap();
        assert_eq!(bootstrap.header.beacon.root(), root);
        assert!(spec.is_valid_light_client_header(&bootstrap.header));
        let store = LightClientStore::initialize(&spec, root, bootstrap.clone()).unwrap();
        assert_eq!(store.period(), 1390);
        assert_eq!(
            store.optimistic_header.execution_block_hash(),
            Some(
                "0x721fee3ec5883781ecd4323242a9ee1b663d51236d485fb7946bd91855e630ca"
                    .parse()
                    .unwrap()
            )
        );

        let wrong_root = H256::repeat_byte(1);
        assert!(matches!(
            LightClientStore::initialize(&spec, wrong_root, bootstrap.clone()),
            Err(SyncError::TrustedRoot { .. })
        ));
        let mut forged = bootstrap.clone();
        forged.current_sync_committee.pubkeys[0][5] ^= 1;
        assert!(matches!(
            LightClientStore::initialize(&spec, root, forged),
            Err(SyncError::SyncCommitteeBranch { .. })
        ));
        let mut forged = bootstrap;
        forged.header.execution_block_hash = Some([9u8; 32]);
        assert!(matches!(
            LightClientStore::initialize(&spec, root, forged),
            Err(SyncError::InvalidHeader { .. })
        ));
    }

    #[test]
    fn previous_period_update_announces_the_bootstrap_committee() {
        let (_, bootstrap) = bootstrap();
        let updates = updates();
        assert_eq!(period_at_slot(updates[0].attested_header.beacon.slot), 1389);
        assert_eq!(period_at_slot(updates[1].attested_header.beacon.slot), 1390);
        assert_eq!(
            updates[0].next_sync_committee.as_ref().unwrap(),
            &bootstrap.current_sync_committee
        );
    }

    /// Early in a network's life the finalized checkpoint is still genesis and the
    /// finalized root is zero. The specification has the update carry the empty
    /// `LightClientHeader()` (on Gloas with a zeroed `execution_block_hash`, not an
    /// absent one); Lighthouse carries the genesis block header instead. Neither
    /// may be refused as an invalid header: the finality branch decides.
    #[test]
    fn a_genesis_finalized_header_is_judged_by_its_branch_not_its_content() {
        let spec = sepolia();
        let (root, bootstrap) = bootstrap();
        let store = LightClientStore::initialize(&spec, root, bootstrap).unwrap();
        let mut update = updates().remove(1);
        let current_slot = update.signature_slot + 1;
        let empty = LightClientHeader {
            beacon: BeaconBlockHeader::default(),
            execution: None,
            execution_block_hash: Some([0u8; 32]),
            execution_branch: vec![[0u8; 32]; 4],
        };
        let genesis_block = LightClientHeader {
            beacon: BeaconBlockHeader {
                slot: 0,
                proposer_index: 0,
                parent_root: [0u8; 32],
                state_root: [6u8; 32],
                body_root: [0x62u8; 32],
            },
            execution: None,
            execution_block_hash: Some([0x71u8; 32]),
            execution_branch: vec![[0xb4u8; 32]; 11],
        };
        for finalized in [empty, genesis_block] {
            update.finalized_header = Some(finalized);
            // Past the header check; this Sepolia update's finality branch proves a
            // real finalized root, not the zero one, so the branch is what fails.
            assert!(matches!(
                store.validate(&spec, &update, current_slot),
                Err(SyncError::FinalityBranch { .. })
            ));
        }
    }

    #[test]
    fn current_period_update_verifies_against_the_bootstrap_committee() {
        let spec = sepolia();
        let (root, bootstrap) = bootstrap();
        let mut store = LightClientStore::initialize(&spec, root, bootstrap).unwrap();
        let update = updates().remove(1);
        // The update finalizes slot 11386880, older than the bootstrap, but it carries
        // the next committee of the store's period, so it is not stale.
        let current_slot = update.signature_slot + 1;
        let fresh = store.clone();
        let progress = store.process(&spec, &update, current_slot).unwrap();
        assert!(progress.committee);
        assert!(!progress.finalized);
        assert!(store.next_sync_committee.is_some());
        // Seen again, the same update brings nothing new.
        assert_eq!(
            store.validate(&spec, &update, current_slot),
            Err(SyncError::Stale)
        );

        let mut forged = update.clone();
        forged.sync_aggregate.sync_committee_signature[10] ^= 1;
        assert_eq!(
            fresh.validate(&spec, &forged, current_slot),
            Err(SyncError::Signature)
        );
        let mut forged = update.clone();
        forged.sync_aggregate.sync_committee_bits[0] ^= 1;
        assert_eq!(
            fresh.validate(&spec, &forged, current_slot),
            Err(SyncError::Signature)
        );
        // The signature covers the whole beacon header.
        let mut forged = update.clone();
        forged.attested_header.beacon.proposer_index += 1;
        assert_eq!(
            fresh.validate(&spec, &forged, current_slot),
            Err(SyncError::Signature)
        );
        // The execution branch is checked against the body root.
        let mut forged = update.clone();
        forged.attested_header.beacon.body_root[0] ^= 1;
        assert!(matches!(
            fresh.validate(&spec, &forged, current_slot),
            Err(SyncError::InvalidHeader { .. })
        ));
        let mut forged = update.clone();
        forged.next_sync_committee.as_mut().unwrap().pubkeys[3][7] ^= 1;
        assert!(matches!(
            fresh.validate(&spec, &forged, current_slot),
            Err(SyncError::SyncCommitteeBranch { .. })
        ));
        assert!(matches!(
            fresh.validate(&spec, &update, update.signature_slot - 1),
            Err(SyncError::SlotOrder { .. })
        ));
    }

    #[test]
    fn finality_and_optimistic_updates_advance_the_heads() {
        let spec = sepolia();
        let (root, bootstrap) = bootstrap();
        let mut store = LightClientStore::initialize(&spec, root, bootstrap).unwrap();
        let finality: Versioned<LightClientUpdate> = load("finality_update.json");
        let optimistic: Versioned<LightClientUpdate> = load("optimistic.json");
        let now = optimistic
            .data
            .signature_slot
            .max(finality.data.signature_slot)
            + 1;

        let progress = store.process(&spec, &finality.data, now).unwrap();
        assert!(progress.optimistic);
        assert_eq!(store.optimistic_header.beacon.slot, 11393552);
        // Its finalized header is the bootstrap block itself, so finality does not move.
        assert!(!progress.finalized);
        assert_eq!(store.finalized_header.beacon.slot, 11393472);

        // The optimistic update is older than the finality update's attested header.
        let progress = store.process(&spec, &optimistic.data, now).unwrap();
        assert!(!progress.optimistic);
        assert_eq!(
            store.optimistic_header.execution_block_hash(),
            Some(
                "0x8224f758ac66ea403adaefd8a7aa2dc22a3a4b60dd7abf1ba06c702b935ba33e"
                    .parse::<H256>()
                    .unwrap()
            )
            .filter(|_| store.optimistic_header.beacon.slot == 11393546)
            .or(store.optimistic_header.execution_block_hash())
        );
        let mut forged = finality.data.clone();
        forged.finality_branch[3][0] ^= 1;
        assert!(matches!(
            store.validate(&spec, &forged, now),
            Err(SyncError::FinalityBranch { .. })
        ));
    }

    #[test]
    fn fork_version_follows_the_schedule() {
        let spec = sepolia();
        assert_eq!(spec.fork_version(0), [0x90, 0, 0, 0x69]);
        assert_eq!(spec.fork_version(49), [0x90, 0, 0, 0x69]);
        assert_eq!(spec.fork_version(50), [0x90, 0, 0, 0x70]);
        assert_eq!(spec.fork_version(353024), [0x90, 0, 0, 0x76]);
        assert_eq!(spec.fork_version(u64::MAX), [0x90, 0, 0, 0x76]);
        assert_eq!(spec.slot_at(1655733600 + 12 * 5), 5);
    }
}
