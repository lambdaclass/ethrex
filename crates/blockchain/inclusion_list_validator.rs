//! EIP-7805 (FOCIL) inclusion-list satisfaction validator. Tracks per-sender
//! `(nonce, balance, code)` during block execution and, after execution,
//! decides whether each IL transaction is `present | not-Profile-1 |
//! unrecoverable | intrinsic_gas_too_low | insufficient_gas | fee_invalid |
//! invalid_sender | invalid_nonce | invalid_balance | unsatisfied`. Which
//! omissions are enforceable at all is decided by [`crate::focil_eligibility`]
//! per EIP-8369. Returns `Err(IlUnsatisfied)` if any IL transaction is missing
//! AND could still have been validly appended to the block (mirrors EELS
//! `check_inclusion_list_transactions`).
//!
//! ## State abstraction
//!
//! The validator reuses [`IlStateProvider`] / [`AccountStateView`] from
//! [`crate::inclusion_list_builder`]. The IL builder defined the trait first;
//! the validator imports it so there is exactly one trait definition for the
//! Phase 4 engine handler to implement against.
//!
//! ## Sender resolution
//!
//! `Transaction::sender` requires a `&dyn Crypto` to lazily recover the sender
//! from signature material. The validator threads a `&dyn Crypto` through
//! `new` and `observe_executed_tx` (it is not a state read but it is the only
//! crypto surface needed). The Phase 5 caller already has a `NativeCrypto` in
//! scope, so this adds no new dependency at the call site.
//!
//! ## No EVM — except where EIP-8369 requires one
//!
//! Two different specs govern the two VOPS profiles, and each is followed to
//! the letter:
//!
//! - **Profile 1** is governed by EIP-7805's "No re-execution of IL
//!   transactions". [`InclusionListSatisfactionValidator::check`] /
//!   [`InclusionListSatisfactionValidator::check_with_profile_2`]'s Profile 1
//!   path NEVER calls into the EVM: every classification is a state
//!   comparison against the per-sender tracker.
//! - **Profile 2** is governed by EIP-8369, which instead requires replaying
//!   the transaction's validation prefix to decide stateful eligibility. That
//!   replay runs entirely behind the [`IlProfile2Evaluator`] trait — this
//!   module has no `ethrex-vm` dependency and never runs it directly. Its
//!   verdict reaches `unsatisfied` only through [`Profile2Eligibility::Eligible`];
//!   it never influences how a Profile 1 transaction is classified. See
//!   [`crate::focil_profile2`] for the concrete evaluator.

use std::collections::HashSet;

use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCAK_HASH,
    types::{BlockHeader, ChainConfig, EIP7702_DELEGATED_CODE_LEN, FrameTransaction, Transaction},
};
use ethrex_crypto::Crypto;
use ethrex_storage::Store;
use rustc_hash::FxHashMap;

use crate::focil_eligibility::{
    SenderCode, VopsProfile, classify, classify_sender_code, fee_valid, fill_il_budget,
};
use crate::inclusion_list_builder::{AccountStateView, IlStateProvider, IlStateProviderError};
use crate::mempool::transaction_intrinsic_gas;

/// Verdict from an [`IlProfile2Evaluator`] for one omitted EIP-8369 Profile 2
/// (`VopsProfile::TwoCandidate`) inclusion-list transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Profile2Eligibility {
    /// The transaction would have passed EIP-8369 Profile 2 stateful
    /// eligibility replay (recent-root references, payer resolution, and the
    /// AA-VOPS validation-prefix replay) at the evaluation index: its
    /// omission is the kind EIP-8369 makes enforceable.
    Eligible,
    /// A Profile 2 eligibility condition failed (budget, recent-root
    /// reference, payer resolution, or a validation-trace violation such as
    /// [`ethrex_levm::validation_observer::FrameSimViolation::StorageOutsideVopsSurface`]):
    /// the omission is excused.
    Ineligible(String),
    /// Eligibility could not be decided (e.g. an EIP-8312 UTXO frame, which
    /// EIP-8369 does not model, or a VM construction failure). Per the
    /// governing asymmetry, an undecidable omission is excused rather than
    /// risking an unjustified verdict.
    Undecided(String),
}

