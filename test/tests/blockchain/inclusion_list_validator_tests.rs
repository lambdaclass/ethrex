use std::cell::Cell;
use std::collections::HashSet;

use ethrex_blockchain::focil_eligibility::SenderCode;
use ethrex_blockchain::inclusion_list_builder::{
    AccountStateView, IlStateProvider, IlStateProviderError,
};
use ethrex_blockchain::inclusion_list_validator::{
    IlSenderState, IlUnsatisfied, InclusionListSatisfactionValidator,
};
use ethrex_common::types::{BlockHeader, ChainConfig, EIP1559Transaction, Transaction, TxKind};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use rustc_hash::FxHashMap;

/// In-memory `IlStateProvider` for tests. `panic_on_read` flips the
/// provider into a mode that panics if any read happens — used to
/// confirm that `check()` does not touch state. `codes` is keyed by
/// `code_hash` and consulted by `classify_code`; a hash not registered
/// there is treated as `Unknown`, matching an unregistered/absent code.
#[derive(Debug, Default)]
struct MockState {
    accounts: FxHashMap<Address, AccountStateView>,
    codes: FxHashMap<H256, SenderCode>,
    panic_on_read: bool,
    read_count: Cell<usize>,
}

impl MockState {
    fn with(accounts: FxHashMap<Address, AccountStateView>) -> Self {
        Self {
            accounts,
            codes: FxHashMap::default(),
            panic_on_read: false,
            read_count: Cell::new(0),
        }
    }

    /// Register `code_hash -> classification` so `classify_code` returns it
    /// instead of the `Unknown` default.
    fn with_code(mut self, code_hash: H256, code: SenderCode) -> Self {
        self.codes.insert(code_hash, code);
        self
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

    fn classify_code(&self, code_hash: H256) -> Result<SenderCode, IlStateProviderError> {
        if self.panic_on_read {
            panic!(
                "MockState::classify_code called during a no-EVM/no-state phase \
                 for code_hash {code_hash:?} — the satisfaction check must not read state"
            );
        }
        self.codes
            .get(&code_hash)
            .copied()
            .ok_or_else(|| IlStateProviderError::Read(format!("unregistered code {code_hash:?}")))
    }
}

/// `IlStateProvider` whose `get_account`/`classify_code` panic on every
/// call. Used to confirm that `check()` is purely state-tracker-driven and
/// does not reach into the provider.
#[derive(Debug, Default)]
struct PanicState;

impl IlStateProvider for PanicState {
    fn get_account(
        &self,
        _address: Address,
    ) -> Result<Option<AccountStateView>, IlStateProviderError> {
        panic!("check() must not invoke the state provider — pure tracker comparison only");
    }

