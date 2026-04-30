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
    let result = LEVM::trace_tx_prestate(
        &mut db,
        &header,
        &tx_b,
        false,
        false,
        VMType::L1,
        &NativeCrypto,
    )
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
    let result = LEVM::trace_tx_prestate(
        &mut db,
        &header,
        &tx_b,
        true,
        false,
        VMType::L1,
        &NativeCrypto,
    )
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
    let result = LEVM::trace_tx_prestate(
        &mut db,
        &header,
        &tx_b,
        false,
        false,
        VMType::L1,
        &NativeCrypto,
    )
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

    let result = LEVM::trace_tx_prestate(
        &mut db,
        &header,
        &tx,
        true,
        false,
        VMType::L1,
        &NativeCrypto,
    )
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

    // Find the child address as the only newly-touched address in post that isn't
    // sender/factory/coinbase.
    let known_addrs = [sender_addr, factory_addr, header.coinbase];
    let child_addr = diff
        .post
        .keys()
        .find(|addr| !known_addrs.contains(addr))
        .copied()
        .expect("child contract should appear in post state");

    // Diff mode drops accounts whose pre-state was empty (zero balance, zero nonce,
    // no code, no storage worth keeping). A newly-CREATE'd account fits that, so it
    // should be absent from pre even though it appears in post.
    assert!(
        !diff.pre.contains_key(&child_addr),
        "newly-created child should NOT appear in diff pre (its pre-state is empty)"
    );

    let child_post = diff
        .post
        .get(&child_addr)
        .expect("child should appear in post");
    assert_eq!(
        child_post.nonce, 1,
        "child post-state nonce should be 1 after creation"
    );
    assert!(
        !child_post.code.is_empty(),
        "child post-state code should be the deployed runtime"
    );
}

