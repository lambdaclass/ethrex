//! EIP-7805 (FOCIL) inclusion-list satisfaction validator. Tracks per-sender
//! `(nonce, balance, code)` during block execution and, after execution,
//! replays for each missing IL transaction the exact validity gates that block
//! inclusion would apply. Returns `Err(IlUnsatisfied)` if any IL transaction
//! is missing AND could still have been validly appended to the block
//! (mirrors EELS `check_inclusion_list_transactions` + `validate_transaction`
//! + `check_transaction`, forks/amsterdam, as of tests-focil-devnet@v0.2.0).
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
    Address, Bytes, H256, U256,
    constants::{EMPTY_KECCAK_HASH, GAS_PER_BLOB, TX_MAX_GAS_LIMIT_AMSTERDAM},
    types::{
        BlockHeader, ChainConfig, GWEI_TO_WEI, MAX_BLOB_COUNT, Transaction, TxType,
        VERSIONED_HASH_VERSION_KZG, Withdrawal, calculate_base_fee_per_blob_gas,
        is_eip7702_delegation,
    },
};
use ethrex_crypto::Crypto;
use ethrex_storage::Store;
use rustc_hash::FxHashMap;

use crate::constants::{AMSTERDAM_MAX_INITCODE_SIZE, MAX_INITCODE_SIZE};
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

    fn get_code(&self, address: Address) -> Result<Option<Bytes>, IlStateProviderError> {
        let Some(acct) = self
            .store
            .get_account_state_by_root(self.state_root, address)
            .map_err(|e| IlStateProviderError::Read(e.to_string()))?
        else {
            return Ok(None);
        };
        if acct.code_hash == *EMPTY_KECCAK_HASH {
            return Ok(None);
        }
        let code = self
            .store
            .get_account_code(acct.code_hash)
            .map_err(|e| IlStateProviderError::Read(e.to_string()))?
            .ok_or_else(|| {
                // The account names a code hash the store cannot resolve: a DB
                // inconsistency, not an empty account. Surfacing it beats
                // misclassifying the sender as code-free.
                IlStateProviderError::Read(format!(
                    "code missing for hash {:?} despite non-empty code_hash",
                    acct.code_hash
                ))
            })?;
        Ok(Some(code.code_bytes()))
    }
}

/// Post-state snapshot of one inclusion-list sender, as consulted by
/// [`InclusionListSatisfactionValidator::check`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TrackedSender {
    pub nonce: u64,
    pub balance: U256,
    /// The sender's contract code (`None` when it has none), for the
    /// EIP-3607/EIP-7702 sender-is-EOA gate.
    pub code: Option<Bytes>,
}

/// Tracker of per-sender [`TrackedSender`] state for senders appearing in the
/// inclusion list. Built once before block execution from the parent's
/// pre-state, refreshed incrementally during block execution as IL senders'
/// transactions are applied, and consulted once after block execution by
/// [`Self::check`].
///
/// Tracker size is bounded by `|IL senders|` (≤ ~60 in practice, by the 8 KiB
/// IL byte cap), NOT by block transaction count.
#[derive(Debug, Default, Clone)]
pub struct InclusionListSatisfactionValidator {
    pub il_senders: FxHashMap<Address, TrackedSender>,
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

