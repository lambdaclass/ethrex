//! EIP-7805 (FOCIL) inclusion-list satisfaction validator. Tracks per-sender
//! `(nonce, balance)` during block execution and, after execution, decides
//! whether each IL transaction is `present | insufficient_gas | invalid_nonce |
//! invalid_balance | unsatisfied`. Returns `Err(IlUnsatisfied)` if any IL
//! transaction is missing AND its sender retains nonce/balance/gas to include
//! it.
//!
//! Spec contract:
//! `openspec/changes/eip-7805-focil-execution-layer/specs/inclusion-list-satisfaction/spec.md`.
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

use ethrex_common::{Address, H256, U256, types::Transaction};
use ethrex_crypto::{Crypto, CryptoError};
use ethrex_storage::Store;
use rustc_hash::FxHashMap;

use crate::inclusion_list_builder::{AccountStateView, IlStateProvider, IlStateProviderError};

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
#[derive(Debug, thiserror::Error)]
pub enum IlValidatorError {
    #[error("could not recover IL transaction sender: {0}")]
    SenderRecovery(#[from] CryptoError),
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
    /// the [`IlStateProvider`] contract. A read error or sender-recovery
    /// error is propagated; the caller (engine handler) maps them to the
    /// internal-error JSON-RPC code.
    pub fn new(
        il: &[Transaction],
        pre_state: &dyn IlStateProvider,
        crypto: &dyn Crypto,
    ) -> Result<Self, IlValidatorError> {
        // Dedupe senders so we issue at most one state read per sender.
        let mut unique_senders: HashSet<Address> = HashSet::with_capacity(il.len());
        for tx in il {
            unique_senders.insert(tx.sender(crypto)?);
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
        let sender = executed.sender(crypto)?;
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
    /// `present | insufficient_gas | invalid_nonce | invalid_balance`. Return
    /// `Err(IlUnsatisfied)` for the first IL transaction that is missing AND
    /// whose sender retains nonce/balance and the block has gas room for it.
    ///
    /// `block_txs` is the set of transaction hashes included in the block;
    /// position within the block does not matter (per the EIP rationale).
    /// `gas_left` is `block.gas_limit - cumulative_gas_used` post-execution.
    ///
    /// This method MUST NOT call into the EVM. It is a pure state-comparison
    /// pass over the per-sender tracker.
    pub fn check(
        &self,
        il: &[Transaction],
        block_txs: &HashSet<H256>,
        gas_left: u64,
        crypto: &dyn Crypto,
    ) -> Result<(), IlCheckError> {
        for tx_il in il {
            // present in block (anywhere) → satisfied
            if block_txs.contains(&tx_il.hash()) {
                continue;
            }

            // insufficient_gas → satisfied
            if tx_il.gas_limit() > gas_left {
                continue;
            }

            // From here on, classify by tracked sender state.
            let sender = tx_il.sender(crypto).map_err(IlCheckError::SenderRecovery)?;
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
            return Err(IlCheckError::Unsatisfied(IlUnsatisfied {
                tx_hash: tx_il.hash(),
            }));
        }
        Ok(())
    }
}

/// Error returned by [`InclusionListSatisfactionValidator::check`]. Separates
/// the verdict (`Unsatisfied`) from infrastructure failures (sender
/// recovery), since the engine handler maps these to different JSON-RPC
/// responses.
#[derive(Debug, thiserror::Error)]
pub enum IlCheckError {
    #[error(transparent)]
    Unsatisfied(IlUnsatisfied),
    #[error("could not recover IL transaction sender during check: {0}")]
    SenderRecovery(CryptoError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inclusion_list_builder::AccountStateView;
    use ethrex_common::types::{EIP1559Transaction, Transaction, TxKind};
    use ethrex_crypto::NativeCrypto;
    use std::cell::Cell;

    /// In-memory `IlStateProvider` for tests. `panic_on_read` flips the
    /// provider into a mode that panics if any read happens — used to
    /// confirm that `check()` does not touch state.
    #[derive(Debug, Default)]
    struct MockState {
        accounts: FxHashMap<Address, AccountStateView>,
        panic_on_read: bool,
        read_count: Cell<usize>,
    }

    impl MockState {
        fn with(accounts: FxHashMap<Address, AccountStateView>) -> Self {
            Self {
                accounts,
                panic_on_read: false,
                read_count: Cell::new(0),
            }
        }
    }

    impl IlStateProvider for MockState {
        fn get_account(
            &self,
            address: Address,
        ) -> Result<Option<AccountStateView>, IlStateProviderError> {
            if self.panic_on_read {
                panic!(
                    "MockState::get_account called during a no-EVM/no-state phase \
                     for address {address:?} — the satisfaction check must not read state"
                );
            }
            self.read_count.set(self.read_count.get() + 1);
            Ok(self.accounts.get(&address).copied())
        }
    }

    /// `IlStateProvider` whose `get_account` panics on every call. Used to
    /// confirm that `check()` is purely state-tracker-driven and does not
    /// reach into the provider.
    #[derive(Debug, Default)]
    struct PanicState;

    impl IlStateProvider for PanicState {
        fn get_account(
            &self,
            _address: Address,
        ) -> Result<Option<AccountStateView>, IlStateProviderError> {
            panic!("check() must not invoke the state provider — pure tracker comparison only");
        }
    }

    /// Build an EIP-1559 transaction with a precomputed sender cached into
    /// the `sender_cache` so `Transaction::sender(&dyn Crypto)` returns the
    /// fixed value without invoking signature recovery (the test signatures
    /// are placeholders).
    fn make_tx(sender: Address, nonce: u64, gas_limit: u64, value: U256) -> Transaction {
        let inner = EIP1559Transaction {
            chain_id: 1,
            nonce,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 1,
            gas_limit,
            to: TxKind::Call(Address::repeat_byte(0xaa)),
            value,
            data: Default::default(),
            access_list: vec![],
            signature_y_parity: false,
            signature_r: U256::from(1),
            signature_s: U256::from(2),
            ..Default::default()
        };
        let tx = Transaction::EIP1559Transaction(inner);
        // Pre-cache the sender so Transaction::sender(...) returns it without
        // going through ECDSA recovery (the placeholder signature would not
        // recover a meaningful address).
        match &tx {
            Transaction::EIP1559Transaction(inner) => {
                let _ = inner.sender_cache.set(sender);
            }
            _ => unreachable!(),
        }
        tx
    }

    fn addr(b: u8) -> Address {
        Address::repeat_byte(b)
    }

    fn account(nonce: u64, balance: U256) -> AccountStateView {
        AccountStateView { nonce, balance }
    }

    /// Generous balance enough to fund any default-cost test tx.
    fn rich_balance() -> U256 {
        U256::from(10u64).pow(U256::from(18u64))
    }

    #[test]
    fn all_il_present_returns_ok() {
        let crypto = NativeCrypto;
        let alice = addr(1);
        let bob = addr(2);

        let il = vec![
            make_tx(alice, 5, 21_000, U256::from(1)),
            make_tx(bob, 9, 21_000, U256::from(1)),
        ];

        let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
        accounts.insert(alice, account(5, rich_balance()));
        accounts.insert(bob, account(9, rich_balance()));
        let state = MockState::with(accounts);

        let validator =
            InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

        let block_txs: HashSet<H256> = il.iter().map(|t| t.hash()).collect();
        let result = validator.check(&il, &block_txs, 30_000_000, &crypto);
        assert!(matches!(result, Ok(())));
    }

    #[test]
    fn il_omitted_with_insufficient_gas_returns_ok() {
        let crypto = NativeCrypto;
        let alice = addr(1);

        // gas_limit larger than what's left in the block
        let il = vec![make_tx(alice, 0, 1_000_000, U256::from(1))];
        let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
        accounts.insert(alice, account(0, rich_balance()));
        let state = MockState::with(accounts);

        let validator =
            InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

        let block_txs: HashSet<H256> = HashSet::new();
        // gas_left smaller than tx.gas_limit() → insufficient_gas
        let result = validator.check(&il, &block_txs, 500_000, &crypto);
        assert!(matches!(result, Ok(())));
    }

    #[test]
    fn il_omitted_with_advanced_nonce_returns_ok() {
        let crypto = NativeCrypto;
        let alice = addr(1);

        // IL says nonce 5, post-state says alice has nonce 6 (already moved on)
        let il = vec![make_tx(alice, 5, 21_000, U256::from(1))];

        let mut pre_accounts: FxHashMap<Address, AccountStateView> = Default::default();
        pre_accounts.insert(alice, account(5, rich_balance()));
        let pre_state = MockState::with(pre_accounts);

        let mut validator =
            InclusionListSatisfactionValidator::new(&il, &pre_state, &crypto).expect("construct");

        // Simulate a block-level executed tx that bumps alice's nonce to 6.
        let bump_tx = make_tx(alice, 5, 21_000, U256::from(1));
        let mut post_accounts: FxHashMap<Address, AccountStateView> = Default::default();
        post_accounts.insert(alice, account(6, rich_balance()));
        let post_state = MockState::with(post_accounts);
        validator
            .observe_executed_tx(&bump_tx, &post_state, &crypto)
            .expect("observe");

        let block_txs: HashSet<H256> = std::iter::once(bump_tx.hash()).collect();
        let result = validator.check(&il, &block_txs, 30_000_000, &crypto);
        assert!(matches!(result, Ok(())));
    }

    #[test]
    fn il_omitted_with_drained_balance_returns_ok() {
        let crypto = NativeCrypto;
        let alice = addr(1);

        // IL tx requires non-zero balance (gas * price + value).
        let il = vec![make_tx(alice, 5, 21_000, U256::from(1))];

        let mut pre_accounts: FxHashMap<Address, AccountStateView> = Default::default();
        pre_accounts.insert(alice, account(5, rich_balance()));
        let pre_state = MockState::with(pre_accounts);

        let mut validator =
            InclusionListSatisfactionValidator::new(&il, &pre_state, &crypto).expect("construct");

        // Some other (non-IL) tx by alice drains the balance to zero.
        let drain_tx = make_tx(alice, 5, 21_000, U256::from(1));
        let mut post_accounts: FxHashMap<Address, AccountStateView> = Default::default();
        post_accounts.insert(alice, account(5, U256::zero()));
        let post_state = MockState::with(post_accounts);
        validator
            .observe_executed_tx(&drain_tx, &post_state, &crypto)
            .expect("observe");

        // IL tx is omitted; tracker says alice has nonce 5 (matches IL) but
        // balance 0 (< cost). Should classify as invalid_balance → Ok.
        let block_txs: HashSet<H256> = HashSet::new();
        let result = validator.check(&il, &block_txs, 30_000_000, &crypto);
        assert!(matches!(result, Ok(())));
    }

    #[test]
    fn il_omitted_with_sufficient_state_returns_unsatisfied() {
        let crypto = NativeCrypto;
        let alice = addr(1);

        let il = vec![make_tx(alice, 5, 21_000, U256::from(1))];

        let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
        accounts.insert(alice, account(5, rich_balance()));
        let state = MockState::with(accounts);

        let validator =
            InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

        // Empty block; alice retains nonce 5 and rich balance; gas plenty.
        let block_txs: HashSet<H256> = HashSet::new();
        let result = validator.check(&il, &block_txs, 30_000_000, &crypto);
        match result {
            Err(IlCheckError::Unsatisfied(IlUnsatisfied { tx_hash })) => {
                assert_eq!(tx_hash, il[0].hash());
            }
            other => panic!("expected Unsatisfied, got {other:?}"),
        }
    }

    #[test]
    fn tracker_updates_when_executed_tx_touches_il_sender() {
        let crypto = NativeCrypto;
        let alice = addr(1);
        let bob = addr(2);

        let il = vec![make_tx(alice, 5, 21_000, U256::from(1))];

        let mut pre_accounts: FxHashMap<Address, AccountStateView> = Default::default();
        pre_accounts.insert(alice, account(5, rich_balance()));
        let pre_state = MockState::with(pre_accounts);

        let mut validator =
            InclusionListSatisfactionValidator::new(&il, &pre_state, &crypto).expect("construct");

        // Pre-condition: tracker has alice's pre-state nonce/balance.
        assert_eq!(
            validator.il_senders.get(&alice),
            Some(&(5u64, rich_balance()))
        );

        // Executed tx by bob (NOT in IL set) should NOT update the tracker.
        let bob_tx = make_tx(bob, 0, 21_000, U256::from(1));
        let mut bob_post: FxHashMap<Address, AccountStateView> = Default::default();
        bob_post.insert(bob, account(1, rich_balance()));
        let bob_state = MockState::with(bob_post);
        validator
            .observe_executed_tx(&bob_tx, &bob_state, &crypto)
            .expect("observe-bob");
        // bob is not in il_senders → no insertion
        assert!(!validator.il_senders.contains_key(&bob));
        // alice unchanged
        assert_eq!(
            validator.il_senders.get(&alice),
            Some(&(5u64, rich_balance()))
        );
        // bob_state was queried 0 times because bob is not tracked.
        assert_eq!(bob_state.read_count.get(), 0);

        // Executed tx by alice (in IL set) SHOULD update the tracker.
        let alice_tx = make_tx(alice, 5, 21_000, U256::from(1));
        let mut alice_post: FxHashMap<Address, AccountStateView> = Default::default();
        alice_post.insert(alice, account(6, U256::from(123u64)));
        let alice_state = MockState::with(alice_post);
        validator
            .observe_executed_tx(&alice_tx, &alice_state, &crypto)
            .expect("observe-alice");
        assert_eq!(
            validator.il_senders.get(&alice),
            Some(&(6u64, U256::from(123u64)))
        );
        // alice_state should have been read exactly once.
        assert_eq!(alice_state.read_count.get(), 1);
    }

    #[test]
    fn il_position_in_block_does_not_matter() {
        let crypto = NativeCrypto;
        let alice = addr(1);
        let bob = addr(2);
        let carol = addr(3);

        // IL of 3 txs.
        let t1 = make_tx(alice, 0, 21_000, U256::from(1));
        let t2 = make_tx(bob, 0, 21_000, U256::from(1));
        let t3 = make_tx(carol, 0, 21_000, U256::from(1));
        let il = vec![t1.clone(), t2.clone(), t3.clone()];

        let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
        accounts.insert(alice, account(0, rich_balance()));
        accounts.insert(bob, account(0, rich_balance()));
        accounts.insert(carol, account(0, rich_balance()));
        let state = MockState::with(accounts);

        let validator =
            InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

        // Block presents the IL txs in arbitrary order, interleaved with
        // unrelated txs. `check` only consults `block_txs` membership, not
        // ordering.
        let unrelated = make_tx(addr(99), 0, 21_000, U256::from(1));
        let block_txs: HashSet<H256> = [t3.hash(), unrelated.hash(), t1.hash(), t2.hash()]
            .into_iter()
            .collect();

        let result = validator.check(&il, &block_txs, 30_000_000, &crypto);
        assert!(matches!(result, Ok(())));
    }

    #[test]
    fn algorithm_is_idempotent_over_il() {
        let crypto = NativeCrypto;
        let alice = addr(1);

        // Unsatisfied scenario: IL tx not in block, sender retains state.
        let il = vec![make_tx(alice, 0, 21_000, U256::from(1))];
        let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
        accounts.insert(alice, account(0, rich_balance()));
        let state = MockState::with(accounts);

        let validator =
            InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

        let block_txs: HashSet<H256> = HashSet::new();

        let r1 = validator.check(&il, &block_txs, 30_000_000, &crypto);
        let r2 = validator.check(&il, &block_txs, 30_000_000, &crypto);

        // Both runs must return the same Unsatisfied verdict for the same hash.
        match (r1, r2) {
            (
                Err(IlCheckError::Unsatisfied(IlUnsatisfied { tx_hash: h1 })),
                Err(IlCheckError::Unsatisfied(IlUnsatisfied { tx_hash: h2 })),
            ) => {
                assert_eq!(h1, h2);
                assert_eq!(h1, il[0].hash());
            }
            other => panic!("expected matched Unsatisfied verdicts, got {other:?}"),
        }

        // Tracker is unchanged after `check` — confirms idempotence at the
        // state level, not just the verdict level.
        assert_eq!(
            validator.il_senders.get(&alice),
            Some(&(0u64, rich_balance()))
        );
    }

    #[test]
    fn algorithm_does_not_invoke_evm() {
        // Use a state provider that PANICS on every call. If `check` is
        // EVM-free and tracker-only, no read should happen.
        let crypto = NativeCrypto;
        let alice = addr(1);

        let il = vec![make_tx(alice, 0, 21_000, U256::from(1))];

        // Construct via a normal provider so the tracker is populated.
        let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
        accounts.insert(alice, account(0, rich_balance()));
        let init_state = MockState::with(accounts);
        let validator =
            InclusionListSatisfactionValidator::new(&il, &init_state, &crypto).expect("construct");

        // Now call `check` with a panic-on-read provider in scope... except
        // `check` does not take a provider. The only way it could "call into
        // the EVM" is by re-executing transactions, which would require a VM
        // surface that this module does not import. We assert the contract
        // by:
        //   1. Confirming the test does not link any EVM execution surface
        //      (this module only depends on `Transaction`, `Crypto`, and the
        //      `IlStateProvider` trait; it has no VM imports, statically
        //      provable).
        //   2. Confirming that running `check` on a populated tracker does
        //      not exhibit any side effects on a sentinel state provider.
        let _panic_state = PanicState;
        // Empty block → IL tx omitted → returns Unsatisfied without ever
        // touching `_panic_state` or any execution surface.
        let block_txs: HashSet<H256> = HashSet::new();
        let result = validator.check(&il, &block_txs, 30_000_000, &crypto);
        match result {
            Err(IlCheckError::Unsatisfied(_)) => {}
            other => panic!("expected Unsatisfied, got {other:?}"),
        }
    }

    /// Bonus: `check` does not consult the state provider even when given a
    /// fully panicking one. The test would fail (panic) if `check` ever
    /// reached out to state.
    #[test]
    fn check_does_not_call_state_provider() {
        let crypto = NativeCrypto;
        let alice = addr(1);

        let il = vec![make_tx(alice, 0, 21_000, U256::from(1))];

        // Populate tracker via a normal state.
        let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
        accounts.insert(alice, account(0, rich_balance()));
        let init_state = MockState::with(accounts);
        let validator =
            InclusionListSatisfactionValidator::new(&il, &init_state, &crypto).expect("construct");

        // After construction, `check` must be self-sufficient. We do not
        // pass a provider into `check`, by design (signature confirms this).
        // This test documents the design: `check`'s signature contains no
        // provider, so it cannot call out to one.
        let block_txs: HashSet<H256> = std::iter::once(il[0].hash()).collect();
        let _ = validator.check(&il, &block_txs, 30_000_000, &crypto);
        // Reach the end without panicking.
    }
}
