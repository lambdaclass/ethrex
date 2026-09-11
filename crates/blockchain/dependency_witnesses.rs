//! The dependencies this node has verified for itself.
//!
//! EIP-8288 splits a dependency from its proof: a transaction declares
//! `(scheme, data_hash, verification_key_hash)` and the proof travels separately, in
//! a mempool wrapper. That split is the point -- it is what lets one recursive STARK
//! discharge a whole block's worth of signatures -- but it leaves a node holding
//! transactions whose claims it has not checked.
//!
//! Two things go wrong if nothing tracks what has been checked:
//!
//! - **Admission.** A transaction whose approval rests on a declared signature can
//!   pass validation-prefix simulation without the node ever establishing that the
//!   signature exists. The declaration is free to write.
//! - **Block production.** A builder that includes such a transaction must publish a
//!   proof discharging it, and cannot produce one without the witness. The block
//!   then fails its own rule 2 on import.
//!
//! This store is the answer to both: a dependency is admitted only once its witness
//! has been verified, and the builder aggregates from the same material. A node that
//! has verified nothing admits nothing, which is the safe direction.

use std::sync::{Arc, Mutex};

use ethrex_common::types::DependencyTriple;
use ethrex_dep_aggregation::{AggregateError, DependencyAggregator, DependencyWitness};
use rustc_hash::FxHashMap;
use tracing::warn;

/// Verified witnesses, keyed by the dependency each one discharges.
///
/// Bounded, because entries arrive from the network: a peer that could add without
/// limit could exhaust memory for the price of signing. Eviction is arbitrary rather
/// than LRU -- a dropped entry costs an admission, not correctness, so the ordering
/// is not worth the bookkeeping.
#[derive(Debug)]
pub struct DependencyWitnessStore {
    inner: Mutex<FxHashMap<DependencyTriple, Arc<Vec<u8>>>>,
    capacity: usize,
}

/// Room for a few hundred blocks' worth of a busy pool at the EIP's per-transaction
/// limit, which is far more than the wrapper limits can actually deliver.
pub const DEFAULT_WITNESS_CAPACITY: usize = 4096;

impl Default for DependencyWitnessStore {
    fn default() -> Self {
        // Not a derive: a derived `Default` gives capacity zero, and a store that can
        // hold nothing makes every dependency permanently inadmissible without
        // anything reporting a problem.
        Self::new(DEFAULT_WITNESS_CAPACITY)
    }
}

impl DependencyWitnessStore {
    /// `capacity` is clamped to at least one, for the reason in [`Default`].
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(FxHashMap::default()),
            capacity: capacity.max(1),
        }
    }

    /// Verify `witness` and keep it if it checks out.
    ///
    /// Verification happens here rather than at the call site so that nothing can
    /// put an unchecked witness in: the store's whole value is that membership means
    /// "this node established this claim". `Ok` therefore means the witness was
    /// verified **and** stored, and the only error is the verification failing.
    pub fn insert_verified(
        &self,
        aggregator: &dyn DependencyAggregator,
        witness: &DependencyWitness,
    ) -> Result<(), AggregateError> {
        aggregator.verify_witness(witness)?;

        let mut map = self.lock();
        // Evict before inserting a new key, never after, so the bound is never
        // briefly exceeded. Re-inserting a key already present displaces nothing.
        if !map.contains_key(&witness.triple) {
            while map.len() >= self.capacity {
                let Some(victim) = map.keys().next().copied() else {
                    break;
                };
                map.remove(&victim);
            }
        }
        map.insert(witness.triple, Arc::new(witness.witness.clone()));
        Ok(())
    }

    /// Whether this node has verified the witness for `triple`.
    pub fn holds(&self, triple: &DependencyTriple) -> bool {
        self.lock().contains_key(triple)
    }

    /// The witnesses for `triples`, in the order asked for.
    ///
    /// `None` if any is missing: a builder that aggregated over a partial set would
    /// publish a proof covering less than the block declares, which fails rule 2 just
    /// as surely as publishing nothing.
    pub fn witnesses_for(&self, triples: &[DependencyTriple]) -> Option<Vec<DependencyWitness>> {
        let map = self.lock();
        triples
            .iter()
            .map(|triple| {
                map.get(triple).map(|witness| DependencyWitness {
                    triple: *triple,
                    witness: witness.as_ref().clone(),
                })
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Take the lock, recovering if a previous holder panicked.
    ///
    /// Recovery is right here, not merely convenient: the guarded value is a plain
    /// map of verified witnesses with no invariant spanning two operations, so a
    /// panic cannot leave it half-updated in a way that matters. Treating poisoning
    /// as fatal would instead make one unrelated panic silently refuse every
    /// dependency for the rest of the process's life.
    fn lock(&self) -> std::sync::MutexGuard<'_, FxHashMap<DependencyTriple, Arc<Vec<u8>>>> {
        self.inner.lock().unwrap_or_else(|poisoned| {
            warn!("dependency witness store lock was poisoned by a panicking holder; recovering");
            poisoned.into_inner()
        })
    }
}