        let mut il_senders: FxHashMap<Address, TrackedSender> =
            FxHashMap::with_capacity_and_hasher(unique_senders.len(), Default::default());
        for sender in unique_senders {
            let view = pre_state.get_account(sender)?.unwrap_or_default();
            let code = pre_state.get_code(sender)?;
            il_senders.insert(
                sender,
                TrackedSender {
                    nonce: view.nonce,
                    balance: view.balance,
                    code,
                },
            );
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
        let code = post_state.get_code(sender)?;
        self.il_senders.insert(
            sender,
            TrackedSender {
                nonce: view.nonce,
                balance: view.balance,
                code,
            },
        );
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
            let code = state.get_code(sender)?;
            self.il_senders.insert(
                sender,
                TrackedSender {
                    nonce: view.nonce,
                    balance: view.balance,
                    code,
                },
            );
        }
        Ok(())
    }

    /// Roll tracked balances back to the pre-withdrawals point of the block.
    ///
    /// EELS `apply_body` runs `check_inclusion_list_transactions` after the
    /// block's transactions but BEFORE `process_withdrawals`, so a sender
    /// funded only by a same-block withdrawal is NOT includable at check time.
    /// The tracker is refreshed from the block's final state root, which
    /// already includes those credits; withdrawals only ever add balance
    /// (never touching nonce or code), so subtracting the block's credits per
    /// sender reconstructs the pre-withdrawals balance exactly. `saturating_`
    /// arithmetic is defensive only: a final balance below the block's own
    /// credits to that account cannot occur.
    pub fn discount_withdrawals(&mut self, withdrawals: &[Withdrawal]) {
        for withdrawal in withdrawals {
            if withdrawal.amount == 0 {
                continue;
            }
            if let Some(tracked) = self.il_senders.get_mut(&withdrawal.address) {
                let credit = U256::from(withdrawal.amount).saturating_mul(U256::from(GWEI_TO_WEI));
                tracked.balance = tracked.balance.saturating_sub(credit);
            }
        }
    }

    /// Return `Ok(())` iff every inclusion-list transaction is classified as
    /// non-appendable; return `Err(IlUnsatisfied)` for the first IL
    /// transaction that is missing AND could still have been validly appended
    /// to the end of the block.
    ///
    /// This mirrors EELS `check_inclusion_list_transactions` →
    /// `validate_transaction` → `check_transaction` (forks/amsterdam, as of
    /// tests-focil-devnet@v0.2.0): for each missing IL tx it replays exactly
    /// the validity gates that block inclusion would apply, and reports the
    /// block as unsatisfied only when every gate passes. Gate order can differ
    /// from EELS (each failing gate raises there); the satisfied/unsatisfied
    /// verdict is order-independent.
    ///
    /// `block_txs` is the set of transaction hashes included in the block;
    /// position within the block does not matter (per the EIP rationale).
    /// `gas_left` is `block.gas_limit - header.gas_used` post-execution.
    /// `header` and `config` describe the block under check; they supply the
    /// fork (for the intrinsic-gas calculation), the `base_fee_per_gas`, and
    /// the blob-gas parameters.
    ///
    /// This method MUST NOT call into the EVM. It is a pure state-comparison
    /// pass over the per-sender tracker plus stateless transaction validity
    /// gates (intrinsic gas, fees, signature recoverability).
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
        // EELS `check_block_gas_capacity`: the blob dimension has its own
        // budget, `MAX_BLOB_GAS_PER_BLOCK - block_output.blob_gas_used`. The
        // per-block maximum comes from the blob schedule in force at the
        // block's timestamp; a Hegotá chain without one is a config anomaly,
        // and 0 keeps every blob tx classified as not includable.
        let blob_gas_left = config
            .get_fork_blob_schedule(header.timestamp)
            .map(|schedule| u64::from(schedule.max) * u64::from(GAS_PER_BLOB))
            .unwrap_or_default()
            .saturating_sub(header.blob_gas_used.unwrap_or_default());
        // EIP-7954: Amsterdam raises the init-code cap (same selection as
        // mempool admission).
        let max_initcode_size = if config.is_amsterdam_activated(header.timestamp) {
            AMSTERDAM_MAX_INITCODE_SIZE
        } else {
            MAX_INITCODE_SIZE
        } as usize;

        for tx_il in il {
            // present in block (anywhere) → satisfied
            if block_txs.contains(&tx_il.hash(crypto)) {
                continue;
            }

            // wrong chain id → satisfied. EELS `check_inclusion_list_transactions`
            // excuses a tx whose declared chain id differs from the chain's;
            // pre-EIP-155 legacy txs declare none and skip the gate.
            if let Some(tx_chain_id) = tx_il.chain_id()
                && tx_chain_id != config.chain_id
            {
                continue;
            }

            // Unrecoverable signature → cannot be appended (EELS
            // `recover_sender` raises) → satisfied.
            let Ok(sender) = tx_il.sender(crypto) else {
                continue;
            };

            // EIP-2681 nonce overflow → satisfied (EELS `validate_transaction`
            // rejects `nonce >= 2**64 - 1`; larger values already fail the
            // canonical decode upstream, so only the exact maximum reaches
            // this pass).
            if tx_il.nonce() == u64::MAX {
                continue;
            }

            // Contract creation with oversized init code → satisfied (EELS
            // `validate_transaction` raises `InitCodeTooLargeError`).
            if tx_il.is_contract_creation() && tx_il.data().len() > max_initcode_size {
                continue;
            }

            // Priority fee above the fee cap → satisfied (EELS
            // `validate_transaction` raises `PriorityFeeGreaterThanMaxFeeError`
            // for fee-market transactions).
            if let (Some(max_priority), Some(max_fee)) =
                (tx_il.max_priority_fee(), tx_il.max_fee_per_gas())
                && max_priority > max_fee
            {
                continue;
            }

            // Blob (EIP-4844) txs run through the same gates as any other type
            // since tests-focil-devnet@v0.2.0 ("Type-3 transactions were
            // considered not-includable by default" spec fix), plus the
            // blob-specific ones from EELS `validate_transaction`:
            // no blob at all, more blobs than a tx may carry, or a versioned
            // hash that is not KZG-versioned → satisfied. (A blob or set-code
            // creation, `TransactionTypeContractCreationError` in EELS, cannot
            // be represented here: `to` is a plain `Address`, so such a tx
            // already failed the canonical decode and was excused.)
            if tx_il.tx_type() == TxType::EIP4844 {
                let blob_hashes = tx_il.blob_versioned_hashes();
                if blob_hashes.is_empty() || blob_hashes.len() > MAX_BLOB_COUNT {
                    continue;
                }
                if blob_hashes
                    .iter()
                    .any(|hash| hash.as_bytes()[0] != VERSIONED_HASH_VERSION_KZG)
                {
                    continue;
                }
            }

            // EIP-7702 set-code tx with an empty authorization list →
            // satisfied (EELS `validate_transaction` raises
            // `EmptyAuthorizationListError`).
            if tx_il
                .authorization_list()
                .is_some_and(|auths| auths.is_empty())
            {
                continue;
            }

            // intrinsic_gas_too_low → satisfied. A tx whose gas limit is below
            // its intrinsic cost can never be validly included (EELS
            // `validate_transaction`; `transaction_intrinsic_gas` already folds
            // in the EIP-7623 calldata floor). A pricing/overflow error here
            // likewise means the tx is not includable, so we treat it as
            // satisfied. EELS also rejects an intrinsic cost above
            // `TX_MAX_GAS_LIMIT` — unreachable for any decodable transaction
            // (it would take megabytes of calldata), but mirrored for
            // completeness.
            match transaction_intrinsic_gas(tx_il, sender, header, config) {
                Ok(intrinsic) if tx_il.gas_limit() < intrinsic => continue,
                Ok(intrinsic) if intrinsic > TX_MAX_GAS_LIMIT_AMSTERDAM => continue,
                Err(_) => continue,
                Ok(_) => {}
            }

            // insufficient_gas → satisfied. EELS `check_block_gas_capacity`
            // checks the execution and state gas dimensions against their own
            // remaining budgets; the header only records their maximum
            // (`gas_used = max(execution, state)`), so `gas_left` is the
            // smaller of the two remaining budgets and this check matches EELS
            // exactly whenever `tx.gas <= TX_MAX_GAS_LIMIT` (the execution
            // dimension counts at most that much per tx).
            if tx_il.gas_limit() > gas_left {
                continue;
            }

            // Blob gas over the block's remaining blob budget → satisfied
            // (EELS `check_block_gas_capacity`, blob dimension).
            if tx_il.tx_type() == TxType::EIP4844 {
                let tx_blob_gas =
                    tx_il.blob_versioned_hashes().len() as u64 * u64::from(GAS_PER_BLOB);
                if tx_blob_gas > blob_gas_left {
                    continue;
                }
            }

            // below_base_fee → satisfied. Legacy/2930/privileged use
            // `gas_price`; all other types use `max_fee_per_gas`. A typed tx
            // with no recoverable max fee is unpriceable and not includable.
            // (EELS `calculate_effective_gas_price`.)
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

            // Blob fee cap below the block's blob gas price → satisfied (EELS
            // `check_max_fee_per_blob_gas` against the block's own
            // `excess_blob_gas`).
            if tx_il.tx_type() == TxType::EIP4844 {
                let blob_gas_price = calculate_base_fee_per_blob_gas(
                    header.excess_blob_gas.unwrap_or_default(),
                    config
                        .get_fork_blob_schedule(header.timestamp)
                        .map(|schedule| schedule.base_fee_update_fraction)
                        .unwrap_or_default(),
                );
                if tx_il.max_fee_per_blob_gas().unwrap_or_default() < blob_gas_price {
                    continue;
                }
            }

            // From here on, classify by tracked sender state.
            let tracked = match self.il_senders.get(&sender) {
                Some(entry) => entry.clone(),
                // The sender was not registered at construction. This means
                // the IL handed to `check` differs from the one handed to
                // `new`, which is a caller bug. Be defensive: treat the
                // sender as having empty state, which makes the tx unable
                // to be included (nonce/balance mismatch) and counts as
                // satisfied. This branch is unreachable in normal flow.
                None => TrackedSender::default(),
            };

            // invalid_nonce → satisfied
            if tx_il.nonce() != tracked.nonce {
                continue;
            }

            // invalid_balance → satisfied
            // `cost_without_base_fee` returns `None` only for unsigned/malformed
            // EIP-1559+ txs; treat such txs as `invalid_balance` (cannot be
            // priced) and count them as satisfied.
            let Some(cost) = tx_il.cost_without_base_fee() else {
                continue;
            };
            if cost > tracked.balance {
                continue;
            }

            // Sender is not an EOA → satisfied. EELS `check_transaction`
            // raises `InvalidSenderError` when the sender's code is set and is
            // not a valid EIP-7702 delegation (EIP-3607); such a sender's
            // transaction can never be included. The code snapshot comes from
            // the same post-state the nonce/balance were refreshed from.
            if let Some(code) = &tracked.code
                && !code.is_empty()
                && !is_eip7702_delegation(code)
            {
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
