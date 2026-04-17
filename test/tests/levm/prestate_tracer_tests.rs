use super::test_db::TestDatabase;
use bytes::Bytes;
use ethrex_common::tracing::PrestateResult;
use ethrex_common::types::{Account, BlockHeader, Code, EIP1559Transaction, Transaction, TxKind};
use ethrex_common::{Address, BigEndianHash, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::vm::VMType;
use ethrex_vm::backends::levm::LEVM;
use once_cell::sync::OnceCell;
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Create an EIP-1559 tx that calls `contract` with 32-byte calldata encoding `slot`.
fn call_contract_tx(contract: Address, sender: Address, slot: H256, nonce: u64) -> Transaction {
    let tx = EIP1559Transaction {
        chain_id: 1,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 10,
        gas_limit: 100_000,
        to: TxKind::Call(contract),
        value: U256::zero(),
        data: Bytes::from(slot.0.to_vec()),
        access_list: vec![],
        signature_y_parity: false,
        signature_r: U256::one(),
        signature_s: U256::one(),
        inner_hash: OnceCell::new(),
        sender_cache: {
            let cell = OnceCell::new();
            let _ = cell.set(sender);
            cell
        },
        cached_canonical: OnceCell::new(),
    };
    Transaction::EIP1559Transaction(tx)
}

fn default_header() -> BlockHeader {
    BlockHeader {
        coinbase: Address::from_low_u64_be(0xCCC),
        base_fee_per_gas: Some(1),
        gas_limit: 30_000_000,
        ..Default::default()
    }
}

/// Contract that reads the slot given in calldata[0..32] and writes 0xFF to it.
///
/// ```text
/// PUSH1 0xFF      60 FF
/// PUSH1 0x00      60 00
/// CALLDATALOAD    35
/// DUP1            80
/// SLOAD           54
/// POP             50
/// SSTORE          55
/// STOP            00
/// ```
fn slot_readwrite_contract(storage: FxHashMap<H256, U256>) -> Account {
    let bytecode = Bytes::from(vec![
        0x60, 0xFF, 0x60, 0x00, 0x35, 0x80, 0x54, 0x50, 0x55, 0x00,
    ]);
    Account::new(
        U256::zero(),
        Code::from_bytecode(bytecode, &NativeCrypto),
        1,
        storage,
    )
}

// ── Tests ────────────────────────────────────────────────────────────────

/// Regression test: when tx A caches account C (loading only slot0), then
/// tx B accesses a NEW slot (slot1) of the same account, the pre-state
/// trace for tx B must include slot1's original value.
///
/// The bug was that `build_pre_state_map` would only look at `pre_snapshot`
/// storage, but `pre_snapshot` only contained slots loaded by previous txs —
/// newly-loaded slots from `initial_accounts_state` were missing.
#[test]
fn prestate_trace_includes_newly_accessed_storage_slots() {
    let contract_addr = Address::from_low_u64_be(0xC000);
    let sender_addr = Address::from_low_u64_be(0x1000);

    let slot0 = H256::from_low_u64_be(0);
    let slot1 = H256::from_low_u64_be(1);

    // Contract has slot0=100, slot1=200 in the backing store
    let mut contract_storage = FxHashMap::default();
    contract_storage.insert(slot0, U256::from(100));
    contract_storage.insert(slot1, U256::from(200));

    let mut accounts = FxHashMap::default();
    accounts.insert(contract_addr, slot_readwrite_contract(contract_storage));
    accounts.insert(
        sender_addr,
        Account::new(
            U256::from(10u64) * U256::from(10u64).pow(U256::from(18)), // 10 ETH
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );

    let test_db = TestDatabase { accounts };
    let mut db = GeneralizedDatabase::new(Arc::new(test_db));

    let header = default_header();

    // Tx A: calls contract with slot0 → loads C into cache with only slot0
    let tx_a = call_contract_tx(contract_addr, sender_addr, slot0, 0);
    LEVM::execute_tx(
        &tx_a,
        sender_addr,
        &header,
        &mut db,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("tx_a should succeed");

    // Verify: slot1 is NOT in current_accounts_state cache (lazy loading)
    assert!(
        !db.current_accounts_state[&contract_addr]
            .storage
            .contains_key(&slot1),
        "slot1 should not be cached yet after tx_a"
    );

    // Tx B: calls contract with slot1 → loads slot1 from DB, writes 0xFF
    let tx_b = call_contract_tx(contract_addr, sender_addr, slot1, 1);
    let result = LEVM::trace_tx_prestate(&mut db, &header, &tx_b, false, VMType::L1, &NativeCrypto)
        .expect("trace should succeed");

    let prestate = match result {
        PrestateResult::Prestate(p) => p,
        PrestateResult::Diff(_) => panic!("expected Prestate variant for non-diff mode"),
    };

    // The pre-state for the contract MUST include slot1's original value (200)
    let contract_state = prestate
        .get(&contract_addr)
        .expect("contract should appear in prestate");

    let slot1_value = contract_state
        .storage
        .get(&slot1)
        .expect("slot1 must be in prestate storage — its original value was 200");

    assert_eq!(
        *slot1_value,
        H256::from_uint(&U256::from(200)),
        "slot1 pre-state should be its original value (200), not the post-tx value"
    );
}

/// Same scenario as above but in diff mode: both pre and post maps
/// must include the newly-accessed slot.
#[test]
fn prestate_diff_mode_includes_newly_accessed_storage_slots() {
    let contract_addr = Address::from_low_u64_be(0xC000);
    let sender_addr = Address::from_low_u64_be(0x1000);

    let slot0 = H256::from_low_u64_be(0);
    let slot1 = H256::from_low_u64_be(1);

    let mut contract_storage = FxHashMap::default();
    contract_storage.insert(slot0, U256::from(100));
    contract_storage.insert(slot1, U256::from(200));

    let mut accounts = FxHashMap::default();
    accounts.insert(contract_addr, slot_readwrite_contract(contract_storage));
    accounts.insert(
        sender_addr,
        Account::new(
            U256::from(10u64) * U256::from(10u64).pow(U256::from(18)),
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );

    let test_db = TestDatabase { accounts };
    let mut db = GeneralizedDatabase::new(Arc::new(test_db));
    let header = default_header();

    // Tx A: cache contract with slot0
    let tx_a = call_contract_tx(contract_addr, sender_addr, slot0, 0);
    LEVM::execute_tx(
        &tx_a,
        sender_addr,
        &header,
        &mut db,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("tx_a should succeed");

    // Tx B: access slot1 (new slot) in diff mode
    let tx_b = call_contract_tx(contract_addr, sender_addr, slot1, 1);
    let result = LEVM::trace_tx_prestate(&mut db, &header, &tx_b, true, VMType::L1, &NativeCrypto)
        .expect("trace should succeed");

    let diff = match result {
        PrestateResult::Diff(d) => d,
        PrestateResult::Prestate(_) => panic!("expected Diff variant for diff mode"),
    };

    // Pre-state must have slot1 = 200 (original)
    let pre_state = diff.pre.get(&contract_addr).expect("contract in pre");
    let pre_val = pre_state
        .storage
        .get(&slot1)
        .expect("slot1 must be in pre storage");
    assert_eq!(*pre_val, H256::from_uint(&U256::from(200)));

    // Post-state must have slot1 = 0xFF (written by contract)
    let post_state = diff.post.get(&contract_addr).expect("contract in post");
    let post_val = post_state
        .storage
        .get(&slot1)
        .expect("slot1 must be in post storage");
    assert_eq!(*post_val, H256::from_uint(&U256::from(0xFF)));
}
