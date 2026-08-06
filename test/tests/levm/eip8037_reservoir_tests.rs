//! EIP-8037 source-based state-gas refund unit tests.
//!
//! These exercise the two-pool (reservoir + per-frame spill) state-gas arithmetic
//! (`increase_state_gas` / `credit_state_gas_refund` / `refill_frame_state_gas`) in isolation,
//! fixture-free. The VM is built via `VM::new_state_gas_harness` (a `#[doc(hidden)]` test-support
//! constructor) rather than `VM::new`, because `VM::new` runs `Substate::initialize` /
//! `get_tx_callee` against the database; that machinery is irrelevant to the pure arithmetic under
//! test and would require a full store. The harness wires up only what the three methods touch:
//! an Amsterdam (or Prague) `Environment`, one top-level call frame, and a configurable reservoir.

use ethrex_common::{
    Address, H256, U256,
    types::{AccountState, ChainConfig, Code, CodeMetadata, EIP1559Transaction, Fork, Transaction},
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    db::{Database, gen_db::GeneralizedDatabase},
    errors::DatabaseError,
    vm::VM,
};
use std::sync::Arc;

/// Minimal in-crate [`Database`] used only to satisfy [`GeneralizedDatabase::new`].
/// None of its methods are reached by these tests (the VM is built by the harness, so no
/// account/storage/code loads occur), so every method returns an error.
struct StubDatabase;

impl Database for StubDatabase {
    fn get_account_state(&self, _address: Address) -> Result<AccountState, DatabaseError> {
        Err(DatabaseError::Custom("stub db: no account state".into()))
    }
    fn get_storage_value(&self, _address: Address, _key: H256) -> Result<U256, DatabaseError> {
        Err(DatabaseError::Custom("stub db: no storage value".into()))
    }
    fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
        Err(DatabaseError::Custom("stub db: no block hash".into()))
    }
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Err(DatabaseError::Custom("stub db: no chain config".into()))
    }
    fn get_account_code(&self, _code_hash: H256) -> Result<Code, DatabaseError> {
        Err(DatabaseError::Custom("stub db: no account code".into()))
    }
    fn get_code_metadata(&self, _code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        Err(DatabaseError::Custom("stub db: no code metadata".into()))
    }
}

/// Builds a `GeneralizedDatabase` over the stub backend. Returned by value so the caller
/// owns it for at least the VM's lifetime.
fn stub_db() -> GeneralizedDatabase {
    GeneralizedDatabase::new(Arc::new(StubDatabase))
}

/// A trivial transaction the VM borrows but never reads in these tests.
fn stub_tx() -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction::default())
}

/// Convenience: lossless `u64 -> i64` for test values (all well below `i64::MAX`).
fn as_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// 5.2 — Full spill then refund (reservoir empty).
///
/// reservoir = 0, so `increase_state_gas(N)` spills the whole charge out of `gas_remaining`.
/// `credit_state_gas_refund(N)` must, in LIFO order, return all of it to `gas_remaining` (none
/// to the reservoir) and zero every counter.
#[test]
fn credit_lifo_spill_first() {
    const N: u64 = 5_000;
    let mut db = stub_db();
    let tx = stub_tx();
    let crypto = NativeCrypto;
    let mut vm = VM::new_state_gas_harness(Fork::Amsterdam, &mut db, &tx, &crypto, 0);

    let gas_before = vm.frame_gas_remaining();
    vm.increase_state_gas(N).unwrap();

    // The full charge spilled to gas_remaining.
    assert_eq!(vm.frame_state_gas_spilled(), N, "whole charge must spill");
    assert_eq!(
        vm.frame_gas_remaining(),
        gas_before - as_i64(N),
        "gas_remaining drops by the spilled amount"
    );
    assert_eq!(vm.state_gas_spill(), N, "vm-level spill tracks the spill");
    assert_eq!(vm.state_gas_used(), as_i64(N));
    assert_eq!(vm.state_gas_reservoir(), 0);

    vm.credit_state_gas_refund(N).unwrap();

    assert_eq!(
        vm.frame_gas_remaining(),
        gas_before,
        "gas_remaining fully restored (LIFO: spill returned first)"
    );
    assert_eq!(vm.frame_state_gas_spilled(), 0, "frame spill drained");
    assert_eq!(vm.state_gas_reservoir(), 0, "nothing flowed to reservoir");
    assert_eq!(vm.state_gas_spill(), 0, "vm-level spill drained");
    assert_eq!(vm.state_gas_used(), 0, "state_gas_used back to zero");
}