/// Decides whether an omitted Profile 2 (`VopsProfile::TwoCandidate`)
/// inclusion-list transaction would have passed EIP-8369 stateful
/// eligibility replay.
///
/// Kept as a narrow trait, exactly like [`IlStateProvider`], so the validator
/// gains no dependency on `ethrex-vm` and stays testable against an
/// in-memory fake.
pub trait IlProfile2Evaluator {
    fn evaluate(&self, tx: &FrameTransaction) -> Profile2Eligibility;
}

/// Adapter from `Store` (keyed by state root) to the IL builder/validator's
/// narrow `IlStateProvider` trait. Used by `add_block_pipeline_with_il` to
/// snapshot pre- and post-execution state for the satisfaction check.
pub struct StoreIlStateProvider<'a> {
    pub store: &'a Store,
    pub state_root: H256,
}

impl<'a> IlStateProvider for StoreIlStateProvider<'a> {
    fn get_account(
        &self,
        address: Address,
    ) -> Result<Option<AccountStateView>, IlStateProviderError> {
        let acct = self
            .store
            .get_account_state_by_root(self.state_root, address)
            .map_err(|e| IlStateProviderError::Read(e.to_string()))?;
        Ok(acct.map(|a| AccountStateView {
            nonce: a.nonce,
            balance: a.balance,
            code_hash: a.code_hash,
        }))
    }

    /// Mirrors the EIP-3607 gate at `Blockchain::add_transaction_to_pool`
    /// (mempool admission): a length-based fast path avoids loading the code
    /// body for every contract sender, since only a body of exactly
    /// `EIP7702_DELEGATED_CODE_LEN` bytes can be a delegation indicator.
    fn classify_code(&self, code_hash: H256) -> Result<SenderCode, IlStateProviderError> {
        if code_hash == *EMPTY_KECCAK_HASH {
            return Ok(SenderCode::Eoa);
        }
        let metadata_len = self
            .store
            .get_code_metadata(code_hash)
            .map_err(|e| IlStateProviderError::Read(e.to_string()))?
            .map(|m| m.length);
        if metadata_len != Some(EIP7702_DELEGATED_CODE_LEN as u64) {
            return Ok(SenderCode::Contract);
        }
        let code = self
            .store
            .get_account_code(code_hash)
            .map_err(|e| IlStateProviderError::Read(e.to_string()))?;
        Ok(match code {
            Some(code) => classify_sender_code(code.code()),
            // Metadata claims a delegation-shaped body but the body is
            // missing. Per the governing asymmetry, resolve to the
            // non-originating side (`Contract`) rather than erroring: it
            // excuses the omission instead of risking a wrong punishment.
            None => SenderCode::Contract,
        })
    }
}

/// A tracked IL sender's `(nonce, balance, code)` as of the last refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IlSenderState {
    pub nonce: u64,
    pub balance: U256,
    pub code: SenderCode,
}

/// Tracker of per-sender `(nonce, balance, code)` for senders appearing in
/// the inclusion list. Built once before block execution from the parent's
/// pre-state, refreshed incrementally during block execution as IL senders'
/// transactions are applied, and consulted once after block execution by
/// [`Self::check`].
///
/// Tracker size is bounded by `|IL senders|` (≤ ~60 in practice, by the 8 KiB
/// IL byte cap), NOT by block transaction count.
#[derive(Debug, Default, Clone)]
pub struct InclusionListSatisfactionValidator {
    pub il_senders: FxHashMap<Address, IlSenderState>,
}

