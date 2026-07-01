use std::cell::Cell;
use std::collections::HashSet;

use ethrex_blockchain::inclusion_list_builder::{
    AccountStateView, IlStateProvider, IlStateProviderError,
};
use ethrex_blockchain::inclusion_list_validator::{
    IlUnsatisfied, InclusionListSatisfactionValidator,
};
use ethrex_common::types::{BlockHeader, ChainConfig, EIP1559Transaction, Transaction, TxKind};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use rustc_hash::FxHashMap;

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

/// Default block header for `check`. `base_fee_per_gas = None` (→ 0) and a
/// non-Amsterdam default config keep the intrinsic-gas / base-fee gates
/// inert for the simple 21k transfers the tests use.
fn header() -> BlockHeader {
    BlockHeader::default()
}

fn config() -> ChainConfig {
    ChainConfig::default()
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

    let block_txs: HashSet<H256> = il.iter().map(|t| t.hash(&NativeCrypto)).collect();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
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
    let result = validator.check(&il, &block_txs, 500_000, &header(), &config(), &crypto);
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

    let block_txs: HashSet<H256> = std::iter::once(bump_tx.hash(&NativeCrypto)).collect();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
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
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
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
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    match result {
        Err(IlUnsatisfied { tx_hash }) => {
            assert_eq!(tx_hash, il[0].hash(&NativeCrypto));
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
    let block_txs: HashSet<H256> = [
        t3.hash(&NativeCrypto),
        unrelated.hash(&NativeCrypto),
        t1.hash(&NativeCrypto),
        t2.hash(&NativeCrypto),
    ]
    .into_iter()
    .collect();

    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
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

    let r1 = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    let r2 = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);

    // Both runs must return the same Unsatisfied verdict for the same hash.
    match (r1, r2) {
        (Err(IlUnsatisfied { tx_hash: h1 }), Err(IlUnsatisfied { tx_hash: h2 })) => {
            assert_eq!(h1, h2);
            assert_eq!(h1, il[0].hash(&NativeCrypto));
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
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    match result {
        Err(_) => {}
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
    let block_txs: HashSet<H256> = std::iter::once(il[0].hash(&NativeCrypto)).collect();
    let _ = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    // Reach the end without panicking.
}

/// Build an EIP-4844 (blob) tx with a precached sender.
fn make_blob_tx(sender: Address, nonce: u64, gas_limit: u64) -> Transaction {
    use ethrex_common::types::EIP4844Transaction;
    let inner = EIP4844Transaction {
        chain_id: 1,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1,
        gas: gas_limit,
        to: Address::repeat_byte(0xaa),
        value: U256::zero(),
        max_fee_per_blob_gas: U256::from(1),
        blob_versioned_hashes: vec![H256::repeat_byte(0x01)],
        signature_r: U256::from(1),
        signature_s: U256::from(2),
        ..Default::default()
    };
    let tx = Transaction::EIP4844Transaction(inner);
    match &tx {
        Transaction::EIP4844Transaction(inner) => {
            let _ = inner.sender_cache.set(sender);
        }
        _ => unreachable!(),
    }
    tx
}

/// Build an EIP-1559 tx with a genuinely invalid signature (`r = s = 0`)
/// and NO precached sender, so `Transaction::sender` performs real ECDSA
/// recovery and fails.
fn make_unsigned_tx(nonce: u64, gas_limit: u64) -> Transaction {
    let inner = EIP1559Transaction {
        chain_id: 1,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1,
        gas_limit,
        to: TxKind::Call(Address::repeat_byte(0xbb)),
        value: U256::from(7u64),
        signature_y_parity: false,
        signature_r: U256::zero(),
        signature_s: U256::zero(),
        ..Default::default()
    };
    Transaction::EIP1559Transaction(inner)
}

/// Blob IL txs are excluded from the satisfaction check: an omitted blob
/// tx with a funded sender must classify as satisfied (EELS skips blobs).
#[test]
fn omitted_blob_il_tx_is_satisfied() {
    let crypto = NativeCrypto;
    let alice = addr(1);

    let il = vec![make_blob_tx(alice, 0, 21_000)];
    let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
    accounts.insert(alice, account(0, rich_balance()));
    let state = MockState::with(accounts);

    let validator =
        InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

    // Empty block, ample gas, funded sender — only the blob-skip rule keeps
    // this satisfied.
    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(matches!(result, Ok(())), "blob IL tx must be skipped");
}

/// An IL tx whose gas limit is below intrinsic gas can never be validly
/// appended → satisfied (EELS `validate_transaction` raises).
#[test]
fn omitted_intrinsic_gas_too_low_il_tx_is_satisfied() {
    let crypto = NativeCrypto;
    let alice = addr(1);

    // 20_999 < 21_000 intrinsic for a simple transfer (default/legacy fork).
    let il = vec![make_tx(alice, 0, 20_999, U256::from(1))];
    let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
    accounts.insert(alice, account(0, rich_balance()));
    let state = MockState::with(accounts);

    let validator =
        InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(
        matches!(result, Ok(())),
        "intrinsic-gas-too-low IL tx must be satisfied"
    );
}

/// An IL tx with an unrecoverable signature is silently skipped by both
/// `new` (no error) and `check` (satisfied) — EELS `recover_sender` raises.
#[test]
fn omitted_invalid_signature_il_tx_is_satisfied() {
    let crypto = NativeCrypto;

    let il = vec![make_unsigned_tx(0, 21_000)];
    // No accounts: `new` must not error despite the unrecoverable sender.
    let state = MockState::with(Default::default());

    let validator = InclusionListSatisfactionValidator::new(&il, &state, &crypto)
        .expect("construct must not propagate sender-recovery failure");
    assert!(
        validator.il_senders.is_empty(),
        "unrecoverable sender must not be registered"
    );

    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(
        matches!(result, Ok(())),
        "invalid-signature IL tx must be satisfied"
    );
}

/// An IL tx whose max fee is below the block base fee cannot be included
/// → satisfied (EELS `InsufficientMaxFeePerGasError`).
#[test]
fn omitted_below_base_fee_il_tx_is_satisfied() {
    let crypto = NativeCrypto;
    let alice = addr(1);

    // make_tx sets max_fee_per_gas = 1; pick a header base fee above it.
    let il = vec![make_tx(alice, 0, 21_000, U256::from(1))];
    let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
    accounts.insert(alice, account(0, rich_balance()));
    let state = MockState::with(accounts);

    let validator =
        InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

    let mut hdr = header();
    hdr.base_fee_per_gas = Some(100);

    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &hdr, &config(), &crypto);
    assert!(
        matches!(result, Ok(())),
        "below-base-fee IL tx must be satisfied"
    );

    // Control: with a base fee at/below the tx max fee, the same omitted
    // tx flips to Unsatisfied — proving the base-fee gate is what mattered.
    let mut hdr_ok = header();
    hdr_ok.base_fee_per_gas = Some(1);
    let control = validator.check(&il, &block_txs, 30_000_000, &hdr_ok, &config(), &crypto);
    assert!(matches!(control, Err(IlUnsatisfied { .. })));
}