/// 5.3 — Charge fully from reservoir, no spill, then refund.
///
/// reservoir = N, so `increase_state_gas(N)` draws entirely from the reservoir and never touches
/// `gas_remaining`. The refund must return the full amount to the reservoir.
#[test]
fn credit_lifo_reservoir_only() {
    const N: u64 = 5_000;
    let mut db = stub_db();
    let tx = stub_tx();
    let crypto = NativeCrypto;
    let mut vm = VM::new_state_gas_harness(Fork::Amsterdam, &mut db, &tx, &crypto, N);

    let gas_before = vm.frame_gas_remaining();
    vm.increase_state_gas(N).unwrap();

    assert_eq!(
        vm.frame_state_gas_spilled(),
        0,
        "no spill when reservoir covers charge"
    );
    assert_eq!(
        vm.frame_gas_remaining(),
        gas_before,
        "gas_remaining untouched"
    );
    assert_eq!(vm.state_gas_reservoir(), 0, "reservoir fully drawn down");
    assert_eq!(vm.state_gas_spill(), 0);
    assert_eq!(vm.state_gas_used(), as_i64(N));

    vm.credit_state_gas_refund(N).unwrap();

    assert_eq!(
        vm.state_gas_reservoir(),
        N,
        "refund flows back to reservoir"
    );
    assert_eq!(
        vm.frame_gas_remaining(),
        gas_before,
        "gas_remaining still untouched"
    );
    assert_eq!(vm.frame_state_gas_spilled(), 0);
    assert_eq!(vm.state_gas_spill(), 0);
    assert_eq!(vm.state_gas_used(), 0, "state_gas_used back to zero");
}

/// 5.4 — Partial spill: reservoir K covers part, S spills, then refund the whole charge.
///
/// LIFO refund returns the spilled S to `gas_remaining` first and the remaining K to the reservoir.
#[test]
fn credit_lifo_partial_spill() {
    const K: u64 = 3_000;
    const S: u64 = 2_000;
    let mut db = stub_db();
    let tx = stub_tx();
    let crypto = NativeCrypto;
    let mut vm = VM::new_state_gas_harness(Fork::Amsterdam, &mut db, &tx, &crypto, K);

    let gas_before = vm.frame_gas_remaining();
    vm.increase_state_gas(K + S).unwrap();

    assert_eq!(
        vm.frame_state_gas_spilled(),
        S,
        "only the over-reservoir part spills"
    );
    assert_eq!(
        vm.frame_gas_remaining(),
        gas_before - as_i64(S),
        "gas_remaining drops only by the spill S"
    );
    assert_eq!(vm.state_gas_reservoir(), 0, "reservoir fully drawn");
    assert_eq!(vm.state_gas_spill(), S);
    assert_eq!(vm.state_gas_used(), as_i64(K + S));

    vm.credit_state_gas_refund(K + S).unwrap();

    assert_eq!(
        vm.frame_gas_remaining(),
        gas_before,
        "gas_remaining += S (spill returned)"
    );
    assert_eq!(
        vm.state_gas_reservoir(),
        K,
        "remainder K flows to reservoir"
    );
    assert_eq!(vm.frame_state_gas_spilled(), 0, "frame spill drained");
    assert_eq!(vm.state_gas_spill(), 0, "vm-level spill drained");
    assert_eq!(vm.state_gas_used(), 0);
}

/// 5.5 — `refill_frame_state_gas` on a frame that spilled (reservoir empty at entry).
#[test]
fn refill_on_spilled_frame() {
    const N: u64 = 5_000;
    let mut db = stub_db();
    let tx = stub_tx();
    let crypto = NativeCrypto;
    let mut vm = VM::new_state_gas_harness(Fork::Amsterdam, &mut db, &tx, &crypto, 0);

    let gas_before = vm.frame_gas_remaining();
    vm.increase_state_gas(N).unwrap();
    assert_eq!(vm.frame_state_gas_spilled(), N);
    assert_eq!(vm.frame_gas_remaining(), gas_before - as_i64(N));

    vm.refill_frame_state_gas(0).unwrap();

    assert_eq!(
        vm.frame_gas_remaining(),
        gas_before,
        "spilled gas returned to gas_remaining"
    );
    assert_eq!(vm.state_gas_reservoir(), 0, "no spill went to reservoir");
    assert_eq!(
        vm.state_gas_used(),
        0,
        "state_gas_used rolled back to entry"
    );
    assert_eq!(vm.frame_state_gas_spilled(), 0, "frame spill cleared");
    assert_eq!(vm.state_gas_spill(), 0, "vm-level spill cleared");
}

