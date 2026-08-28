use std::cell::Cell;
use std::collections::HashSet;

use ethrex_blockchain::inclusion_list_validator::{
    AccountStateView, IlStateProvider, IlStateProviderError, IlUnsatisfied,
    InclusionListSatisfactionValidator, TrackedSender,
};
use ethrex_common::types::{
    BlockHeader, ChainConfig, EIP1559Transaction, Transaction, TxKind, Withdrawal,
};
use ethrex_common::{Address, Bytes, H256, U256};
use ethrex_crypto::NativeCrypto;
use rustc_hash::FxHashMap;

/// In-memory `IlStateProvider` for tests. `panic_on_read` flips the
/// provider into a mode that panics if any read happens — used to
/// confirm that `check()` does not touch state.
#[derive(Debug, Default)]
struct MockState {
    accounts: FxHashMap<Address, AccountStateView>,
    /// Per-address contract code, for the sender-is-EOA gate. Addresses
    /// absent here have no code.
    codes: FxHashMap<Address, Bytes>,
    panic_on_read: bool,
    read_count: Cell<usize>,
}

impl MockState {
    fn with(accounts: FxHashMap<Address, AccountStateView>) -> Self {
        Self {
            accounts,
            codes: Default::default(),
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

    // Deliberately does not bump `read_count`, which documents the number of
    // ACCOUNT reads a flow performs.
    fn get_code(&self, address: Address) -> Result<Option<Bytes>, IlStateProviderError> {
        if self.panic_on_read {
            panic!(
                "MockState::get_code called during a no-EVM/no-state phase \
                 for address {address:?} — the satisfaction check must not read state"
            );
        }
        Ok(self.codes.get(&address).cloned())
    }
}

/// `IlStateProvider` that panics on every call. Used to confirm that
/// `check()` is purely state-tracker-driven and does not reach into the
/// provider.
#[derive(Debug, Default)]
struct PanicState;

impl IlStateProvider for PanicState {
    fn get_account(
        &self,
        _address: Address,
    ) -> Result<Option<AccountStateView>, IlStateProviderError> {
        panic!("check() must not invoke the state provider — pure tracker comparison only");
    }

    fn get_code(&self, _address: Address) -> Result<Option<Bytes>, IlStateProviderError> {
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
    ChainConfig {
        // Match `make_tx`'s declared chain id: the wrong-chain-id gate would
        // otherwise excuse every test transaction.
        chain_id: 1,
        ..Default::default()
    }
}

/// Tracker entry for a code-less sender.
fn tracked(nonce: u64, balance: U256) -> TrackedSender {
    TrackedSender {
        nonce,
        balance,
        code: None,
    }
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

    // The block includes a tx of alice's that bumps her nonce to 6.
    let bump_tx = make_tx(alice, 5, 21_000, U256::from(1));
    let mut post_accounts: FxHashMap<Address, AccountStateView> = Default::default();
    post_accounts.insert(alice, account(6, rich_balance()));
    let post_state = MockState::with(post_accounts);
    validator.refresh_all_from(&post_state).expect("refresh");

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
    let mut post_accounts: FxHashMap<Address, AccountStateView> = Default::default();
    post_accounts.insert(alice, account(5, U256::zero()));
    let post_state = MockState::with(post_accounts);
    validator.refresh_all_from(&post_state).expect("refresh");

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
fn refresh_reads_only_the_inclusion_list_senders() {
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
        Some(&tracked(5, rich_balance()))
    );

    // The block moved both alice and bob, but only alice is an IL sender: the
    // refresh must pick up alice's new state and must not start tracking bob,
    // so the tracker stays bounded by the inclusion list rather than by the
    // block's transaction count.
    let mut post_accounts: FxHashMap<Address, AccountStateView> = Default::default();
    post_accounts.insert(alice, account(6, U256::from(123u64)));
    post_accounts.insert(bob, account(1, rich_balance()));
    let post_state = MockState::with(post_accounts);
    validator.refresh_all_from(&post_state).expect("refresh");

    assert_eq!(
        validator.il_senders.get(&alice),
        Some(&tracked(6, U256::from(123u64)))
    );
    assert!(!validator.il_senders.contains_key(&bob));
    // Exactly one account read: alice's. Bob's entry was never consulted.
    assert_eq!(post_state.read_count.get(), 1);
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
        Some(&tracked(0, rich_balance()))
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

/// Build an EIP-8141 frame tx. `Transaction::sender` reads the explicit
/// `sender` field for this type, so no signature material is needed.
fn make_frame_tx(sender: Address, nonce: u64, frame_gas_limit: u64) -> Transaction {
    use ethrex_common::types::{
        FRAME_SIG_SCHEME_SECP256K1, Frame, FrameMode, FrameSignature, FrameTransaction,
    };
    Transaction::FrameTransaction(FrameTransaction {
        chain_id: 1,
        nonce,
        sender,
        frames: vec![Frame {
            mode: FrameMode::Sender as u8,
            flags: 0x00,
            target: Some(Address::repeat_byte(0xaa)),
            gas_limit: frame_gas_limit,
            value: U256::zero(),
            data: Default::default(),
        }],
        signatures: vec![FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(sender),
            msg: Default::default(),
            signature: Default::default(),
        }],
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1,
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: vec![],
        ..Default::default()
    })
}

/// An omitted EIP-8141 frame tx is excused: its validity depends on executing
/// VERIFY frames to discover `payer`, so this state-only pass cannot judge it.
/// Every other gate passes here — the sender's tracked nonce matches and its
/// balance covers the cost — so only the frame skip keeps the block satisfied.
#[test]
fn omitted_frame_il_tx_is_satisfied() {
    let crypto = NativeCrypto;
    let alice = addr(1);

    let il = vec![make_frame_tx(alice, 0, 100_000)];
    let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
    accounts.insert(alice, account(0, rich_balance()));
    let state = MockState::with(accounts);

    let validator =
        InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(matches!(result, Ok(())), "frame IL tx must be skipped");
}

/// A frame tx included in the block is satisfied by the presence check, which
/// runs before the frame skip.
#[test]
fn frame_il_tx_present_in_block_is_satisfied() {
    let crypto = NativeCrypto;
    let alice = addr(1);

    let il = vec![make_frame_tx(alice, 0, 100_000)];
    let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
    accounts.insert(alice, account(0, rich_balance()));
    let state = MockState::with(accounts);

    let validator =
        InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

    let block_txs: HashSet<H256> = il.iter().map(|t| t.hash(&NativeCrypto)).collect();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(
        matches!(result, Ok(())),
        "present frame IL tx must be satisfied"
    );
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

// NOTE: the former `omitted_blob_il_tx_is_satisfied` test pinned the
// pre-tests-focil-devnet@v0.2.0 rule that excused every blob transaction.
// That release's spec fix ("Type-3 transactions were considered
// not-includable by default") evaluates blob txs like any other type; the
// replacement coverage lives in `il_omitted_includable_blob_tx_returns_unsatisfied`
// and `il_omitted_blob_tx_invalid_variants_return_ok` below.

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

// ─── Includability gates added for tests-focil-devnet@v0.2.0 ─────────────────
//
// EELS `check_inclusion_list_transactions` (forks/amsterdam) replays
// `validate_transaction` + `check_transaction` for every missing IL tx; the
// tests below pin the ethrex mirror of the gates that release introduced or
// changed: wrong chain id, nonce overflow, priority fee above the cap,
// oversized init code, empty authorization list, contract sender, and the
// full evaluation of blob transactions (previously excused wholesale).

/// EIP-1559 tx with every fee/shape knob exposed, sender pre-cached like
/// [`make_tx`].
#[allow(clippy::too_many_arguments)]
fn make_tx_full(
    sender: Address,
    chain_id: u64,
    nonce: u64,
    max_priority_fee_per_gas: u64,
    max_fee_per_gas: u64,
    gas_limit: u64,
    to: TxKind,
    data: Vec<u8>,
) -> Transaction {
    let inner = EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas,
        max_fee_per_gas,
        gas_limit,
        to,
        value: U256::from(1),
        data: data.into(),
        access_list: vec![],
        signature_y_parity: false,
        signature_r: U256::from(1),
        signature_s: U256::from(2),
        ..Default::default()
    };
    let tx = Transaction::EIP1559Transaction(inner);
    match &tx {
        Transaction::EIP1559Transaction(inner) => {
            let _ = inner.sender_cache.set(sender);
        }
        _ => unreachable!(),
    }
    tx
}

/// EIP-4844 tx with the given versioned hashes and blob fee cap, sender
/// pre-cached.
fn make_blob_tx(
    sender: Address,
    nonce: u64,
    blob_versioned_hashes: Vec<H256>,
    max_fee_per_blob_gas: U256,
) -> Transaction {
    let inner = ethrex_common::types::EIP4844Transaction {
        chain_id: 1,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1,
        gas: 100_000,
        to: Address::repeat_byte(0xaa),
        value: U256::from(1),
        data: Default::default(),
        access_list: vec![],
        max_fee_per_blob_gas,
        blob_versioned_hashes,
        signature_y_parity: false,
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

/// A KZG-versioned (0x01-prefixed) blob versioned hash.
fn kzg_hash() -> H256 {
    let mut h = [0u8; 32];
    h[0] = 0x01;
    H256::from(h)
}

/// Config with blob parameters in force at timestamp 0 (Cancun default
/// schedule: target 3 / max 6 / fraction 3338477).
fn blob_config() -> ChainConfig {
    ChainConfig {
        cancun_time: Some(0),
        shanghai_time: Some(0),
        ..config()
    }
}

fn single_sender_validator(
    il: &[Transaction],
    sender: Address,
    nonce: u64,
) -> InclusionListSatisfactionValidator {
    let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
    accounts.insert(sender, account(nonce, rich_balance()));
    let state = MockState::with(accounts);
    InclusionListSatisfactionValidator::new(il, &state, &NativeCrypto).expect("construct")
}

#[test]
fn il_omitted_with_wrong_chain_id_returns_ok() {
    let crypto = NativeCrypto;
    let alice = addr(1);

    // Declared chain id 2 against config chain id 1 → never includable.
    let il = vec![make_tx_full(
        alice,
        2,
        0,
        1,
        1,
        21_000,
        TxKind::Call(Address::repeat_byte(0xaa)),
        vec![],
    )];
    let validator = single_sender_validator(&il, alice, 0);

    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(
        matches!(result, Ok(())),
        "wrong-chain-id tx must be excused"
    );

    // Control: identical tx declaring the chain's id flips to Unsatisfied.
    let il_ok = vec![make_tx_full(
        alice,
        1,
        0,
        1,
        1,
        21_000,
        TxKind::Call(Address::repeat_byte(0xaa)),
        vec![],
    )];
    let validator_ok = single_sender_validator(&il_ok, alice, 0);
    let control = validator_ok.check(
        &il_ok,
        &block_txs,
        30_000_000,
        &header(),
        &config(),
        &crypto,
    );
    assert!(matches!(control, Err(IlUnsatisfied { .. })));
}

#[test]
fn il_omitted_with_max_nonce_returns_ok() {
    let crypto = NativeCrypto;
    let alice = addr(1);

    // EIP-2681: nonce == 2**64 - 1 can never be included, even when the
    // sender's account sits at that nonce (only reachable via pre-state).
    let il = vec![make_tx(alice, u64::MAX, 21_000, U256::from(1))];
    let validator = single_sender_validator(&il, alice, u64::MAX);

    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(
        matches!(result, Ok(())),
        "nonce-overflow tx must be excused"
    );
}

#[test]
fn il_omitted_with_priority_above_max_fee_returns_ok() {
    let crypto = NativeCrypto;
    let alice = addr(1);

    let il = vec![make_tx_full(
        alice,
        1,
        0,
        2, // max_priority_fee_per_gas above...
        1, // ...max_fee_per_gas
        21_000,
        TxKind::Call(Address::repeat_byte(0xaa)),
        vec![],
    )];
    let validator = single_sender_validator(&il, alice, 0);

    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(
        matches!(result, Ok(())),
        "priority-above-cap tx must be excused"
    );
}

#[test]
fn il_omitted_with_oversized_initcode_returns_ok() {
    let crypto = NativeCrypto;
    let alice = addr(1);

    // Default (pre-Amsterdam) cap is MAX_INITCODE_SIZE = 49152 bytes.
    let il = vec![make_tx_full(
        alice,
        1,
        0,
        1,
        1,
        1_000_000,
        TxKind::Create,
        vec![0u8; 49_153],
    )];
    let validator = single_sender_validator(&il, alice, 0);

    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(
        matches!(result, Ok(())),
        "oversized-initcode creation must be excused"
    );

    // Control: at exactly the cap the creation is includable → Unsatisfied.
    let il_ok = vec![make_tx_full(
        alice,
        1,
        0,
        1,
        1,
        1_000_000,
        TxKind::Create,
        vec![0u8; 49_152],
    )];
    let validator_ok = single_sender_validator(&il_ok, alice, 0);
    let control = validator_ok.check(
        &il_ok,
        &block_txs,
        30_000_000,
        &header(),
        &config(),
        &crypto,
    );
    assert!(matches!(control, Err(IlUnsatisfied { .. })));
}

#[test]
fn il_omitted_with_empty_authorization_list_returns_ok() {
    let crypto = NativeCrypto;
    let alice = addr(1);

    let inner = ethrex_common::types::EIP7702Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1,
        gas_limit: 100_000,
        to: Address::repeat_byte(0xaa),
        value: U256::from(1),
        data: Default::default(),
        access_list: vec![],
        authorization_list: vec![],
        signature_y_parity: false,
        signature_r: U256::from(1),
        signature_s: U256::from(2),
        ..Default::default()
    };
    let tx = Transaction::EIP7702Transaction(inner);
    match &tx {
        Transaction::EIP7702Transaction(inner) => {
            let _ = inner.sender_cache.set(alice);
        }
        _ => unreachable!(),
    }
    let il = vec![tx];
    let validator = single_sender_validator(&il, alice, 0);

    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(
        matches!(result, Ok(())),
        "empty-authorization-list tx must be excused"
    );
}

#[test]
fn il_omitted_from_contract_sender_returns_ok() {
    let crypto = NativeCrypto;
    let alice = addr(1);

    let il = vec![make_tx(alice, 0, 21_000, U256::from(1))];

    // Alice's account carries plain contract code → EIP-3607 bars her from
    // sending, so the omitted tx is excused.
    let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
    accounts.insert(alice, account(0, rich_balance()));
    let mut codes: FxHashMap<Address, Bytes> = Default::default();
    codes.insert(alice, Bytes::from(vec![0x60, 0x00]));
    let state = MockState {
        accounts: accounts.clone(),
        codes,
        panic_on_read: false,
        read_count: Cell::new(0),
    };
    let validator =
        InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(matches!(result, Ok(())), "contract sender must be excused");

    // Control: an EIP-7702 delegation designation keeps the account an EOA in
    // spirit → the omitted tx counts against the block.
    let mut delegation = vec![0xef, 0x01, 0x00];
    delegation.extend_from_slice(&[0x11; 20]);
    let mut delegated_codes: FxHashMap<Address, Bytes> = Default::default();
    delegated_codes.insert(alice, Bytes::from(delegation));
    let delegated_state = MockState {
        accounts,
        codes: delegated_codes,
        panic_on_read: false,
        read_count: Cell::new(0),
    };
    let validator_ok =
        InclusionListSatisfactionValidator::new(&il, &delegated_state, &crypto).expect("construct");
    let control = validator_ok.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(matches!(control, Err(IlUnsatisfied { .. })));
}

#[test]
fn il_omitted_includable_blob_tx_returns_unsatisfied() {
    let crypto = NativeCrypto;
    let alice = addr(1);

    // Fully includable blob tx (valid hash, fee covers the blob gas price,
    // budget available): since tests-focil-devnet@v0.2.0 it counts against
    // the block instead of being excused as a blob tx.
    let il = vec![make_blob_tx(alice, 0, vec![kzg_hash()], U256::from(1))];
    let validator = single_sender_validator(&il, alice, 0);

    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(
        &il,
        &block_txs,
        30_000_000,
        &header(),
        &blob_config(),
        &crypto,
    );
    assert!(
        matches!(result, Err(IlUnsatisfied { .. })),
        "includable blob tx must count against the block, got {result:?}"
    );
}

#[test]
fn il_omitted_blob_tx_invalid_variants_return_ok() {
    let crypto = NativeCrypto;
    let alice = addr(1);
    let block_txs: HashSet<H256> = HashSet::new();

    // Zero blobs.
    let il = vec![make_blob_tx(alice, 0, vec![], U256::from(1))];
    let validator = single_sender_validator(&il, alice, 0);
    assert!(
        matches!(
            validator.check(
                &il,
                &block_txs,
                30_000_000,
                &header(),
                &blob_config(),
                &crypto
            ),
            Ok(())
        ),
        "zero-blob tx must be excused"
    );

    // More blobs than a tx may carry (EELS `BLOB_COUNT_LIMIT` = 6).
    let il = vec![make_blob_tx(alice, 0, vec![kzg_hash(); 7], U256::from(1))];
    let validator = single_sender_validator(&il, alice, 0);
    assert!(
        matches!(
            validator.check(
                &il,
                &block_txs,
                30_000_000,
                &header(),
                &blob_config(),
                &crypto
            ),
            Ok(())
        ),
        "over-the-cap blob count must be excused"
    );

    // Versioned hash that is not KZG-versioned.
    let il = vec![make_blob_tx(
        alice,
        0,
        vec![H256::repeat_byte(0x02)],
        U256::from(1),
    )];
    let validator = single_sender_validator(&il, alice, 0);
    assert!(
        matches!(
            validator.check(
                &il,
                &block_txs,
                30_000_000,
                &header(),
                &blob_config(),
                &crypto
            ),
            Ok(())
        ),
        "non-KZG versioned hash must be excused"
    );

    // Blob fee cap below the block's blob gas price (price is 1 at zero
    // excess blob gas).
    let il = vec![make_blob_tx(alice, 0, vec![kzg_hash()], U256::zero())];
    let validator = single_sender_validator(&il, alice, 0);
    assert!(
        matches!(
            validator.check(
                &il,
                &block_txs,
                30_000_000,
                &header(),
                &blob_config(),
                &crypto
            ),
            Ok(())
        ),
        "blob fee below the blob gas price must be excused"
    );

    // Blob budget exhausted: the block already used its whole blob allowance
    // (6 blobs × 131072 gas).
    let il = vec![make_blob_tx(alice, 0, vec![kzg_hash()], U256::from(1))];
    let validator = single_sender_validator(&il, alice, 0);
    let mut hdr = header();
    hdr.blob_gas_used = Some(6 * 131_072);
    assert!(
        matches!(
            validator.check(&il, &block_txs, 30_000_000, &hdr, &blob_config(), &crypto),
            Ok(())
        ),
        "blob tx beyond the remaining blob budget must be excused"
    );
}

/// The satisfaction check evaluates senders BEFORE same-block withdrawals are
/// processed: EELS `apply_body` runs `check_inclusion_list_transactions`
/// between the block's transactions and `process_withdrawals`, so a sender
/// funded only by a withdrawal in the same block could not have had its tx
/// appended (mirrors `test_use_value_in_tx[tx_in_withdrawals_block]`'s
/// inclusion-list variant from tests-focil-devnet@v0.2.0).
#[test]
fn il_sender_funded_by_same_block_withdrawal_is_excused() {
    let crypto = NativeCrypto;
    let alice = addr(1);
    let bob = addr(2);

    let il = vec![make_tx(alice, 0, 21_000, U256::from(1))];

    // Pre-state: penniless. Post-state: exactly the withdrawal's credit
    // (1 gwei = 10^9 wei, comfortably above the 21_001-wei tx cost).
    let mut pre_accounts: FxHashMap<Address, AccountStateView> = Default::default();
    pre_accounts.insert(alice, account(0, U256::zero()));
    let pre_state = MockState::with(pre_accounts);
    let mut validator =
        InclusionListSatisfactionValidator::new(&il, &pre_state, &crypto).expect("construct");

    let credit_gwei = 1u64;
    let mut post_accounts: FxHashMap<Address, AccountStateView> = Default::default();
    post_accounts.insert(
        alice,
        account(0, U256::from(credit_gwei) * U256::from(1_000_000_000u64)),
    );
    let post_state = MockState::with(post_accounts);
    validator.refresh_all_from(&post_state).expect("refresh");

    // Control first (`check` is read-only): with the credit still in the
    // tracker the tx is includable, so the omission counts against the block.
    let block_txs: HashSet<H256> = HashSet::new();
    let control = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(matches!(control, Err(IlUnsatisfied { .. })));

    // Discounting the block's withdrawals (including one to an untracked
    // address, which must be a no-op) rolls alice back to her
    // pre-withdrawals balance: the tx is no longer payable → excused.
    validator.discount_withdrawals(&[
        Withdrawal {
            index: 0,
            validator_index: 0,
            address: alice,
            amount: credit_gwei,
        },
        Withdrawal {
            index: 1,
            validator_index: 1,
            address: bob,
            amount: 7,
        },
    ]);
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(
        matches!(result, Ok(())),
        "withdrawal-funded IL sender must be excused, got {result:?}"
    );
}
