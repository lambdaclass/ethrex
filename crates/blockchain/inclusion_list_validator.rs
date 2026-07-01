//! EIP-7805 (FOCIL) inclusion-list satisfaction validator. Tracks per-sender
//! `(nonce, balance)` during block execution and, after execution, decides
//! whether each IL transaction is `present | blob | unrecoverable |
//! intrinsic_gas_too_low | insufficient_gas | below_base_fee | invalid_nonce |
//! invalid_balance | unsatisfied`. Returns `Err(IlUnsatisfied)` if any IL
//! transaction is missing AND could still have been validly appended to the
//! block (mirrors EELS `check_inclusion_list_transactions`).
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
    types::{BlockHeader, ChainConfig, Transaction, TxType},
};
use ethrex_crypto::Crypto;
use ethrex_storage::Store;
use rustc_hash::FxHashMap;

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
        }))
    }
}

/// Tracker of per-sender `(nonce, balance)` for senders appearing in the
/// inclusion list. Built once before block execution from the parent's
/// pre-state, refreshed incrementally during block execution as IL senders'
/// transactions are applied, and consulted once after block execution by
/// [`Self::check`].
///
/// Tracker size is bounded by `|IL senders|` (≤ ~60 in practice, by the 8 KiB
/// IL byte cap), NOT by block transaction count.
#[derive(Debug, Default, Clone)]
pub struct InclusionListSatisfactionValidator {
    pub il_senders: FxHashMap<Address, (u64, U256)>,
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

        let mut il_senders: FxHashMap<Address, (u64, U256)> =
            FxHashMap::with_capacity_and_hasher(unique_senders.len(), Default::default());
        for sender in unique_senders {
            let view = pre_state.get_account(sender)?.unwrap_or_default();
            il_senders.insert(sender, (view.nonce, view.balance));
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
        self.il_senders.insert(sender, (view.nonce, view.balance));
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
            self.il_senders.insert(sender, (view.nonce, view.balance));
        }
        Ok(())
    }

    /// Return `Ok(())` iff every inclusion-list transaction is classified as
    /// non-appendable (`present | blob | unrecoverable | intrinsic_gas_too_low
    /// | insufficient_gas | below_base_fee | invalid_nonce | invalid_balance`).
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
        let base_fee = U256::from(header.base_fee_per_gas.unwrap_or_default());

        for tx_il in il {
            // present in block (anywhere) → satisfied
            if block_txs.contains(&tx_il.hash(crypto)) {
                continue;
            }

            // Blob (EIP-4844) txs are excluded from the IL satisfaction check
            // (EELS skips `BlobTransaction`) → satisfied.
            if tx_il.tx_type() == TxType::EIP4844 {
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

            // below_base_fee → satisfied. Legacy/2930/privileged use
            // `gas_price`; all other types use `max_fee_per_gas`. A typed tx
            // with no recoverable max fee is unpriceable and not includable.
            let max_price = match tx_il.tx_type() {
                TxType::Legacy | TxType::EIP2930 | TxType::Privileged => tx_il.gas_price(),
                _ => match tx_il.max_fee_per_gas() {
                    Some(fee) => U256::from(fee),
                    None => continue,
                },
            };
            if max_price < base_fee {
                continue;
            }

            // From here on, classify by tracked sender state.
            let (tracker_nonce, tracker_balance) = match self.il_senders.get(&sender) {
                Some(entry) => *entry,
                // The sender was not registered at construction. This means
                // the IL handed to `check` differs from the one handed to
                // `new`, which is a caller bug. Be defensive: treat the
                // sender as having empty state, which makes the tx unable
                // to be included (nonce/balance mismatch) and counts as
                // satisfied. This branch is unreachable in normal flow.
                None => (0, U256::zero()),
            };

            // invalid_nonce → satisfied
            if tx_il.nonce() != tracker_nonce {
                continue;
            }

            // invalid_balance → satisfied
            // `cost_without_base_fee` returns `None` only for unsigned/malformed
            // EIP-1559+ txs; treat such txs as `invalid_balance` (cannot be
            // priced) and count them as satisfied.
            let Some(cost) = tx_il.cost_without_base_fee() else {
                continue;
            };
            if cost > tracker_balance {
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