/// 5.5 (no-spill variant) — `refill_frame_state_gas` on a frame whose charge came entirely from
/// the reservoir. The reservoir-sourced portion must flow back to the reservoir.
#[test]
fn refill_on_reservoir_only_frame() {
    const N: u64 = 5_000;
    let mut db = stub_db();
    let tx = stub_tx();
    let crypto = NativeCrypto;
    let mut vm = VM::new_state_gas_harness(Fork::Amsterdam, &mut db, &tx, &crypto, N);

    let gas_before = vm.frame_gas_remaining();
    vm.increase_state_gas(N).unwrap();
    assert_eq!(vm.frame_state_gas_spilled(), 0);
    assert_eq!(vm.state_gas_reservoir(), 0);

    vm.refill_frame_state_gas(0).unwrap();

    assert_eq!(
        vm.state_gas_reservoir(),
        N,
        "reservoir-sourced charge returns to reservoir"
    );
    assert_eq!(
        vm.frame_gas_remaining(),
        gas_before,
        "gas_remaining unchanged (no spill)"
    );
    assert_eq!(
        vm.state_gas_used(),
        0,
        "state_gas_used rolled back to entry"
    );
    assert_eq!(vm.frame_state_gas_spilled(), 0);
    assert_eq!(vm.state_gas_spill(), 0);
}

/// 5.6 — `refill_frame_state_gas` preserves the intrinsic baseline.
///
/// The top frame's entry is seeded at the post-intrinsic `state_gas_used` value (mirrors
/// `add_intrinsic_gas`). A revert/halt refill from that entry must roll back only the
/// execution-portion of the charge and leave the intrinsic portion billed.
#[test]
fn refill_preserves_intrinsic_baseline() {
    const INTRINSIC: i64 = 4_000;
    const EXEC: u64 = 6_000;
    let mut db = stub_db();
    let tx = stub_tx();
    let crypto = NativeCrypto;
    let mut vm = VM::new_state_gas_harness(Fork::Amsterdam, &mut db, &tx, &crypto, 0);

    // Simulate the post-intrinsic baseline: intrinsic state gas accounted for, and the frame's
    // entry snapshot taken at that point.
    vm.seed_state_gas_baseline(INTRINSIC);

    let gas_before = vm.frame_gas_remaining();
    // Execution-time state-gas charge (spills, reservoir is empty).
    vm.increase_state_gas(EXEC).unwrap();
    assert_eq!(vm.state_gas_used(), INTRINSIC + as_i64(EXEC));
    assert_eq!(vm.frame_state_gas_spilled(), EXEC);

    let entry = vm.frame_state_gas_used_at_entry();
    vm.refill_frame_state_gas(entry).unwrap();

    assert_eq!(
        vm.state_gas_used(),
        INTRINSIC,
        "execution portion rolled back, intrinsic preserved"
    );
    assert_eq!(
        vm.frame_gas_remaining(),
        gas_before,
        "spilled execution gas returned to gas_remaining"
    );
    assert_eq!(vm.frame_state_gas_spilled(), 0);
    assert_eq!(vm.state_gas_spill(), 0);
    assert_eq!(vm.state_gas_reservoir(), 0);
}