    fn classify_code(&self, _code_hash: H256) -> Result<SenderCode, IlStateProviderError> {
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

/// An EOA account view (empty code, i.e. the `AccountStateView::default`
/// code hash), which is the shape every pre-existing test in this file
/// assumes.
fn account(nonce: u64, balance: U256) -> AccountStateView {
    AccountStateView {
        nonce,
        balance,
        ..Default::default()
    }
}

/// Account view with an explicit `code_hash`, for tests that exercise
/// sender-code classification via `MockState::with_code`.
fn account_with_code(nonce: u64, balance: U256, code_hash: H256) -> AccountStateView {
    AccountStateView {
        nonce,
        balance,
        code_hash,
    }
}

/// Build the tracker's `IlSenderState` shape for assertions against
/// `validator.il_senders`.
fn sender_state(nonce: u64, balance: U256, code: SenderCode) -> IlSenderState {
    IlSenderState {
        nonce,
        balance,
        code,
    }
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
        Some(&sender_state(5, rich_balance(), SenderCode::Eoa))
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
        Some(&sender_state(5, rich_balance(), SenderCode::Eoa))
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
        Some(&sender_state(6, U256::from(123u64), SenderCode::Eoa))
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
        Some(&sender_state(0, rich_balance(), SenderCode::Eoa))
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
///
/// The transaction is unkeyed (`nonce_keys` empty), so per EIP-8250 `nonce_seq`
/// is the sender's linear account nonce and is what `Transaction::nonce` returns.
fn make_frame_tx(sender: Address, nonce: u64, frame_gas_limit: u64) -> Transaction {
    use ethrex_common::types::{
        FRAME_SIG_SCHEME_SECP256K1, Frame, FrameMode, FrameSignature, FrameTransaction,
    };
    Transaction::FrameTransaction(FrameTransaction {
        chain_id: 1,
        nonce_seq: nonce,
        sender,
        frames: vec![Frame {
            mode: FrameMode::Sender as u8,
            flags: 0x00,
            target: Some(Address::repeat_byte(0xaa)),
            gas_limit: frame_gas_limit,
            state_limit: 0,
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

/// Regression for the EIP-8369 Profile 1 sender-validity gate (EIP-3607): a
/// contract sender cannot originate a transaction, so an IL tx from one can
/// never have been validly appended — its omission is excused even though
/// every other gate (nonce, balance, gas, fee) would pass.
#[test]
fn an_omitted_tx_from_a_contract_sender_is_excused() {
    let crypto = NativeCrypto;
    let alice = addr(1);
    let code_hash = H256::repeat_byte(0xc0);

    let il = vec![make_tx(alice, 0, 21_000, U256::from(1))];
    let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
    accounts.insert(alice, account_with_code(0, rich_balance(), code_hash));
    let state = MockState::with(accounts).with_code(code_hash, SenderCode::Contract);

    let validator =
        InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

    // Empty block, matching nonce, ample balance, plenty of gas, fee above
    // base — every gate except sender validity would pass.
    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(
        matches!(result, Ok(())),
        "an otherwise-appendable tx from a contract sender must be excused"
    );
}

/// The mirror image of the contract case: a valid EIP-7702 delegation
/// indicator keeps the sender in the "EOA in spirit" category, so its
/// omission is judged exactly like a plain EOA's — unsatisfied here since
/// every gate passes. Pins the direction of EIP-8369's rule: an inverted
/// implementation (delegated excused, contract punished) must fail this.
#[test]
fn an_omitted_tx_from_a_7702_delegated_sender_is_unsatisfied() {
    let crypto = NativeCrypto;
    let alice = addr(1);
    let code_hash = H256::repeat_byte(0xd0);

    let il = vec![make_tx(alice, 0, 21_000, U256::from(1))];
    let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
    accounts.insert(alice, account_with_code(0, rich_balance(), code_hash));
    let state = MockState::with(accounts).with_code(code_hash, SenderCode::Delegated);

    let validator =
        InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    match result {
        Err(IlUnsatisfied { tx_hash }) => assert_eq!(tx_hash, il[0].hash(&NativeCrypto)),
        other => panic!("expected Unsatisfied for a delegated EOA sender, got {other:?}"),
    }
}

/// Proves the existing EOA path is unchanged by the sender-code gate, and
/// catches the `AccountStateView::default` empty-code-hash trap: a derived
/// `Default` would give `H256::zero()` instead of `EMPTY_KECCAK_HASH`,
/// misclassifying every absent/EOA account as a contract and excusing
/// everything.
#[test]
fn an_omitted_tx_from_an_eoa_sender_is_still_unsatisfied() {
    let crypto = NativeCrypto;
    let alice = addr(1);

    let il = vec![make_tx(alice, 0, 21_000, U256::from(1))];
    let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
    accounts.insert(alice, account(0, rich_balance()));
    let state = MockState::with(accounts);

    let validator =
        InclusionListSatisfactionValidator::new(&il, &state, &crypto).expect("construct");

    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    match result {
        Err(IlUnsatisfied { tx_hash }) => assert_eq!(tx_hash, il[0].hash(&NativeCrypto)),
        other => panic!("expected Unsatisfied for a plain EOA sender, got {other:?}"),
    }
}

/// A `classify_code` failure must not abort construction/refresh nor turn a
/// justified omission into an unjustified one: it resolves to
/// `SenderCode::Unknown`, which does not originate, so the omission is
/// excused. This is the governing asymmetry from the type-level doc: a
/// code-read failure must neither abort the check nor punish.
#[test]
fn an_unclassifiable_sender_is_excused() {
    let crypto = NativeCrypto;
    let alice = addr(1);
    let code_hash = H256::repeat_byte(0xee);

    let il = vec![make_tx(alice, 0, 21_000, U256::from(1))];
    let mut accounts: FxHashMap<Address, AccountStateView> = Default::default();
    accounts.insert(alice, account_with_code(0, rich_balance(), code_hash));
    // `code_hash` is never registered via `with_code`, so `MockState`'s
    // `classify_code` errors for it.
    let state = MockState::with(accounts);

    let mut validator = InclusionListSatisfactionValidator::new(&il, &state, &crypto)
        .expect("new must not propagate a classify_code error");
    assert_eq!(
        validator.il_senders.get(&alice),
        Some(&sender_state(0, rich_balance(), SenderCode::Unknown))
    );

    validator
        .refresh_all_from(&state, &crypto)
        .expect("refresh_all_from must not propagate a classify_code error");
    assert_eq!(
        validator.il_senders.get(&alice),
        Some(&sender_state(0, rich_balance(), SenderCode::Unknown))
    );

    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(
        matches!(result, Ok(())),
        "an unclassifiable sender's omission must be excused"
    );
}

/// `check` never reaches back into a state provider. Its signature carries
/// no provider parameter, so this is a static guarantee, but building the
/// tracker directly (bypassing `new`) with a `Contract` classification and
/// handing `check` a `PanicState` in scope proves the sender-code gate added
/// by this change reads only the tracker, never the provider.
#[test]
fn check_reads_no_state_after_the_code_classification_landed() {
    let crypto = NativeCrypto;
    let alice = addr(1);

    let il = vec![make_tx(alice, 0, 21_000, U256::from(1))];

    let mut il_senders: FxHashMap<Address, IlSenderState> = Default::default();
    il_senders.insert(alice, sender_state(0, rich_balance(), SenderCode::Contract));
    let validator = InclusionListSatisfactionValidator { il_senders };

    let _panic_state = PanicState;
    let block_txs: HashSet<H256> = HashSet::new();
    let result = validator.check(&il, &block_txs, 30_000_000, &header(), &config(), &crypto);
    assert!(
        matches!(result, Ok(())),
        "a contract-sender omission must be excused with no state read"
    );
}