/// Resolve `(nonce, balance, code)` from an account snapshot. Short-circuits
/// the empty-code hash to `SenderCode::Eoa` without a `classify_code` call.
/// A `classify_code` error is mapped to `SenderCode::Unknown` and never
/// propagated: per the governing asymmetry, a code-read failure must not
/// abort the check nor turn a justified omission into an unjustified one.
///
/// This is the only place `SenderCode` is derived for the tracker, so
/// `new`, `observe_executed_tx`, and `refresh_all_from` all route through it
/// — a mid-block delegation set or cleared on an IL sender is reflected the
/// same way regardless of which of the three touched it.
fn resolve(view: &AccountStateView, state: &dyn IlStateProvider) -> IlSenderState {
    let code = if view.code_hash == *EMPTY_KECCAK_HASH {
        SenderCode::Eoa
    } else {
        state
            .classify_code(view.code_hash)
            .unwrap_or(SenderCode::Unknown)
    };
    IlSenderState {
        nonce: view.nonce,
        balance: view.balance,
        code,
    }
}

/// Errors returned by the validator surface itself (separate from
/// [`IlUnsatisfied`], which signals a satisfied/unsatisfied verdict).
///
/// Sender-recovery failures are NOT errors here: per EELS
/// `check_inclusion_list_transactions`, an IL transaction whose sender cannot
/// be recovered can never be validly appended, so it is silently skipped
/// (counts as satisfied) rather than aborting the whole check.
#[derive(Debug, thiserror::Error)]
pub enum IlValidatorError {
    #[error("state read error during IL validator construction: {0}")]
    State(#[from] IlStateProviderError),
}

/// Verdict from [`InclusionListSatisfactionValidator::check`] when the IL is
/// not satisfied. Carries the offending transaction's hash for local
/// debugging/tracing — per spec, the engine API translates this into
/// `{status: INCLUSION_LIST_UNSATISFIED, latestValidHash: null,
/// validationError: null}` and does NOT echo the hash on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IlUnsatisfied {
    pub tx_hash: H256,
}

impl std::fmt::Display for IlUnsatisfied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "inclusion list unsatisfied: tx 0x{:x} omitted with sender retaining nonce/balance/gas",
            self.tx_hash
        )
    }
}

impl std::error::Error for IlUnsatisfied {}

/// Result of [`InclusionListSatisfactionValidator::check_with_profile_2`].
///
/// `unsatisfied` is the consensus verdict, reached by either profile: Profile 1
/// by state comparison, Profile 2 by stateful eligibility replay. It holds the
/// first offending transaction found, in list order.
///
/// The two `Vec`s record every Profile 2 outcome, including those that did not
/// reach a verdict, so an operator can see what the replay decided and why
/// without inferring it from the single verdict.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IlCheckReport {
    /// The Profile 1 verdict, exactly as [`InclusionListSatisfactionValidator::check`]
    /// would have produced it.
    pub unsatisfied: Option<IlUnsatisfied>,
    /// Omitted Profile 2 candidates an [`IlProfile2Evaluator`] classified
    /// [`Profile2Eligibility::Eligible`]: omissions EIP-8369 makes enforceable.
    /// The first also sets `unsatisfied`.
    pub profile_2_unjustified: Vec<H256>,
    /// Omitted Profile 2 candidates an [`IlProfile2Evaluator`] classified
    /// [`Profile2Eligibility::Undecided`].
    pub profile_2_undecided: Vec<H256>,
}

