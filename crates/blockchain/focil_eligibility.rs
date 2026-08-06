//! EIP-8369 (VOPS Profiles for FOCIL Eligibility) classification.
//!
//! Decides which inclusion-list transactions FOCIL ([EIP-7805]) enforces
//! omission for. Two profiles:
//!
//! - **Profile 1**, base VOPS: legacy / EIP-2930 / EIP-1559 / EIP-7702
//!   transactions without blobs. Keeps EIP-7805's end-of-payload omission check
//!   and consumes no VERIFY budget.
//! - **Profile 2**, FOCIL AA-VOPS: EIP-8141 frame transactions whose validation
//!   stays inside a fixed state surface, judged at a builder-claimed index.
//!
//! A transaction in neither profile may still be listed and included, but its
//! omission is excused.
//!
//! Everything here is static: it reads the transaction and the block header, and
//! never touches state or the EVM. Stateful eligibility is decided later, during
//! replay at the evaluation index, because it depends on that index.
//!
//! ## Eligibility is not mempool admission
//!
//! EIP-8369: "Public mempool admission and FOCIL eligibility remain separate",
//! and "the sets differ in both directions". So nothing here may consult a
//! node's own admission policy or its operator-tunable knobs. The rules must be
//! self-contained and computed identically by every client, or the omission
//! verdict splits between nodes.

use ethrex_common::Address;
use ethrex_common::types::{FrameTransaction, Transaction, TxType};

/// EIP-8369 `MAX_VERIFY_GAS_PER_TX`: the largest VERIFY budget a single
/// Profile 2 candidate may declare.
///
/// Deliberately higher than EIP-8141's `MAX_VERIFY_GAS = 100_000` public mempool
/// cap, "because FOCIL eligibility and public mempool admission are separate
/// policies". This is a consensus-relevant classification input, so it is a
/// constant here and must never be read from node configuration.
pub const MAX_VERIFY_GAS_PER_TX: u64 = 1 << 20;

/// EIP-8369 `MAX_VERIFY_GAS_PER_IL`: the VERIFY budget one inclusion list may
/// consume across all of its Profile 2 occurrences.
pub const MAX_VERIFY_GAS_PER_IL: u64 = 1 << 20;

/// Which VOPS profile a transaction falls into, for FOCIL enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VopsProfile {
    /// Base VOPS. Omission is judged by EIP-7805's end-of-payload rule.
    One,
    /// FOCIL AA-VOPS. Omission is judged at a builder-claimed index, and only
    /// after the transaction also passes stateful eligibility there.
    TwoCandidate,
    /// Outside FOCIL enforcement; omission is always excused.
    Ineligible,
}

/// EIP-8369 `fee_valid(tx, block)`: `max_fee_per_gas >= block.base_fee_per_gas`
/// and `max_priority_fee_per_gas <= max_fee_per_gas`. For legacy and EIP-2930
/// transactions `gas_price` stands in for both fields, which makes the second
/// condition trivially true.
///
/// This is the whole of the fee check. EIP-8369 carries no closed-form solvency
/// formula: payer capacity is decided by replay under full `APPROVE` semantics,
/// including maximum-cost collection, so a second closed-form copy of that rule
/// would only generate divergence.
pub fn fee_valid(tx: &Transaction, base_fee_per_gas: u64) -> bool {
    match tx.tx_type() {
        TxType::Legacy | TxType::EIP2930 | TxType::Privileged => {
            tx.gas_price() >= base_fee_per_gas.into()
        }
        _ => {
            let Some(max_fee) = tx.max_fee_per_gas() else {
                return false;
            };
            let priority = tx.max_priority_fee().unwrap_or_default();
            max_fee >= base_fee_per_gas && priority <= max_fee
        }
    }
}