/// Read-only access: a contract whose state isn't modified by the tx must appear
/// in non-diff `pre` (every accessed account is captured), but must be absent from
/// both `pre` and `post` in diff mode (unmodified accounts are pruned in diff output).
#[test]
fn prestate_trace_includes_read_only_account() {
    // Oracle: read slot from calldata, SLOAD it, return the value. No SSTORE.
    //   PUSH1 0x00   60 00     ; calldata offset
    //   CALLDATALOAD 35        ; -> slot
    //   SLOAD        54        ; -> value
    //   PUSH1 0x00   60 00
    //   MSTORE       52        ; mem[0..32] = value
    //   PUSH1 0x20   60 20
    //   PUSH1 0x00   60 00
    //   RETURN       F3
    let oracle_bytecode = Bytes::from(vec![
        0x60, 0x00, 0x35, 0x54, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xF3,
    ]);

    let oracle_addr = Address::from_low_u64_be(0xF000);
    let sender_addr = Address::from_low_u64_be(0x1000);

    let slot0 = H256::from_low_u64_be(0);
    let oracle_value = U256::from(42);

    let mut oracle_storage = FxHashMap::default();
    oracle_storage.insert(slot0, oracle_value);

    let mut accounts = FxHashMap::default();
    accounts.insert(
        oracle_addr,
        Account::new(
            U256::zero(),
            Code::from_bytecode(oracle_bytecode.clone(), &NativeCrypto),
            1,
            oracle_storage,
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

    // ── non-diff mode: oracle must appear in pre with code + slot0 ─────────
    {
        let test_db = TestDatabase {
            accounts: accounts.clone(),
        };
        let mut db = GeneralizedDatabase::new(Arc::new(test_db));
        let header = default_header();

        let tx = call_contract_tx(oracle_addr, sender_addr, slot0, 0);
        let result = LEVM::trace_tx_prestate(
            &mut db,
            &header,
            &tx,
            false,
            false,
            VMType::L1,
            &NativeCrypto,
        )
        .expect("trace should succeed");

        let prestate = match result {
            PrestateResult::Prestate(p) => p,
            PrestateResult::Diff(_) => panic!("expected Prestate variant"),
        };

        let oracle_state = prestate
            .get(&oracle_addr)
            .expect("oracle must appear in prestate even though its state didn't change");
        assert_eq!(
            oracle_state.code, oracle_bytecode,
            "oracle code must be present in prestate"
        );
        let slot0_val = oracle_state
            .storage
            .get(&slot0)
            .expect("oracle slot0 (read by SLOAD) must appear in prestate storage");
        assert_eq!(*slot0_val, H256::from_uint(&oracle_value));
    }

    // ── diff mode: oracle is unmodified → absent from BOTH pre and post ───
    {
        let test_db = TestDatabase { accounts };
        let mut db = GeneralizedDatabase::new(Arc::new(test_db));
        let header = default_header();

        let tx = call_contract_tx(oracle_addr, sender_addr, slot0, 0);
        let result = LEVM::trace_tx_prestate(
            &mut db,
            &header,
            &tx,
            true,
            false,
            VMType::L1,
            &NativeCrypto,
        )
        .expect("trace should succeed");

        let diff = match result {
            PrestateResult::Diff(d) => d,
            PrestateResult::Prestate(_) => panic!("expected Diff variant"),
        };

        assert!(
            !diff.pre.contains_key(&oracle_addr),
            "oracle must NOT appear in diff pre (state was unchanged)"
        );
        assert!(
            !diff.post.contains_key(&oracle_addr),
            "oracle must NOT appear in diff post (state was unchanged)"
        );
    }
}

/// Geth `processDiffState` filters slots whose post value equals the pre value.
/// When a contract is first accessed in this tx and SLOADs slot A while SSTORE-ing slot B,
/// only slot B should appear in diff `post` — slot A was read-only.
#[test]
fn prestate_diff_post_excludes_unchanged_storage_for_newly_accessed_account() {
    // Contract: SLOAD slot0 (discarded), SSTORE 0xFF to slot1, STOP.
    //   60 00 PUSH1 0    ; slot 0
    //   54    SLOAD
    //   50    POP        ; discard
    //   60 FF PUSH1 0xFF ; value
    //   60 01 PUSH1 1    ; slot 1
    //   55    SSTORE
    //   00    STOP
    let bytecode = Bytes::from(vec![
        0x60, 0x00, 0x54, 0x50, 0x60, 0xFF, 0x60, 0x01, 0x55, 0x00,
    ]);
    let contract_addr = Address::from_low_u64_be(0xC000);
    let sender_addr = Address::from_low_u64_be(0x1000);

    let slot0 = H256::from_low_u64_be(0);
    let slot1 = H256::from_low_u64_be(1);

    let mut contract_storage = FxHashMap::default();
    contract_storage.insert(slot0, U256::from(100));
    contract_storage.insert(slot1, U256::from(200));

    let mut accounts = FxHashMap::default();
    accounts.insert(
        contract_addr,
        Account::new(
            U256::zero(),
            Code::from_bytecode(bytecode, &NativeCrypto),
            1,
            contract_storage,
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

    let tx = call_contract_tx(contract_addr, sender_addr, slot0, 0);
    let result = LEVM::trace_tx_prestate(
        &mut db,
        &header,
        &tx,
        true,
        false,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace should succeed");

    let diff = match result {
        PrestateResult::Diff(d) => d,
        PrestateResult::Prestate(_) => panic!("expected Diff variant"),
    };

    let post = diff
        .post
        .get(&contract_addr)
        .expect("contract should appear in post (slot1 was modified)");

    assert!(
        !post.storage.contains_key(&slot0),
        "slot0 was SLOAD-only — must NOT appear in diff post"
    );
    let slot1_post = post
        .storage
        .get(&slot1)
        .expect("slot1 was SSTORE'd — must appear in diff post");
    assert_eq!(*slot1_post, H256::from_uint(&U256::from(0xFF)));
}

/// Geth keeps zero-valued accessed slots in non-diff `pre` (the original SLOAD value
/// of an empty slot is `0x0`, and that's what's recorded). Test that ethrex now
/// matches by including the zero pre value of a slot that SLOAD'd from an empty store.
#[test]
fn prestate_trace_includes_zero_value_storage_in_non_diff_pre() {
    // Contract: SLOAD slot0, POP, STOP.
    //   60 00 PUSH1 0
    //   54    SLOAD
    //   50    POP
    //   00    STOP
    let bytecode = Bytes::from(vec![0x60, 0x00, 0x54, 0x50, 0x00]);
    let contract_addr = Address::from_low_u64_be(0xC000);
    let sender_addr = Address::from_low_u64_be(0x1000);

    let slot0 = H256::from_low_u64_be(0);

    // Contract storage is intentionally empty — slot0 reads as zero from the DB.
    let mut accounts = FxHashMap::default();
    accounts.insert(
        contract_addr,
        Account::new(
            U256::zero(),
            Code::from_bytecode(bytecode, &NativeCrypto),
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

    let tx = call_contract_tx(contract_addr, sender_addr, slot0, 0);
    let result = LEVM::trace_tx_prestate(
        &mut db,
        &header,
        &tx,
        false,
        false,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace should succeed");

    let prestate = match result {
        PrestateResult::Prestate(p) => p,
        PrestateResult::Diff(_) => panic!("expected Prestate variant"),
    };

    let contract_state = prestate
        .get(&contract_addr)
        .expect("contract should appear in prestate");

    let slot0_val = contract_state
        .storage
        .get(&slot0)
        .expect("zero-valued accessed slot must be present in non-diff pre");
    assert_eq!(*slot0_val, H256::zero());
}

/// Pre-state of a contract account carries its code hash alongside the bytecode.
#[test]
fn prestate_trace_includes_code_hash_for_contract_account() {
    let bytecode = Bytes::from(vec![0x60, 0x00, 0x54, 0x50, 0x00]);
    let contract_addr = Address::from_low_u64_be(0xC000);
    let sender_addr = Address::from_low_u64_be(0x1000);
    let slot0 = H256::from_low_u64_be(0);

    let code = Code::from_bytecode(bytecode.clone(), &NativeCrypto);
    let expected_code_hash = code.hash;

    let mut accounts = FxHashMap::default();
    accounts.insert(
        contract_addr,
        Account::new(U256::zero(), code, 1, FxHashMap::default()),
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

    let tx = call_contract_tx(contract_addr, sender_addr, slot0, 0);
    let result = LEVM::trace_tx_prestate(
        &mut db,
        &header,
        &tx,
        false,
        false,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace should succeed");

    let prestate = match result {
        PrestateResult::Prestate(p) => p,
        PrestateResult::Diff(_) => panic!("expected Prestate variant"),
    };

    let contract_state = prestate
        .get(&contract_addr)
        .expect("contract should appear in prestate");
    assert_eq!(
        contract_state.code_hash, expected_code_hash,
        "contract pre-state must carry the code hash"
    );
    assert_eq!(contract_state.code, bytecode);
}

/// An account whose pre-state is fully default (no code, no nonce, no balance, no storage)
/// must be dropped from non-diff pre when `include_empty` is false. Setting `include_empty`
/// keeps it in the map.
#[test]
fn prestate_trace_filters_empty_pre_account_unless_include_empty() {
    let empty_addr = Address::from_low_u64_be(0xDEAD);
    let sender_addr = Address::from_low_u64_be(0x1000);
    let dummy_slot = H256::from_low_u64_be(0);

    let mut accounts = FxHashMap::default();
    accounts.insert(
        sender_addr,
        Account::new(
            U256::from(10u64) * U256::from(10u64).pow(U256::from(18)),
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );
    // No entry for empty_addr — read returns default (no code, no balance, nonce 0).

    // include_empty = false → empty_addr filtered.
    {
        let test_db = TestDatabase {
            accounts: accounts.clone(),
        };
        let mut db = GeneralizedDatabase::new(Arc::new(test_db));
        let header = default_header();

        let tx = call_contract_tx(empty_addr, sender_addr, dummy_slot, 0);
        let result = LEVM::trace_tx_prestate(
            &mut db,
            &header,
            &tx,
            false,
            false,
            VMType::L1,
            &NativeCrypto,
        )
        .expect("trace should succeed");

        let prestate = match result {
            PrestateResult::Prestate(p) => p,
            PrestateResult::Diff(_) => panic!("expected Prestate variant"),
        };

        assert!(
            !prestate.contains_key(&empty_addr),
            "empty account must be filtered from non-diff pre when include_empty=false"
        );
    }

    // include_empty = true → empty_addr kept.
    {
        let test_db = TestDatabase { accounts };
        let mut db = GeneralizedDatabase::new(Arc::new(test_db));
        let header = default_header();

        let tx = call_contract_tx(empty_addr, sender_addr, dummy_slot, 0);
        let result = LEVM::trace_tx_prestate(
            &mut db,
            &header,
            &tx,
            false,
            true,
            VMType::L1,
            &NativeCrypto,
        )
        .expect("trace should succeed");

        let prestate = match result {
            PrestateResult::Prestate(p) => p,
            PrestateResult::Diff(_) => panic!("expected Prestate variant"),
        };

        assert!(
            prestate.contains_key(&empty_addr),
            "empty account must be retained when include_empty=true"
        );
    }
}

/// Diff post entries carry only the fields whose value actually changed. A contract that
/// only receives ETH (balance changes, nonce stays at 1, code unchanged) must serialize
/// with `balance` set and `nonce` / `code` / `code_hash` at their default (skipped).
#[test]
fn prestate_diff_post_emits_only_changed_fields() {
    // Contract whose runtime accepts incoming calls without reverting.
    //   STOP (00)
    let bytecode = Bytes::from(vec![0x00]);
    let contract_addr = Address::from_low_u64_be(0xC000);
    let sender_addr = Address::from_low_u64_be(0x1000);
    let dummy_slot = H256::from_low_u64_be(0);

    let mut accounts = FxHashMap::default();
    accounts.insert(
        contract_addr,
        Account::new(
            U256::zero(),
            Code::from_bytecode(bytecode, &NativeCrypto),
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

    // Tx that sends 1 wei to the contract — only its balance should change.
    let tx = {
        let inner = EIP1559Transaction {
            chain_id: 1,
            nonce: 0,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 10,
            gas_limit: 100_000,
            to: TxKind::Call(contract_addr),
            value: U256::one(),
            data: Bytes::from(dummy_slot.0.to_vec()),
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

    let result = LEVM::trace_tx_prestate(
        &mut db,
        &header,
        &tx,
        true,
        false,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace should succeed");

    let diff = match result {
        PrestateResult::Diff(d) => d,
        PrestateResult::Prestate(_) => panic!("expected Diff variant"),
    };

    let post = diff
        .post
        .get(&contract_addr)
        .expect("contract should appear in diff post (balance changed)");

    assert_eq!(
        post.balance,
        Some(U256::one()),
        "balance change must be emitted"
    );
    assert_eq!(
        post.nonce, 0,
        "nonce did not change → must be at default (skipped from JSON)"
    );
    assert!(
        post.code.is_empty(),
        "code did not change → must be at default (skipped from JSON)"
    );
    assert!(
        post.code_hash.is_zero(),
        "code_hash did not change → must be at default (skipped from JSON)"
    );
    assert!(
        post.storage.is_empty(),
        "no storage change → storage map must be empty"
    );
}

/// When an account's balance changes to exactly zero, the post entry must still
/// carry `Some(0)` so JSON consumers see `"balance": "0x0"`. Dropping the field
/// would silently hide the balance change.
#[test]
fn prestate_diff_post_emits_zero_balance_when_changed() {
    // Sender drains its entire balance into the recipient via `value`.
    // After paying gas + transferring, sender's balance won't be exactly zero
    // (gas refund, fees), so we exercise the contract side: a contract with
    // a non-zero starting balance whose runtime drains itself to a third party.
    //
    // Drain bytecode: CALL(beneficiary, balance, ...) with all funds, return.
    //   PUSH1 0x00          ; retLen
    //   PUSH1 0x00          ; retOff
    //   PUSH1 0x00          ; argLen
    //   PUSH1 0x00          ; argOff
    //   SELFBALANCE         ; value = self balance
    //   PUSH20 <addr>       ; to
    //   GAS                 ; gas
    //   CALL                ; pops: gas, to, value, argOff, argLen, retOff, retLen
    //   STOP
    let beneficiary = Address::from_low_u64_be(0xBEEF);
    let mut bytecode = vec![
        0x60, 0x00, // PUSH1 0
        0x60, 0x00, // PUSH1 0
        0x60, 0x00, // PUSH1 0
        0x60, 0x00, // PUSH1 0
        0x47, // SELFBALANCE
        0x73, // PUSH20
    ];
    bytecode.extend_from_slice(beneficiary.as_bytes());
    bytecode.extend_from_slice(&[
        0x5A, // GAS
        0xF1, // CALL
        0x00, // STOP
    ]);

    let contract_addr = Address::from_low_u64_be(0xC000);
    let sender_addr = Address::from_low_u64_be(0x1000);
    let dummy_slot = H256::from_low_u64_be(0);

    let mut accounts = FxHashMap::default();
    accounts.insert(
        contract_addr,
        Account::new(
            U256::from(1_000_000u64),
            Code::from_bytecode(Bytes::from(bytecode), &NativeCrypto),
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

    let tx = call_contract_tx(contract_addr, sender_addr, dummy_slot, 0);
    let result = LEVM::trace_tx_prestate(
        &mut db,
        &header,
        &tx,
        true,
        false,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace should succeed");

    let diff = match result {
        PrestateResult::Diff(d) => d,
        PrestateResult::Prestate(_) => panic!("expected Diff variant"),
    };

    let post = diff
        .post
        .get(&contract_addr)
        .expect("contract whose balance changed must appear in diff post");

    assert_eq!(
        post.balance,
        Some(U256::zero()),
        "balance change to zero must be emitted as Some(0), not omitted"
    );
}

/// Defensive: even if `initial_accounts_state.storage[addr]` ever held slots that the
/// current tx didn't access (e.g. via more eager upstream caching), pre output for that
/// account must not leak them. The pre map is bounded by what the current tx actually
/// touched (which is reflected in `post.storage`). Today the upstream cache only holds
/// this-tx slots, so this scenario is constructed by manually planting an extra slot
/// into `initial_accounts_state` before tracing.
#[test]
fn prestate_pre_storage_excludes_slots_not_present_in_post() {
    let contract_addr = Address::from_low_u64_be(0xC000);
    let sender_addr = Address::from_low_u64_be(0x1000);
    let extra_slot = H256::from_low_u64_be(0x42);
    let dummy_slot = H256::from_low_u64_be(0);

    // Contract whose runtime is just STOP — never touches storage on call.
    let mut accounts = FxHashMap::default();
    accounts.insert(
        contract_addr,
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from(vec![0x00]), &NativeCrypto),
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

    // Run a first tx so the contract is cached in both initial and current state.
    let warmup = call_contract_tx(contract_addr, sender_addr, dummy_slot, 0);
    LEVM::execute_tx(
        &warmup,
        sender_addr,
        &header,
        &mut db,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("warmup tx should succeed");

    // Plant an extra slot into `initial_accounts_state` only — the equivalent of an
    // upstream change that pre-loads slots the current tx never asks for.
    db.initial_accounts_state
        .get_mut(&contract_addr)
        .expect("contract must be in initial after warmup")
        .storage
        .insert(extra_slot, U256::from(0x99));

    // Trace a second tx that touches the contract again. The contract's bytecode is
    // STOP, so the second call accesses nothing storage-side; pre output for the
    // contract should not include `extra_slot`.
    let traced = call_contract_tx(contract_addr, sender_addr, dummy_slot, 1);
    let result = LEVM::trace_tx_prestate(
        &mut db,
        &header,
        &traced,
        false,
        true,
        VMType::L1,
        &NativeCrypto,
    )
    .expect("trace should succeed");

    let prestate = match result {
        PrestateResult::Prestate(p) => p,
        PrestateResult::Diff(_) => panic!("expected Prestate variant"),
    };

    if let Some(contract_state) = prestate.get(&contract_addr) {
        assert!(
            !contract_state.storage.contains_key(&extra_slot),
            "extra_slot was never accessed by this tx — it must not appear in pre"
        );
    }
}