impl InclusionListSatisfactionValidator {
    /// Build the per-sender tracker from the unique senders in `il`. A read
    /// of `Ok(None)` is treated as an empty account (nonce 0, balance 0) per
    /// the [`IlStateProvider`] contract. A state read error is propagated; the
    /// caller (engine handler) maps it to the internal-error JSON-RPC code.
    /// Sender-recovery failures are silently skipped (see the type-level doc).
    pub fn new(
        il: &[Transaction],
        pre_state: &dyn IlStateProvider,
        crypto: &dyn Crypto,
    ) -> Result<Self, IlValidatorError> {
        // Dedupe senders so we issue at most one state read per sender. An IL
        // transaction whose signature does not recover a sender can never be
        // validly appended (EELS `recover_sender` raises → skipped), so we do
        // not register it and do not propagate the recovery failure.
        let mut unique_senders: HashSet<Address> = HashSet::with_capacity(il.len());
        for tx in il {
            if let Ok(sender) = tx.sender(crypto) {
                unique_senders.insert(sender);
            }
        }

        let mut il_senders: FxHashMap<Address, IlSenderState> =
            FxHashMap::with_capacity_and_hasher(unique_senders.len(), Default::default());
        for sender in unique_senders {
            let view = pre_state.get_account(sender)?.unwrap_or_default();
            il_senders.insert(sender, resolve(&view, pre_state));
        }

        Ok(Self { il_senders })
    }

    /// Refresh the tracked `(nonce, balance)` for `executed.sender()` if the
    /// sender appears in the IL set. Senders not in the IL set are a no-op,
    /// keeping the per-executed-tx overhead at one HashMap lookup.
    pub fn observe_executed_tx(
        &mut self,
        executed: &Transaction,
        post_state: &dyn IlStateProvider,
        crypto: &dyn Crypto,
    ) -> Result<(), IlValidatorError> {
        let Ok(sender) = executed.sender(crypto) else {
            // Unrecoverable sender cannot be an IL sender we track.
            return Ok(());
        };
        if !self.il_senders.contains_key(&sender) {
            return Ok(());
        }
        let view = post_state.get_account(sender)?.unwrap_or_default();
        self.il_senders.insert(sender, resolve(&view, post_state));
        Ok(())
    }

    /// Refresh every tracked sender's `(nonce, balance)` from `state`.
    /// Equivalent to calling `observe_executed_tx` for every block tx that
    /// touched an IL sender, but cheaper when the post-state is already
    /// available — reads exactly `|IL senders|` entries from `state`.
    ///
    /// Used by `add_block_pipeline_with_il` after the block has been imported
    /// and the post-state trie is committed.
    pub fn refresh_all_from(
        &mut self,
        state: &dyn IlStateProvider,
        _crypto: &dyn Crypto,
    ) -> Result<(), IlValidatorError> {
        // Collect addresses first to avoid borrow-checker conflict (we
        // mutate `self.il_senders` while iterating).
        let senders: Vec<Address> = self.il_senders.keys().copied().collect();
        for sender in senders {
            let view = state.get_account(sender)?.unwrap_or_default();
            self.il_senders.insert(sender, resolve(&view, state));
        }
        Ok(())
    }

    /// `Ok(())` wrapper over [`Self::check_with_profile_2`] with no Profile 2
    /// evaluator: the Profile 1 path — the only one that decides the returned
    /// verdict — is byte-identical to a version of this method that never
    /// mentions Profile 2.
    ///
    /// This method MUST NOT call into the EVM. It is a pure state-comparison
    /// pass over the per-sender tracker plus stateless transaction validity
    /// gates (intrinsic gas, base fee, signature recoverability).
    pub fn check(
        &self,
        il: &[Transaction],
        block_txs: &HashSet<H256>,
        gas_left: u64,
        header: &BlockHeader,
        config: &ChainConfig,
        crypto: &dyn Crypto,
    ) -> Result<(), IlUnsatisfied> {
        self.check_with_profile_2(il, block_txs, gas_left, header, config, crypto, None)
            .unsatisfied
            .map_or(Ok(()), Err)
    }

