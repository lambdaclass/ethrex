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

/// When tx A touches slot0 of a contract and tx B only touches slot1,
/// the prestate trace for tx B must NOT include slot0.
#[test]
fn prestate_trace_excludes_storage_slots_from_previous_txs() {
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

    // Tx A: touches slot0 → caches contract with slot0
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

    // Tx B: touches only slot1 → should NOT include slot0 in prestate
    let tx_b = call_contract_tx(contract_addr, sender_addr, slot1, 1);
    let result = LEVM::trace_tx_prestate(&mut db, &header, &tx_b, false, VMType::L1, &NativeCrypto)
        .expect("trace should succeed");

    let prestate = match result {
        PrestateResult::Prestate(p) => p,
        PrestateResult::Diff(_) => panic!("expected Prestate variant"),
    };

    let contract_state = prestate
        .get(&contract_addr)
        .expect("contract should appear in prestate");

    // slot1 was accessed by tx B → should be present
    assert!(
        contract_state.storage.contains_key(&slot1),
        "slot1 must be in prestate (accessed by tx B)"
    );

    // slot0 was only accessed by tx A, not tx B → should NOT be present
    assert!(
        !contract_state.storage.contains_key(&slot0),
        "slot0 must NOT be in prestate (only accessed by tx A, not tx B)"
    );
}

/// Newly-created accounts (via CREATE) should appear in diff mode post state.
#[test]
fn prestate_diff_includes_created_account() {
    let sender_addr = Address::from_low_u64_be(0x1000);

    // Contract bytecode: CREATE a child contract that stores 0x42 at slot 0.
    //
    // Child init code (deployed by CREATE):
    //   PUSH1 0x42  PUSH1 0x00  SSTORE      -- store 0x42 at slot 0
    //   PUSH1 0x01  PUSH1 0x00  RETURN       -- return 1 byte of runtime code
    // Hex: 60 42 60 00 55 60 01 60 00 F3
    //
    // Factory bytecode:
    //   PUSH10 <init_code>   PUSH1 0x00  MSTORE   -- store init code in memory
    //   PUSH1 0x0A           PUSH1 0x16  PUSH1 0x00  CREATE   -- create child
    //   STOP
    //
    // The factory stores the 10-byte init code at memory offset 0 (right-padded in the 32-byte word),
    // then calls CREATE with offset=22 (0x16), size=10 (0x0A) to deploy the child.
    let init_code: [u8; 10] = [0x60, 0x42, 0x60, 0x00, 0x55, 0x60, 0x01, 0x60, 0x00, 0xF3];
    let mut factory_bytecode = vec![0x69]; // PUSH10
    factory_bytecode.extend_from_slice(&init_code);
    factory_bytecode.extend_from_slice(&[
        0x60, 0x00, // PUSH1 0x00
        0x52, // MSTORE (stores at offset 0, 32 bytes, init_code is right-padded)
        0x60, 0x0A, // PUSH1 0x0A (size = 10)
        0x60, 0x16, // PUSH1 0x16 (offset = 22, since MSTORE pads left)
        0x60, 0x00, // PUSH1 0x00 (value = 0)
        0xF0, // CREATE
        0x00, // STOP
    ]);

    let factory_addr = Address::from_low_u64_be(0xF000);

    let mut accounts = FxHashMap::default();
    accounts.insert(
        factory_addr,
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from(factory_bytecode), &NativeCrypto),
            1,
            FxHashMap::default(),
        ),
    );
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

    // Call the factory — creates a child contract
    let tx = {
        let inner = EIP1559Transaction {
            chain_id: 1,
            nonce: 0,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 10,
            gas_limit: 500_000,
            to: TxKind::Call(factory_addr),
            value: U256::zero(),
            data: Bytes::new(),
            access_list: vec![],
            signature_y_parity: false,
            signature_r: U256::one(),
            signature_s: U256::one(),
            inner_hash: OnceCell::new(),
            sender_cache: {
                let cell = OnceCell::new();
                let _ = cell.set(sender_addr);
                cell
            },
            cached_canonical: OnceCell::new(),
        };
        Transaction::EIP1559Transaction(inner)
    };

    let result = LEVM::trace_tx_prestate(&mut db, &header, &tx, true, VMType::L1, &NativeCrypto)
        .expect("trace should succeed");

    let diff = match result {
        PrestateResult::Diff(d) => d,
        PrestateResult::Prestate(_) => panic!("expected Diff variant"),
    };

    // The factory's nonce should be incremented (it called CREATE)
    let factory_post = diff.post.get(&factory_addr).expect("factory in post");
    assert_eq!(
        factory_post.nonce, 2,
        "factory nonce should be 2 after CREATE (started at 1)"
    );

    // The child account should appear in both pre (empty state) and post (created state).
    // Find it as the address with nonce=0 in pre and nonce=1 in post that isn't sender/factory/coinbase.
    let known_addrs = [sender_addr, factory_addr, header.coinbase];
    let child_addr = diff
        .post
        .keys()
        .find(|addr| !known_addrs.contains(addr))
        .expect("child contract should appear in post state");

    let child_pre = diff
        .pre
        .get(child_addr)
        .expect("child should appear in pre (empty state before creation)");
    assert_eq!(child_pre.nonce, 0, "child pre-state nonce should be 0");
    assert_eq!(
        child_pre.balance,
        U256::zero(),
        "child pre-state balance should be 0"
    );

    let child_post = diff
        .post
        .get(child_addr)
        .expect("child should appear in post");
    assert_eq!(
        child_post.nonce, 1,
        "child post-state nonce should be 1 after creation"
    );
}
