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
//! ## No EVM
//!
//! The satisfaction check NEVER calls into the EVM. Every classification is a
//! state comparison against the per-sender tracker, exactly per the spec's
//! "No re-execution of IL transactions" requirement.

use std::collections::HashSet;

use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCAK_HASH,
    types::{BlockHeader, ChainConfig, EIP7702_DELEGATED_CODE_LEN, Transaction},
};
use ethrex_crypto::Crypto;
use ethrex_storage::Store;
use rustc_hash::FxHashMap;

use crate::focil_eligibility::{
    SenderCode, VopsProfile, classify, classify_sender_code, fee_valid,
};
use crate::inclusion_list_builder::{AccountStateView, IlStateProvider, IlStateProviderError};
use crate::mempool::transaction_intrinsic_gas;

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

    /// Return `Ok(())` iff every inclusion-list transaction is classified as
    /// non-appendable (`present | blob | frame | unrecoverable |
    /// intrinsic_gas_too_low | insufficient_gas | below_base_fee |
    /// invalid_sender | invalid_nonce | invalid_balance`).
    /// Return `Err(IlUnsatisfied)` for the first IL transaction that is missing
    /// AND could still have been validly appended to the end of the block.
    ///
    /// This mirrors EELS `check_inclusion_list_transactions` +
    /// `check_transaction` (forks/amsterdam/fork.py): for each missing IL tx it
    /// replays exactly the validity gates that block inclusion would apply, and
    /// reports the block as unsatisfied only when every gate passes.
    ///
    /// `block_txs` is the set of transaction hashes included in the block;
    /// position within the block does not matter (per the EIP rationale).
    /// `gas_left` is `block.gas_limit - cumulative_gas_used` post-execution.
    /// `header` and `config` describe the block under check; they supply the
    /// fork (for the intrinsic-gas calculation) and the `base_fee_per_gas`.
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
        for tx_il in il {
            // present in block (anywhere) → satisfied
            if block_txs.contains(&tx_il.hash(crypto)) {
                continue;
            }

            // EIP-8369 decides which omissions are enforceable at all. Anything
            // that is not Profile 1 is excused here:
            //
            // - `Ineligible` covers every blob carrier (blob gas has its own
            //   target and maximum, and EIP-8369 defines no omission check over
            //   that second budget) and any frame transaction that is not a
            //   Profile 2 candidate.
            // - `TwoCandidate` is enforceable in principle, but only after
            //   stateful eligibility replay at the evaluation index. This pass
            //   never calls into the EVM, so it cannot decide that, and an
            //   omission that cannot be shown unjustified is excused.
            let utxo_frames_active = config.is_utxo_frames_activated(header.timestamp);
            if classify(tx_il, utxo_frames_active) != VopsProfile::One {
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
            if !fee_valid(tx_il, header.base_fee_per_gas.unwrap_or_default()) {
                continue;
            }

            // From here on, classify by tracked sender state.
            let entry = match self.il_senders.get(&sender) {
                Some(entry) => *entry,
                // The sender was not registered at construction. This means
                // the IL handed to `check` differs from the one handed to
                // `new`, which is a caller bug. Be defensive: treat the
                // sender as having empty state (an EOA with nonce 0, balance
                // 0), which makes the tx unable to be included (nonce/balance
                // mismatch) and counts as satisfied. This branch is
                // unreachable in normal flow.
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

            // unsatisfied
            return Err(IlUnsatisfied {
                tx_hash: tx_il.hash(crypto),
            });
        }
        Ok(())
    }
}