/// The VERIFY budget cost of a frame transaction, per EIP-8369: "the signature
/// verification gas counted by EIP-8141 plus the declared gas limits of every
/// frame in the validation prefix".
///
/// Returns `None` when the cost cannot be computed, which for the budget fill
/// means the occurrence is ignored and consumes nothing.
///
/// An expiry verifier frame "is ignored only when matching the four allowed
/// prefix shapes; its gas limit still counts". `ValidationPrefix::frame_indices`
/// deliberately excludes expiry frames, so summing it alone would undercount;
/// they are added back here.
pub fn verify_budget_cost(tx: &FrameTransaction) -> Option<u64> {
    let prefix = tx.validation_prefix().ok()?;

    let prefix_gas = prefix
        .frame_indices
        .iter()
        .map(|&i| tx.frames.get(i).map_or(0, |f| f.gas_limit))
        .fold(0u64, |acc, g| acc.saturating_add(g));

    let expiry_gas = tx
        .frames
        .iter()
        .filter(|f| f.is_expiry_verifier())
        .map(|f| f.gas_limit)
        .fold(0u64, |acc, g| acc.saturating_add(g));

    Some(
        prefix_gas
            .saturating_add(expiry_gas)
            .saturating_add(tx.signature_verification_cost()),
    )
}

/// Classify a transaction into its VOPS profile from the transaction alone.
///
/// `utxo_frames_active` is `ChainConfig::is_utxo_frames_activated` for the block
/// being judged; it is threaded through to EIP-8141 static validation and gates
/// only whether EIP-8312 UTXO frames are admissible.
///
/// Shape decides the candidate profile: non-frame transactions without blobs are
/// Profile 1 candidates, frame transactions with empty `blob_versioned_hashes`
/// are Profile 2 candidates. Anything carrying blobs is outside both, because
/// blob gas has its own target and maximum and EIP-8369 defines no omission
/// check over that second budget.
pub fn classify(tx: &Transaction, utxo_frames_active: bool) -> VopsProfile {
    if tx.tx_type() == TxType::EIP4844 || !tx.blob_versioned_hashes().is_empty() {
        return VopsProfile::Ineligible;
    }

    match tx {
        Transaction::FrameTransaction(frame_tx) => {
            if is_profile_2_candidate(frame_tx, utxo_frames_active) {
                VopsProfile::TwoCandidate
            } else {
                VopsProfile::Ineligible
            }
        }
        Transaction::LegacyTransaction(_)
        | Transaction::EIP2930Transaction(_)
        | Transaction::EIP1559Transaction(_)
        | Transaction::EIP7702Transaction(_) => VopsProfile::One,
        // Privileged (L2) transactions are not part of either profile.
        _ => VopsProfile::Ineligible,
    }
}

/// The three static Profile 2 candidate conditions, "true from the transaction
/// alone":
///
/// 1. a statically valid EIP-8141 frame transaction;
/// 2. its prefix matches one of the four recognized shapes, with any expiry
///    verifier frame unique and first;
/// 3. its VERIFY budget cost is at most [`MAX_VERIFY_GAS_PER_TX`].
///
/// A self-funded EIP-8312 UTXO spend matches no recognized prefix and so is not
/// a candidate, which is the intended outcome.
pub fn is_profile_2_candidate(tx: &FrameTransaction, utxo_frames_active: bool) -> bool {
    if tx.validate_static_constraints(utxo_frames_active).is_err() {
        return false;
    }

    let Ok(prefix) = tx.validation_prefix() else {
        return false;
    };

    // Structural conformance only. The budget is checked separately below
    // against the FOCIL constant, so `u64::MAX` disables the caller-supplied
    // mempool budget inside this call rather than letting it decide eligibility.
    if tx.validate_prefix_structure(&prefix, u64::MAX).is_err() {
        return false;
    }

    verify_budget_cost(tx).is_some_and(|cost| cost <= MAX_VERIFY_GAS_PER_TX)
}

/// The `payer` a Profile 2 candidate will establish, resolved from the prefix
/// shape alone.
///
/// EIP-8369: "`payer` is `sender` for the `self_verify` shapes and the `pay`
/// frame's EIP-8141 `resolved_target` otherwise. A null `pay` target resolves to
/// `sender`."
///
/// Static resolution is what makes the Profile 2 storage surface knowable before
/// replay begins, which is why the surface can be enforced from the first opcode
/// rather than discovered part-way through.
///
/// Returns `None` when the transaction has no recognized prefix.
pub fn profile_2_payer(tx: &FrameTransaction) -> Option<Address> {
    let prefix = tx.validation_prefix().ok()?;
    match prefix.pay_index {
        Some(idx) => Some(
            tx.frames
                .get(idx)
                .and_then(|f| f.target)
                .unwrap_or(tx.sender),
        ),
        None => Some(tx.sender),
    }
}