/// 5.7 — Revert-vs-halt regular-dimension equivalence (method-level proxy).
///
/// A full `vm.execute()` path is not feasible fixture-free here, so this drives the exact two-method
/// sequence the production revert and halt paths use (`increase_state_gas` to spill, then
/// `refill_frame_state_gas` to roll the frame back) and asserts the regular-gas dimension for both
/// a "gas not zeroed" (revert) and a "gas then zeroed" (exceptional halt) sequence.
///
/// The regular-gas dimension is, per `refund_sender`/`default_hook`:
///   regular = (gas_limit - gas_remaining) - state_gas_spill
/// On revert the refilled spill returns to `gas_remaining` AND `state_gas_spill` drops by the same
/// amount, so the spilled gas is refunded to the sender. On exceptional halt the caller zeroes
/// `gas_remaining` after the refill, burning everything left to the regular dimension; but because
/// `refill_frame_state_gas` already decremented `state_gas_spill`, the spilled gas stays counted as
/// regular (burned, not refunded).
#[test]
fn revert_vs_halt_regular_dimension_proxy() {
    const N: u64 = 5_000;

    // Computes the regular-gas dimension exactly as default_hook's refund_sender does.
    fn regular_dimension(vm: &VM<'_>) -> i64 {
        let consumed = as_i64(VM::STATE_GAS_HARNESS_FRAME_GAS) - vm.frame_gas_remaining();
        consumed - as_i64(vm.state_gas_spill())
    }

    // --- Revert path: refill, gas_remaining NOT zeroed ---
    let mut db_r = stub_db();
    let tx_r = stub_tx();
    let crypto_r = NativeCrypto;
    let mut vm_revert = VM::new_state_gas_harness(Fork::Amsterdam, &mut db_r, &tx_r, &crypto_r, 0);
    vm_revert.increase_state_gas(N).unwrap();
    // Mid-spill: the spilled gas is currently counted as regular.
    assert_eq!(
        regular_dimension(&vm_revert),
        0,
        "before refill, spill is netted out of the regular dimension"
    );
    vm_revert.refill_frame_state_gas(0).unwrap();
    // Revert keeps gas_remaining as-is (no zeroing).
    let revert_regular = regular_dimension(&vm_revert);
    assert_eq!(
        revert_regular, 0,
        "revert: spilled gas refunded to sender (regular dimension unchanged at 0)"
    );
    assert_eq!(
        vm_revert.state_gas_spill(),
        0,
        "revert drains vm-level spill"
    );

    // --- Halt path: refill, THEN zero gas_remaining (exceptional halt) ---
    let mut db_h = stub_db();
    let tx_h = stub_tx();
    let crypto_h = NativeCrypto;
    let mut vm_halt = VM::new_state_gas_harness(Fork::Amsterdam, &mut db_h, &tx_h, &crypto_h, 0);
    vm_halt.increase_state_gas(N).unwrap();
    vm_halt.refill_frame_state_gas(0).unwrap();
    // Exceptional halt: caller zeroes gas_remaining after the refill.
    vm_halt.set_frame_gas_remaining(0);
    let halt_regular = regular_dimension(&vm_halt);
    assert_eq!(
        halt_regular,
        as_i64(VM::STATE_GAS_HARNESS_FRAME_GAS),
        "halt: all remaining gas burned to the regular dimension (spilled gas stays burned)"
    );
    assert_eq!(
        vm_halt.state_gas_spill(),
        0,
        "halt also drained vm-level spill"
    );

    // The two paths differ in exactly the regular dimension: revert refunds the spilled gas
    // (sender keeps it), halt burns it.
    assert_ne!(
        revert_regular, halt_regular,
        "revert and halt must produce different regular-gas dimensions"
    );
}

/// 6.2 — Pre-Amsterdam invariance guard.
///
/// Builds a pre-Amsterdam (Prague) VM through the same harness and asserts that every EIP-8037
/// state-gas field is 0 at construction and that nothing seeds them on a pre-Amsterdam fork. The
/// Amsterdam-gated methods are intentionally NOT called here (their `debug_assert!(fork >=
/// Amsterdam)` gates would fire pre-Amsterdam); this proves the no-state-gas path leaves the
/// per-frame and VM-level counters untouched, which every production state-gas call site relies on.
#[test]
fn pre_amsterdam_state_gas_fields_stay_zero() {
    assert!(
        Fork::Prague < Fork::Amsterdam,
        "guard test must run on a pre-Amsterdam fork"
    );

    let mut db = stub_db();
    let tx = stub_tx();
    let crypto = NativeCrypto;
    // Reservoir 0: a pre-Amsterdam VM never funds a state-gas reservoir.
    let vm = VM::new_state_gas_harness(Fork::Prague, &mut db, &tx, &crypto, 0);

    assert_eq!(
        vm.frame_state_gas_spilled(),
        0,
        "per-frame spill must stay 0 pre-Amsterdam"
    );
    assert_eq!(
        vm.state_gas_spill(),
        0,
        "vm-level spill must stay 0 pre-Amsterdam"
    );
    assert_eq!(
        vm.state_gas_reservoir(),
        0,
        "reservoir must stay 0 pre-Amsterdam"
    );
    assert_eq!(
        vm.state_gas_used(),
        0,
        "state_gas_used must stay 0 pre-Amsterdam"
    );
}