    /// The Profile 1 satisfaction pass, extended with an observational EIP-8369
    /// Profile 2 pass over the same inclusion list.
    ///
    /// `unsatisfied` is `Ok(())`/`Err` iff every inclusion-list transaction is
    /// classified as non-appendable (`present | blob |
    /// unrecoverable | intrinsic_gas_too_low | insufficient_gas |
    /// below_base_fee | invalid_sender | invalid_nonce | invalid_balance`),
    ///
    /// Frame transactions are NOT on that list. They used to be excused
    /// wholesale, on the grounds that the generic pass compares the declared
    /// sender's nonce and balance and so cannot judge a keyed, payer-funded
    /// transaction. That reasoning still holds for the generic pass — which is
    /// why type `0x06` must never reach it — but it is no longer a reason to
    /// excuse the omission: an EIP-8369 Profile 2 candidate is judged by
    /// replaying its validation prefix through an [`IlProfile2Evaluator`], and
    /// only an `Eligible` verdict makes the list unsatisfied.
    ///
    /// for the first IL transaction that is missing AND could still have been
    /// validly appended to the end of the block. This mirrors EELS
    /// `check_inclusion_list_transactions` + `check_transaction`
    /// (forks/amsterdam/fork.py): for each missing IL tx it replays exactly
    /// the validity gates that block inclusion would apply, and reports the
    /// block as unsatisfied only when every gate passes. `unsatisfied` alone
    /// is the consensus verdict; see [`IlCheckReport`] for what the rest of
    /// the report is (and is not) for.
    ///
    /// `profile_2`, when present, is consulted for every omitted Profile 2
    /// candidate (`VopsProfile::TwoCandidate`) whose EIP-8369 static budget
    /// fill admitted it and whose fee would have been valid against `header`
    /// — the same two conditions EIP-8369 requires before a candidate's
    /// eligibility is even worth replaying. `fill_il_budget` runs once, before
    /// the loop, and its outcomes are indexed positionally against `il`,
    /// exactly mirroring EIP-8369's own ordered, single-pass budget fill.
    ///
    /// `block_txs` is the set of transaction hashes included in the block;
    /// position within the block does not matter (per the EIP rationale).
    /// `gas_left` is `block.gas_limit - cumulative_gas_used` post-execution.
    /// `header` and `config` describe the block under check; they supply the
    /// fork (for the intrinsic-gas calculation) and the `base_fee_per_gas`.
    #[allow(clippy::too_many_arguments)]
    pub fn check_with_profile_2(
        &self,
        il: &[Transaction],
        block_txs: &HashSet<H256>,
        gas_left: u64,
        header: &BlockHeader,
        config: &ChainConfig,
        crypto: &dyn Crypto,
        profile_2: Option<&dyn IlProfile2Evaluator>,
    ) -> IlCheckReport {
        let utxo_frames_active = config.is_utxo_frames_activated(header.timestamp);
        let base_fee = header.base_fee_per_gas.unwrap_or_default();
        let fill_outcomes = fill_il_budget(
            il,
            utxo_frames_active,
            config.fork(header.timestamp),
            crypto,
        );

        let mut report = IlCheckReport::default();

        for (tx_il, fill_outcome) in il.iter().zip(fill_outcomes.iter()) {
            // present in block (anywhere) → satisfied
            if block_txs.contains(&tx_il.hash(crypto)) {
                continue;
            }

            let profile = classify(tx_il, utxo_frames_active);

            // EIP-8369 Profile 2. A transaction that passes stateful eligibility
            // replay at the evaluation index could have been included, so its
            // omission is unjustified and the list is unsatisfied — the same
            // verdict Profile 1 reaches by state comparison.
            //
            // Only `Eligible` gets there. `Ineligible` is an excused omission,
            // and `Undecided` is one this pass cannot decide, which the
            // governing asymmetry also excuses: a wrong `unsatisfied` makes the
            // consensus layer withhold an attestation from an honest block.
            if profile == VopsProfile::TwoCandidate {
                if let Some(evaluator) = profile_2
                    && fill_outcome.is_admitted()
                    && fee_valid(tx_il, base_fee)
                    && let Transaction::FrameTransaction(frame_tx) = tx_il
                {
                    match evaluator.evaluate(frame_tx) {
                        Profile2Eligibility::Eligible => {
                            let tx_hash = tx_il.hash(crypto);
                            report.profile_2_unjustified.push(tx_hash);
                            report.unsatisfied.get_or_insert(IlUnsatisfied { tx_hash });
                        }
                        Profile2Eligibility::Undecided(_) => {
                            report.profile_2_undecided.push(tx_il.hash(crypto));
                        }
                        Profile2Eligibility::Ineligible(_) => {}
                    }
                }
                continue;
            }

            // EIP-8369 decides which omissions are enforceable at all.
            // `Ineligible` covers every blob carrier (blob gas has its own
            // target and maximum, and EIP-8369 defines no omission check over
            // that second budget) and everything else outside both profiles.
            if profile != VopsProfile::One {
                continue;
            }

            // Unrecoverable signature → cannot be appended (EELS
            // `recover_sender` raises) → satisfied.
            let Ok(sender) = tx_il.sender(crypto) else {
                continue;
            };

            // intrinsic_gas_too_low → satisfied. A tx whose gas limit is below
            // its intrinsic cost can never be validly included (EELS
            // `validate_transaction`). A pricing/overflow error here likewise
            // means the tx is not includable, so we treat it as satisfied.
            match transaction_intrinsic_gas(tx_il, sender, header, config) {
                Ok(intrinsic) if tx_il.gas_limit() < intrinsic => continue,
                Err(_) => continue,
                Ok(_) => {}
            }

            // insufficient_gas → satisfied
            if tx_il.gas_limit() > gas_left {
                continue;
            }

            // EIP-8369 `fee_valid(tx, block)`: below base fee, or a priority fee
            // above the max fee, means the transaction could never be validly
            // appended → satisfied. The priority-versus-max condition is part of
            // the rule and not implied by the base-fee comparison.
            if !fee_valid(tx_il, base_fee) {
                continue;
            }

            // From here on, classify by tracked sender state.
            let entry = match self.il_senders.get(&sender) {
                Some(entry) => *entry,
                // The sender was not registered at construction. This means
                // the IL handed to `check_with_profile_2` differs from the one
                // handed to `new`, which is a caller bug. Be defensive: treat
                // the sender as having empty state (an EOA with nonce 0,
                // balance 0), which makes the tx unable to be included
                // (nonce/balance mismatch) and counts as satisfied. This
                // branch is unreachable in normal flow.
                None => IlSenderState {
                    nonce: 0,
                    balance: U256::zero(),
                    code: SenderCode::Eoa,
                },
            };

            // invalid_sender → satisfied. EIP-8369 Profile 1 sender validity
            // requires the sender to satisfy EIP-3607, with EIP-7702
            // delegation indicators treated as the valid delegated EOA case:
            // "EOAs with empty code and EOAs with a valid EIP-7702 delegation
            // indicator can originate transactions; accounts with any other
            // code cannot." `SenderCode::Unknown` (classification
            // unavailable) deliberately lands here too, per the governing
            // asymmetry: an unclassifiable sender must never make an
            // omission look unjustified.
            if !entry.code.can_originate() {
                continue;
            }

            // invalid_nonce → satisfied
            if tx_il.nonce() != entry.nonce {
                continue;
            }

            // invalid_balance → satisfied
            // `cost_without_base_fee` returns `None` only for unsigned/malformed
            // EIP-1559+ txs; treat such txs as `invalid_balance` (cannot be
            // priced) and count them as satisfied.
            let Some(cost) = tx_il.cost_without_base_fee() else {
                continue;
            };
            if cost > entry.balance {
                continue;
            }

            // unsatisfied. Keep the FIRST one found (matches the short-circuit
            // `check` used before Profile 2 observation existed); keep
            // scanning the rest of `il` regardless, so later Profile 2
            // candidates are still observed.
            if report.unsatisfied.is_none() {
                report.unsatisfied = Some(IlUnsatisfied {
                    tx_hash: tx_il.hash(crypto),
                });
            }
        }
        report
    }
}