/// The evaluation index to judge an omitted Profile 2 candidate at, when the
/// builder supplies no usable claim.
///
/// EIP-8369 lets a builder commit an index in `[0, len(block.transactions)]`, and
/// pins the fallback: "A missing, malformed, or out-of-range index defaults to
/// `len(block.transactions)`, the end of the payload." That default is not a
/// stub. For a position-stable transaction, one whose validation dependencies
/// are constant within the payload or change monotonically, invalidity at any
/// index persists to the end, so judging at the end is equivalent to EIP-7805's
/// existing rule.
///
/// No claimed index can reach the execution layer yet: EIP-7805 defines no
/// beacon-block field and the Engine API has no parameter, and the EIP-8369
/// extension that would define the encoding does not exist. Until it does, every
/// omission is judged here.
pub fn default_evaluation_index(block_tx_count: usize) -> usize {
    block_tx_count
}

/// Clamp a builder-claimed index to the range EIP-8369 allows, falling back to
/// [`default_evaluation_index`] when it is out of range.
///
/// Kept separate from the default so that wiring a claim in later is a change at
/// the call site rather than a change to the rule.
pub fn evaluation_index(claimed: Option<usize>, block_tx_count: usize) -> usize {
    match claimed {
        Some(idx) if idx <= block_tx_count => idx,
        _ => default_evaluation_index(block_tx_count),
    }
}

/// Outcome of the per-inclusion-list static budget fill for one occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillOutcome {
    /// Not a Profile 2 occurrence; Profile 1 transactions do not use the budget.
    NotMetered,
    /// Cost could not be computed, exceeded the per-transaction cap, or did not
    /// fit the remaining budget. Consumes nothing.
    Ignored,
    /// Charged against the list budget and admitted.
    Admitted { cost: u64 },
    /// Charged against the list budget but rejected by a candidate check that
    /// runs after the debit. EIP-8369: "A failed candidate keeps the budget
    /// debit but is not admitted."
    ChargedNotAdmitted { cost: u64 },
}

impl FillOutcome {
    /// Whether this occurrence is enforceable, i.e. it was admitted by the fill.
    pub fn is_admitted(&self) -> bool {
        matches!(self, FillOutcome::Admitted { .. })
    }
}

/// Run EIP-8369's static budget fill over one inclusion list, in list order and
/// per occurrence, before any deduplication.
///
/// The ordering is the point of the rule and is what bounds signature work:
/// compute the cost from decoding and shape checks alone, and only then, if it
/// fits, deduct it *before* verifying any cryptographic signature. A structurally
/// valid transaction with an invalid signature therefore pays for the attempt
/// instead of forcing free verification work.
///
/// Returns one outcome per input transaction, positionally.
pub fn fill_il_budget(il: &[Transaction], utxo_frames_active: bool) -> Vec<FillOutcome> {
    let mut remaining = MAX_VERIFY_GAS_PER_IL;
    let mut outcomes = Vec::with_capacity(il.len());

    for tx in il {
        let Transaction::FrameTransaction(frame_tx) = tx else {
            outcomes.push(FillOutcome::NotMetered);
            continue;
        };
        if !frame_tx.blob_versioned_hashes.is_empty() {
            outcomes.push(FillOutcome::NotMetered);
            continue;
        }

        // Decoding and structural shape checks only, enough to price the
        // occurrence. No signature verification has happened yet.
        let Some(cost) = verify_budget_cost(frame_tx) else {
            outcomes.push(FillOutcome::Ignored);
            continue;
        };
        if cost > MAX_VERIFY_GAS_PER_TX || cost > remaining {
            outcomes.push(FillOutcome::Ignored);
            continue;
        }

        remaining -= cost;

        if is_profile_2_candidate(frame_tx, utxo_frames_active) {
            outcomes.push(FillOutcome::Admitted { cost });
        } else {
            outcomes.push(FillOutcome::ChargedNotAdmitted { cost });
        }
    }

    outcomes
}