/// EIP-8037 #3002 (Case 1, CREATE/CREATE2 success-to-alive-target) — method-level proxy.
///
/// Reproduces the exact two-step the production `generic_create` success arm performs when
/// `target_alive` holds: the unconditional new-account state-gas charge followed by the
/// `if target_alive` refund. Asserts the reservoir and `state_gas_used` are fully restored, i.e.
/// the unconditional charge is net-zero when the target was already alive. Mirrors EELS
/// `generic_create`: `if target_alive: credit_state_gas_refund(evm, StateGasCosts.NEW_ACCOUNT)`.
#[test]
fn create_success_to_alive_target_refund_proxy() {
    const NEW_ACCOUNT: u64 = 7_500;
    let mut db = stub_db();
    let tx = stub_tx();
    let crypto = NativeCrypto;
    // Reservoir large enough that the unconditional charge draws fully from it (no spill).
    let mut vm = VM::new_state_gas_harness(Fork::Amsterdam, &mut db, &tx, &crypto, NEW_ACCOUNT);
    vm.set_state_gas_new_account(NEW_ACCOUNT);

    let gas_before = vm.frame_gas_remaining();
    let reservoir_before = vm.state_gas_reservoir();

    // Top of `generic_create`: charge the new-account state gas unconditionally.
    vm.increase_state_gas(vm.state_gas_new_account()).unwrap();
    assert_eq!(vm.state_gas_used(), as_i64(NEW_ACCOUNT), "charge landed");
    assert_eq!(vm.state_gas_reservoir(), 0, "charge drawn from reservoir");

    // Success arm of `handle_return_create` with `target_alive == true`: refund it.
    vm.credit_state_gas_refund(vm.state_gas_new_account())
        .unwrap();

    assert_eq!(
        vm.state_gas_used(),
        0,
        "alive-target refund makes the new-account charge net-zero"
    );
    assert_eq!(
        vm.state_gas_reservoir(),
        reservoir_before,
        "refund restores the reservoir"
    );
    assert_eq!(
        vm.frame_gas_remaining(),
        gas_before,
        "gas_remaining untouched (charge and refund both via reservoir)"
    );
    assert_eq!(vm.frame_state_gas_spilled(), 0, "no spill to drain");
    assert_eq!(vm.state_gas_spill(), 0);
}

/// EIP-8037 #3002 (Case 1) under state-gas pressure: the unconditional new-account charge spills
/// into `gas_remaining` (reservoir < NEW_ACCOUNT), so the success-arm `credit_state_gas_refund`
/// must restore `gas_remaining` LIFO (spill-drain branch) rather than the reservoir.
#[test]
fn create_success_to_alive_target_refund_proxy_spill() {
    const NEW_ACCOUNT: u64 = 7_500;
    let mut db = stub_db();
    let tx = stub_tx();
    let crypto = NativeCrypto;
    // Reservoir = 0 forces the charge to spill fully into gas_remaining.
    let mut vm = VM::new_state_gas_harness(Fork::Amsterdam, &mut db, &tx, &crypto, 0);
    vm.set_state_gas_new_account(NEW_ACCOUNT);

    let gas_before = vm.frame_gas_remaining();

    vm.increase_state_gas(vm.state_gas_new_account()).unwrap();
    assert_eq!(
        vm.frame_state_gas_spilled(),
        NEW_ACCOUNT,
        "charge fully spilled"
    );
    assert_eq!(
        vm.state_gas_spill(),
        NEW_ACCOUNT,
        "block-accounting spill set"
    );
    assert!(
        vm.frame_gas_remaining() < gas_before,
        "spill drew from gas_remaining"
    );

    vm.credit_state_gas_refund(vm.state_gas_new_account())
        .unwrap();

    assert_eq!(
        vm.state_gas_used(),
        0,
        "alive-target refund makes the new-account charge net-zero"
    );
    assert_eq!(
        vm.frame_gas_remaining(),
        gas_before,
        "refund restored gas_remaining LIFO (spill drained first)"
    );
    assert_eq!(vm.frame_state_gas_spilled(), 0, "spill fully drained");
    assert_eq!(vm.state_gas_spill(), 0, "block-accounting spill cleared");
    assert_eq!(vm.state_gas_reservoir(), 0, "reservoir untouched (was 0)");
}
