//! EIP-8037 top-level reservoir reset tests (execution-specs PR #2689).
//!
//! Verifies that when a top-level transaction fails (revert, exceptional halt, or OOG),
//! the execution portion of state gas is returned to the reservoir and only intrinsic
//! state gas stays charged in block accounting.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{
        Account, AccountState, ChainConfig, Code, CodeMetadata, EIP1559Transaction, Fork,
        Transaction, TxKind,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    constants::TX_MAX_GAS_LIMIT_AMSTERDAM,
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    errors::{DatabaseError, ExecutionReport},
    gas_cost::{
        SSTORE_COLD_DYNAMIC, SSTORE_STORAGE_MODIFICATION, STATE_BYTES_PER_STORAGE_SET,
        cost_per_state_byte,
    },
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ==================== Test Database ====================

struct TestDatabase {
    accounts: FxHashMap<Address, Account>,
}

impl TestDatabase {
    fn new() -> Self {
        Self {
            accounts: FxHashMap::default(),
        }
    }
}

impl Database for TestDatabase {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        Ok(self
            .accounts
            .get(&address)
            .map(|acc| AccountState {
                nonce: acc.info.nonce,
                balance: acc.info.balance,
                storage_root: *EMPTY_TRIE_HASH,
                code_hash: acc.info.code_hash,
            })
            .unwrap_or_default())
    }

    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        Ok(self
            .accounts
            .get(&address)
            .and_then(|acc| acc.storage.get(&key).copied())
            .unwrap_or_default())
    }

    fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
        Ok(H256::zero())
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Ok(ChainConfig::default())
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        for acc in self.accounts.values() {
            if acc.info.code_hash == code_hash {
                return Ok(acc.code.clone());
            }
        }
        Ok(Code::default())
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        for acc in self.accounts.values() {
            if acc.info.code_hash == code_hash {
                return Ok(CodeMetadata {
                    length: acc.code.bytecode.len() as u64,
                });
            }
        }
        Ok(CodeMetadata { length: 0 })
    }
}

// ==================== Constants ====================

const SENDER: u64 = 0x1000;
const CONTRACT_A: u64 = 0x2000;
const CONTRACT_B: u64 = 0x3000;
// GAS_LIMIT large enough for execution but not so large that cpsb becomes significant.
// block_gas_limit = GAS_LIMIT * 2 = 1_000_000.
// NOTE (bal-devnet-4 CPSB pin): cost_per_state_byte is currently fixed at 1174.
// With the dynamic formula, cost_per_state_byte(1_000_000) = 1 → state_gas_storage_set = 32.
// These tests compute amounts via the live function so they pass either way.
const GAS_LIMIT: u64 = 500_000;

// ==================== Bytecode helpers ====================

/// PUSH1 value, PUSH1 slot, SSTORE
fn sstore_byte(slot: u8, value: u8) -> Vec<u8> {
    vec![0x60, value, 0x60, slot, 0x55]
}

/// STOP
fn stop() -> Vec<u8> {
    vec![0x00]
}

/// REVERT(0, 0)
fn revert_bytecode() -> Vec<u8> {
    vec![0x60, 0x00, 0x60, 0x00, 0xfd]
}

/// INVALID (0xfe) — causes exceptional halt
fn invalid_bytecode() -> Vec<u8> {
    vec![0xfe]
}

/// CALL target with no value, collecting return data
fn call_bytecode(target: Address) -> Vec<u8> {
    let mut b = vec![0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00];
    b.push(0x73);
    b.extend_from_slice(target.as_bytes());
    b.push(0x5a); // GAS
    b.push(0xf1); // CALL
    b.push(0x50); // POP
    b
}

/// Inline CREATE-with-failing-initcode bytecode.
///
/// Stores a one-byte initcode at memory[0] and invokes CREATE(value=0, offset=0, size=1).
/// The chosen initcode byte determines how the child frame ends:
/// - 0xfe (INVALID) → exceptional halt
/// - 0xfd (REVERT)  → revert (note: REVERT alone with empty stack is itself a halt)
fn create_failing_bytecode(initcode_byte: u8) -> Vec<u8> {
    vec![
        0x60, initcode_byte, // PUSH1 <byte>
        0x60, 0x00, // PUSH1 0
        0x53, // MSTORE8 — memory[0] = byte
        0x60, 0x01, // PUSH1 1   (size)
        0x60, 0x00, // PUSH1 0   (offset)
        0x60, 0x00, // PUSH1 0   (value)
        0xf0, // CREATE
        0x50, // POP (discard returned address / 0)
    ]
}

/// RETURN(0, 0)
fn return_bytecode() -> Vec<u8> {
    vec![0x60, 0x00, 0x60, 0x00, 0xf3]
}

// ==================== Test runner ====================

fn eoa(balance: U256) -> Account {
    Account::new(balance, Code::default(), 0, FxHashMap::default())
}

fn contract(code: Vec<u8>) -> Account {
    Account::new(
        U256::zero(),
        Code::from_bytecode(Bytes::from(code), &NativeCrypto),
        1,
        FxHashMap::default(),
    )
}

struct TestRunner {
    accounts: Vec<(Address, Account)>,
    target: Address,
    is_create: bool,
    calldata: Bytes,
    gas_limit_override: Option<u64>,
    block_gas_limit_override: Option<u64>,
}

impl TestRunner {
    fn call(target: Address) -> Self {
        Self {
            accounts: Vec::new(),
            target,
            is_create: false,
            calldata: Bytes::new(),
            gas_limit_override: None,
            block_gas_limit_override: None,
        }
    }

    fn create(initcode: Vec<u8>) -> Self {
        Self {
            accounts: Vec::new(),
            target: Address::default(),
            is_create: true,
            calldata: Bytes::from(initcode),
            gas_limit_override: None,
            block_gas_limit_override: None,
        }
    }

    fn with_account(mut self, addr: Address, acc: Account) -> Self {
        self.accounts.push((addr, acc));
        self
    }

    fn with_gas_limit(mut self, gas_limit: u64) -> Self {
        self.gas_limit_override = Some(gas_limit);
        self
    }

    fn with_block_gas_limit(mut self, block_gas_limit: u64) -> Self {
        self.block_gas_limit_override = Some(block_gas_limit);
        self
    }

    fn run(self) -> ExecutionReport {
        let gas_limit = self.gas_limit_override.unwrap_or(GAS_LIMIT);
        let block_gas_limit = self.block_gas_limit_override.unwrap_or(GAS_LIMIT * 2);
        let test_db = TestDatabase::new();
        let accounts_map: FxHashMap<Address, Account> = self.accounts.into_iter().collect();
        let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(test_db), accounts_map);

        let fork = Fork::Amsterdam;
        let blob_schedule = EVMConfig::canonical_values(fork);
        let env = Environment {
            origin: Address::from_low_u64_be(SENDER),
            gas_limit,
            config: EVMConfig::new(fork, blob_schedule),
            block_number: 1,
            coinbase: Address::from_low_u64_be(0xCCC),
            timestamp: 1000,
            prev_randao: Some(H256::zero()),
            difficulty: U256::zero(),
            slot_number: U256::zero(),
            chain_id: U256::from(1),
            base_fee_per_gas: U256::zero(),
            base_blob_fee_per_gas: U256::from(1),
            gas_price: U256::zero(),
            block_excess_blob_gas: None,
            block_blob_gas_used: None,
            tx_blob_hashes: vec![],
            tx_max_priority_fee_per_gas: None,
            tx_max_fee_per_gas: Some(U256::zero()),
            tx_max_fee_per_blob_gas: None,
            tx_nonce: 0,
            block_gas_limit,
            is_privileged: false,
            fee_token: None,
            disable_balance_check: true,
            is_system_call: false,
        };

        let tx = if self.is_create {
            Transaction::EIP1559Transaction(EIP1559Transaction {
                to: TxKind::Create,
                value: U256::zero(),
                data: self.calldata,
                gas_limit,
                max_fee_per_gas: 0,
                max_priority_fee_per_gas: 0,
                ..Default::default()
            })
        } else {
            Transaction::EIP1559Transaction(EIP1559Transaction {
                to: TxKind::Call(self.target),
                value: U256::zero(),
                data: Bytes::new(),
                gas_limit,
                max_fee_per_gas: 0,
                max_priority_fee_per_gas: 0,
                ..Default::default()
            })
        };

        let mut vm = VM::new(
            env,
            &mut db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
            &NativeCrypto,
        )
        .unwrap();
        vm.execute().unwrap()
    }
}

// ==================== Helper: compute expected state gas per storage set ====================

/// For block_gas_limit = GAS_LIMIT * 2 = 1_000_000:
/// - With the dynamic formula: cost_per_state_byte = 1, state_gas_storage_set = 32.
/// - With the bal-devnet-4 CPSB pin: cost_per_state_byte = 1174, state_gas_storage_set = 37_568.
/// The function computes the value live so callers stay correct under both regimes.
fn state_gas_storage_set() -> u64 {
    let cpsb = cost_per_state_byte(GAS_LIMIT * 2);
    STATE_BYTES_PER_STORAGE_SET * cpsb
}

// ==================== Test 1a: Top-level revert refunds execution state gas ====================

/// When a tx SSSTOREs (charges state gas) then top-level REVERTs, the execution state gas
/// must be refunded: state_gas_used in the report should NOT include the SSTORE charge.
#[test]
fn test_top_level_revert_refunds_execution_state_gas() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);

    // SSTORE(slot 0 = 5) then REVERT
    let mut code = sstore_byte(0, 5);
    code.extend(revert_bytecode());

    let report = TestRunner::call(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code))
        .run();

    assert!(
        !report.is_success(),
        "transaction should have reverted: {:?}",
        report.result
    );
    // Execution state gas (SSTORE charge) must be zero after top-level failure.
    assert_eq!(
        report.state_gas_used, 0,
        "state_gas_used should be 0 after top-level REVERT (no intrinsic state gas for plain CALL)"
    );
}

// ==================== Test 1b: Top-level exceptional halt refunds execution state gas ====================

/// When a tx SSSTOREs then hits INVALID (exceptional halt), execution state gas is refunded.
#[test]
fn test_top_level_halt_refunds_execution_state_gas() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);

    // SSTORE(slot 0 = 5) then INVALID
    let mut code = sstore_byte(0, 5);
    code.extend(invalid_bytecode());

    let report = TestRunner::call(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code))
        .run();

    assert!(
        !report.is_success(),
        "transaction should have halted: {:?}",
        report.result
    );
    assert_eq!(
        report.state_gas_used, 0,
        "state_gas_used should be 0 after top-level INVALID halt"
    );
}

// ==================== Test 1c: Top-level OOG refunds execution state gas ====================

/// When a tx charges state gas via SSTORE then the outer execution OOGs, state gas is refunded.
///
/// Calibration (Amsterdam, block_gas_limit = GAS_LIMIT * 2 = 1_000_000, cpsb = 1):
///   intrinsic_regular = TX_BASE(21_000)  [plain CALL, no calldata]
///   execution sequence: PUSH1(3) + PUSH1(3) + SSTORE-regular(5000) + SSTORE-state(32, spills)
///   reservoir = 0 (gas_limit << TX_MAX_GAS_LIMIT_AMSTERDAM = 16_777_216)
///   After SSTORE regular: gas_remaining = gas_limit - 21_006 - 5000 = gas_limit - 26_006
///   OOG fires on state spill when gas_limit - 26_006 < 32  →  gas_limit < 26_038
///   Must succeed for SSTORE regular: gas_limit - 21_006 >= 5000  →  gas_limit >= 26_006
///   Use gas_limit = 26_031: gas_remaining after SSTORE regular = 25 < 32 → OOG deterministic.
#[test]
fn test_top_level_oog_refunds_execution_state_gas() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);

    // SSTORE(slot 0 = 5): [PUSH1 5, PUSH1 0, SSTORE] — 3 opcodes, regular gas = 3+3+5000
    // With gas_limit = 26_031:
    //   reservoir = 0 (execution_gas << TX_MAX_GAS_LIMIT_AMSTERDAM)
    //   after PUSH1+PUSH1+SSTORE-regular: gas_remaining = 26_031 - 21_000 - 6 - 5000 = 25
    //   state spill = 32 > 25 → OOG
    let code = sstore_byte(0, 5);

    // sstore_regular_cold_new_slot = SSTORE_STORAGE_MODIFICATION + SSTORE_COLD_DYNAMIC = 5000
    let sstore_regular = SSTORE_STORAGE_MODIFICATION + SSTORE_COLD_DYNAMIC;
    // 2 PUSH1 instructions before SSTORE = 6 gas
    let push_cost: u64 = 6;
    // State gas for new slot = STATE_BYTES_PER_STORAGE_SET * cpsb(1_000_000) = 32 * 1 = 32
    let sstore_state = STATE_BYTES_PER_STORAGE_SET * cost_per_state_byte(GAS_LIMIT * 2);
    // gas_limit: allow intrinsic + PUSH1+PUSH1 + SSTORE-regular + (sstore_state - 6) gas
    // = 21_000 + push_cost + sstore_regular + sstore_state - 6 = 26_031
    // This leaves (sstore_state - 6) gas after SSTORE regular, which is < sstore_state → OOG.
    let gas_limit = 21_000 + push_cost + sstore_regular + sstore_state - 6;

    // Sanity: reservoir must be zero for the spill to matter
    let intrinsic_regular: u64 = 21_000;
    let execution_gas = gas_limit.saturating_sub(intrinsic_regular + sstore_state);
    let regular_gas_budget = TX_MAX_GAS_LIMIT_AMSTERDAM.saturating_sub(intrinsic_regular);
    let reservoir = execution_gas.saturating_sub(regular_gas_budget.min(execution_gas));
    assert_eq!(
        reservoir, 0,
        "reservoir must be 0 for this test to be valid"
    );

    let report = TestRunner::call(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code))
        .with_gas_limit(gas_limit)
        .run();

    assert!(
        !report.is_success(),
        "tx must OOG with gas_limit={gas_limit}: {:?}",
        report.result
    );
    assert_eq!(
        report.state_gas_used, 0,
        "OOG must zero execution state gas (state_gas_used must be 0 after top-level OOG)"
    );
}

// ==================== Test 2: Top-level failure zeros block state gas ====================

/// Block-level state gas (report.state_gas_used) must be zero for a top-level failure
/// that consumed execution state gas but no intrinsic state gas (plain CALL tx).
#[test]
fn test_top_level_revert_zeros_block_state_gas() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);

    // SSTORE(slot 0 = 5) then REVERT — same as test 1a but focusing on block gas_used
    let mut code = sstore_byte(0, 5);
    code.extend(revert_bytecode());

    let report = TestRunner::call(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code))
        .run();

    assert!(!report.is_success(), "should have reverted");
    // Block accounting: state dimension = 0 for a plain CALL tx that reverted
    assert_eq!(
        report.state_gas_used, 0,
        "block state_gas_used should be 0 for a failed plain CALL"
    );
}

#[test]
fn test_top_level_halt_zeros_block_state_gas() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);

    let mut code = sstore_byte(0, 5);
    code.extend(invalid_bytecode());

    let report = TestRunner::call(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code))
        .run();

    assert!(!report.is_success(), "should have halted");
    assert_eq!(
        report.state_gas_used, 0,
        "block state_gas_used should be 0 for a failed plain CALL"
    );
}

#[test]
fn test_top_level_oog_zeros_block_state_gas() {
    // Same calibration as test_top_level_oog_refunds_execution_state_gas (test 1c):
    // plain CALL that SSTOREs and OOGs on the state-gas spill. Asserts the
    // block-accounting invariant (state_gas_used == 0) per PR #2689.
    let addr_a = Address::from_low_u64_be(CONTRACT_A);

    let code = sstore_byte(0, 5);

    let sstore_regular = SSTORE_STORAGE_MODIFICATION + SSTORE_COLD_DYNAMIC;
    let push_cost: u64 = 6;
    let sstore_state = STATE_BYTES_PER_STORAGE_SET * cost_per_state_byte(GAS_LIMIT * 2);
    let gas_limit = 21_000 + push_cost + sstore_regular + sstore_state - 6;

    let report = TestRunner::call(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code))
        .with_gas_limit(gas_limit)
        .run();

    assert!(
        !report.is_success(),
        "tx must OOG with gas_limit={gas_limit}: {:?}",
        report.result
    );
    assert_eq!(
        report.state_gas_used, 0,
        "block state_gas_used should be 0 for a failed plain CALL that OOG'd"
    );
}

// ==================== Test 3: Creation tx failure preserves intrinsic state gas ====================

/// A CREATE tx whose initcode halts. The top-level failure refund zeroes only execution
/// state gas. The intrinsic new-account state gas STAYS in block accounting.
#[test]
fn test_creation_tx_failure_preserves_intrinsic_state_gas() {
    use ethrex_levm::gas_cost::STATE_BYTES_PER_NEW_ACCOUNT;

    // Initcode: just INVALID (exceptional halt)
    let initcode = invalid_bytecode();

    let report = TestRunner::create(initcode)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .run();

    assert!(
        !report.is_success(),
        "CREATE should fail with INVALID: {:?}",
        report.result
    );

    // Intrinsic state gas for CREATE = state_gas_new_account = STATE_BYTES_PER_NEW_ACCOUNT * cpsb
    let cpsb = cost_per_state_byte(GAS_LIMIT * 2);
    let intrinsic_state_gas = STATE_BYTES_PER_NEW_ACCOUNT * cpsb;

    // state_gas_used should equal only the intrinsic portion (no refund via intrinsic_state_gas_refund).
    assert_eq!(
        report.state_gas_used, intrinsic_state_gas,
        "state_gas_used should equal intrinsic_state_gas_charged (new-account) after CREATE failure"
    );
}

// ==================== Test 4: Subcall failure does not zero top-level state gas ====================

/// Parent calls a child that reverts, then runs its own SSTORE. Top-level tx succeeds.
/// The top-level failure refund MUST NOT apply (scope is top-level only).
/// Parent's SSTORE state gas surfaces in state_gas_used.
#[test]
fn test_subcall_failure_does_not_zero_top_level_state_gas() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);
    let addr_b = Address::from_low_u64_be(CONTRACT_B);

    // Contract B: REVERTs
    let code_b = revert_bytecode();

    // Contract A: CALLs B (which reverts), then SSTOREs slot 0 = 5, then stops.
    let mut code_a = call_bytecode(addr_b);
    code_a.extend(sstore_byte(0, 5));
    code_a.extend(stop());

    let report = TestRunner::call(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code_a))
        .with_account(addr_b, contract(code_b))
        .run();

    assert!(
        report.is_success(),
        "top-level tx should succeed: {:?}",
        report.result
    );

    let expected_state_gas = state_gas_storage_set();
    assert_eq!(
        report.state_gas_used, expected_state_gas,
        "state_gas_used should equal one SSTORE charge (subcall failure must not wipe top-level state gas)"
    );
}

// ==================== Test 5: Top-level failure refunds reservoir-drawn state gas ====================

/// Distinct from Test 1a: here the gas_limit is large enough that a nonzero reservoir is built,
/// so the SSTORE state gas is drawn from the reservoir rather than spilling into gas_remaining.
/// The top-level failure must still zero state_gas_used — both reservoir-drawn and spilled
/// portions must be refunded.
///
/// Reservoir formula (Amsterdam):
///   execution_gas = gas_limit - intrinsic_total
///   regular_gas_budget = TX_MAX_GAS_LIMIT_AMSTERDAM - intrinsic_regular
///   gas_left = min(regular_gas_budget, execution_gas)
///   reservoir = execution_gas - gas_left
///
/// With tx_gas_limit = 20_000_000 (> TX_MAX_GAS_LIMIT_AMSTERDAM = 16_777_216):
///   intrinsic_regular = 21_000; intrinsic_state = 0 (plain CALL)
///   execution_gas = 20_000_000 - 21_000 = 19_979_000
///   regular_gas_budget = 16_777_216 - 21_000 = 16_756_216
///   gas_left = 16_756_216
///   reservoir = 19_979_000 - 16_756_216 = 3_222_784  (> sstore_state_gas for any cpsb)
///
/// block_gas_limit = 40_000_000 (≥ tx_gas_limit) to satisfy the tx < block limit validation.
/// Dynamic formula: cpsb(40_000_000) = 150 → sstore_state = 32 * 150 = 4_800.
/// bal-devnet-4 CPSB pin: cpsb = 1174 → sstore_state = 32 * 1174 = 37_568.
/// Both << reservoir (3.2M), so the test holds under either regime. ✓
///
/// The SSTORE state gas is fully drawn from the reservoir — no spill. On REVERT,
/// the execution portion (including the reservoir-drawn amount) must be wiped to zero.
#[test]
fn test_top_level_failure_refunds_reservoir_drawn_state_gas() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);

    // SSTORE(slot 0 = 5) then REVERT — same opcode sequence as test 1a,
    // but gas_limit is large enough to build a nonzero reservoir.
    let mut code = sstore_byte(0, 5);
    code.extend(revert_bytecode());

    // tx_gas_limit large enough that execution_gas > regular_gas_budget → reservoir > 0
    let large_gas_limit: u64 = 20_000_000;
    // block_gas_limit must be >= tx_gas_limit (protocol validation)
    let large_block_gas_limit: u64 = 40_000_000;

    // Verify reservoir is nonzero and covers the SSTORE state gas
    let intrinsic_regular: u64 = 21_000;
    let execution_gas = large_gas_limit.saturating_sub(intrinsic_regular);
    let regular_gas_budget = TX_MAX_GAS_LIMIT_AMSTERDAM.saturating_sub(intrinsic_regular);
    let gas_left = regular_gas_budget.min(execution_gas);
    let reservoir = execution_gas.saturating_sub(gas_left);
    let sstore_state = STATE_BYTES_PER_STORAGE_SET * cost_per_state_byte(large_block_gas_limit);
    assert!(
        reservoir >= sstore_state,
        "reservoir ({reservoir}) must be >= sstore_state ({sstore_state}) for this test"
    );

    let report = TestRunner::call(addr_a)
        .with_account(
            Address::from_low_u64_be(SENDER),
            eoa(U256::from(1_000_000_000)),
        )
        .with_account(addr_a, contract(code))
        .with_gas_limit(large_gas_limit)
        .with_block_gas_limit(large_block_gas_limit)
        .run();

    assert!(!report.is_success(), "should have reverted");
    // Reservoir-drawn state gas must also be wiped on top-level failure.
    assert_eq!(
        report.state_gas_used, 0,
        "state_gas_used must be 0 after top-level failure (reservoir-drawn state gas also refunded)"
    );
}

// ==================== Test 6: Top-level failure refunds state gas propagated from child ====================

/// A successful subcall runs SSTORE and returns to the parent; then the parent reverts.
/// The top-level failure refund must catch state gas propagated up via child success.
#[test]
fn test_top_level_failure_refunds_state_gas_propagated_from_child() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);
    let addr_b = Address::from_low_u64_be(CONTRACT_B);

    // Contract B: SSTOREs (charges state gas), then RETURNs successfully.
    let mut code_b = sstore_byte(0, 5);
    code_b.extend(return_bytecode());

    // Contract A: CALLs B (which succeeds, propagating state gas up), then REVERTs.
    let mut code_a = call_bytecode(addr_b);
    code_a.extend(revert_bytecode());

    let report = TestRunner::call(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code_a))
        .with_account(addr_b, contract(code_b))
        .run();

    assert!(
        !report.is_success(),
        "top-level tx should revert: {:?}",
        report.result
    );
    // The state gas from B's SSTORE propagated to A's frame on B's success.
    // Then A reverted at the top level, so the full execution portion is wiped.
    assert_eq!(
        report.state_gas_used, 0,
        "state_gas_used should be 0: top-level failure must refund state gas propagated from child"
    );
}

// ==================== Test: top-level failure after a credit-absorbed refund ====================

/// Regression: a tx that absorbs a state-gas refund (e.g. 0→N→0 SSTORE) and then halts
/// top-level must NOT double-refund. The credit already bumped the reservoir and the
/// absorbed counter; top-level-reset logic must only refund the remaining un-credited
/// execution portion.
#[test]
fn test_top_level_failure_after_credit_does_not_double_refund() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);

    // slot[0] = 5 (charges state gas), then slot[0] = 0 (0→N→0 credit), then INVALID.
    let mut code = sstore_byte(0, 5);
    code.extend(sstore_byte(0, 0));
    code.extend(invalid_bytecode());

    let report = TestRunner::call(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code))
        .run();

    assert!(!report.is_success(), "tx should halt on INVALID");
    // Net state gas = gross (S) - credited (S) = 0. Top-level reset must not refund
    // S a second time. state_gas_used in report is the net value after all refunds.
    assert_eq!(
        report.state_gas_used, 0,
        "state_gas_used must be 0 (no double-refund)"
    );
}

// ==================== Test: divergence from EELS on partially-credited spill at top halt ====================

/// Reproduces the geth↔ethrex bal-devnet-6 block-level `gas_used` divergence.
///
/// Scenario (plain CALL tx; intrinsic_state = 0, reservoir = 0):
///   1. Contract A SSTOREs slot 0 → spills `SSTORE_STATE` units of state-gas to
///      `gas_remaining`. After the charge: `state_gas_spill = SSTORE_STATE`,
///      `state_gas_spill_outstanding = SSTORE_STATE`.
///   2. Contract A executes a CREATE opcode with a 1-byte INVALID initcode. The
///      CREATE op charges `STATE_NEW` (= STATE_BYTES_PER_NEW_ACCOUNT * cpsb) of
///      state-gas — also fully spilled. After: spill = SSTORE_STATE + STATE_NEW,
///      spill_outstanding = SSTORE_STATE + STATE_NEW.
///   3. The child frame halts immediately on the INVALID opcode (no further
///      state-gas activity). Returning to the parent in `handle_return_create`
///      runs the halt branch (snapshot restore, no local_excess) followed by
///      `credit_state_gas_refund(STATE_NEW)` — applied entirely to spill since
///      spill_outstanding is well above STATE_NEW. After the credit:
///      spill_outstanding = SSTORE_STATE, reservoir = STATE_NEW.
///   4. Contract A then executes INVALID itself → top-level halt.
///
/// On the top-level halt, ethrex's non-CREATE-tx reclassify formula is
///   `max(state_gas_spill_outstanding, reservoir_surplus)`
///       = max(SSTORE_STATE, STATE_NEW)
///       = STATE_NEW              (STATE_NEW > SSTORE_STATE)
///
/// The reference EELS rule re-classifies the *total gross spill* on halt
/// (`total_state - reservoir` = `state_gas_used + state_gas_left - reservoir`,
/// which after the credit cancellation simplifies to total spill `S`):
///   `reclassify_eels = SSTORE_STATE + STATE_NEW`
///
/// The block-dimension `regular_gas` is then computed in `refund_sender` as
///   `raw_consumed - intrinsic_state - reservoir_initial - state_gas_spill + regular_gas_reclassified`
/// which expands (using raw = gas_limit on halt, both intrinsic_state and
/// reservoir_initial = 0) to:
///   ethrex: gas_limit - (SSTORE_STATE + STATE_NEW) + STATE_NEW       = gas_limit - SSTORE_STATE
///   EELS  : gas_limit - (SSTORE_STATE + STATE_NEW) + (SSTORE_STATE+STATE_NEW) = gas_limit
///
/// Hence `report.gas_used` should equal `gas_limit` per EELS but currently
/// equals `gas_limit - SSTORE_STATE` in ethrex. The asserted difference
/// (== `state_gas_storage_set()`) is exactly the amount of outstanding spill
/// that the credit did NOT cancel — the term ethrex's `max(.,.)` formula drops.
#[test]
fn test_top_halt_after_partial_credit_to_spill_diverges_from_eels() {
    use ethrex_levm::gas_cost::STATE_BYTES_PER_NEW_ACCOUNT;

    let addr_a = Address::from_low_u64_be(CONTRACT_A);

    // Parent contract A:
    //   SSTORE(slot 0, 5)   — charges SSTORE_STATE state-gas (spills, since reservoir = 0)
    //   CREATE(0, 0, 1) where memory[0] = 0xfe — child halts on INVALID
    //   INVALID             — top-level halt
    let mut code = sstore_byte(0, 5);
    code.extend(create_failing_bytecode(0xfe));
    code.extend(invalid_bytecode());

    let report = TestRunner::call(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code))
        .run();

    assert!(
        !report.is_success(),
        "tx should halt on INVALID: {:?}",
        report.result
    );

    // Plain CALL tx: intrinsic_state_gas = 0 → state dimension wipes to 0 on top-level failure.
    assert_eq!(
        report.state_gas_used, 0,
        "block state_gas_used should be 0 for a top-level halted plain CALL tx"
    );

    // Block-level gas_used per EELS reference: equals the entire tx gas_limit, because
    // every byte of charged state-gas (including the credit-refunded portion) and every
    // byte of regular gas was burned by the halt.
    let cpsb = cost_per_state_byte(GAS_LIMIT * 2);
    let _state_new = STATE_BYTES_PER_NEW_ACCOUNT * cpsb;
    let sstore_state = STATE_BYTES_PER_STORAGE_SET * cpsb;
    let expected_gas_used_eels = GAS_LIMIT;
    let expected_gas_used_ethrex_buggy = GAS_LIMIT - sstore_state;

    // Sanity: the formulas only diverge when the credit-applied-to-spill portion
    // (STATE_NEW) is strictly less than the total outstanding spill at halt
    // (SSTORE_STATE + STATE_NEW). That is, SSTORE_STATE > 0 — a trivial check.
    assert!(
        sstore_state > 0,
        "test scenario requires nonzero SSTORE state-gas to leave residual spill after credit"
    );

    // Currently fails on bal-devnet-6: ethrex reports `gas_limit - sstore_state`
    // instead of `gas_limit`. The `min(report.gas_used, _)` line below is the
    // diagnostic showing both candidate values for easier triage.
    assert_eq!(
        report.gas_used, expected_gas_used_eels,
        "block gas_used divergence: ethrex={} expected_eels={} diff={} (== one SSTORE state-gas charge); \
         ethrex's `max(spill_outstanding, reservoir_surplus)` halt formula drops the \
         residual outstanding spill that wasn't cancelled by the CREATE-failure refund",
        report.gas_used,
        expected_gas_used_eels,
        expected_gas_used_eels.saturating_sub(report.gas_used),
    );

    // (Held back from causing a second failure but documented.)
    let _ = expected_gas_used_ethrex_buggy;
}

// ==================== Test: phantom drain credit must not cancel real spill ====================

/// Regression for the bal-devnet-6 block-21 fork between ethrex and geth on a
/// CREATE TX whose initcode performs two failing inner CREATEs.
///
/// Scenario (CREATE TX; intrinsic_state = STATE_NEW; reservoir_initial = 0):
///   1. First inner CREATE charges `STATE_NEW` of state-gas. With reservoir = 0,
///      it spills the full amount to `gas_remaining`.
///        After: state_gas_spill = STATE_NEW, state_gas_spill_outstanding = STATE_NEW.
///   2. Inner-1 child halts on INVALID. `handle_return_create`'s halt branch
///      runs (no local_excess) and then `credit_state_gas_refund(STATE_NEW)`:
///        applied_to_spill = STATE_NEW, applied_to_drain = 0.
///        After: spill_outstanding = 0, reservoir = STATE_NEW.
///   3. Second inner CREATE charges `STATE_NEW` of state-gas. With reservoir =
///      STATE_NEW, the charge is absorbed entirely from the reservoir — NO
///      spill. state_gas_spill stays at STATE_NEW.
///   4. Inner-2 child halts on INVALID. `credit_state_gas_refund(STATE_NEW)`:
///        frame_outstanding_delta = 0, applied_to_spill = 0,
///        applied_to_drain = STATE_NEW (the credit can't cancel spill that
///        doesn't exist in this frame).
///        After: state_gas_credit_against_drain = STATE_NEW.
///   5. Outer initcode hits INVALID → top-level halt.
///
/// At top-halt, the gross spill (STATE_NEW) was real — it was permanently
/// drawn from `gas_remaining` in step 1 and the user paid for it. Per EELS
/// `total_state - reservoir`, it must surface in the regular dimension.
///
/// The pre-fix formula
///   `state_gas_spill - state_gas_credit_against_drain - regular_gas_reclassified`
/// evaluates to `STATE_NEW - STATE_NEW - 0 = 0` because the phantom drain
/// credit (step 4, against a charge that itself didn't spill) cancels the
/// real spill. Capping `credit_against_drain` by `regular_gas_reclassified`
/// (the only legitimate-drain ledger — populated by deeper-frame halt
/// reclassifications) gives 0 here, so the gross spill flows through to
/// `regular_gas_reclassified` as required.
///
/// Block-level expected (EELS): `tx_regular = gas_limit - intrinsic_state`,
/// `tx_state = intrinsic_state`, so `report.gas_used = gas_limit`.
/// Pre-fix ethrex: `tx_regular = gas_limit - 2*STATE_NEW`, hence
/// `report.gas_used = gas_limit - STATE_NEW` (off by one NEW_ACCOUNT charge).
#[test]
fn test_top_halt_phantom_drain_does_not_cancel_real_spill() {
    use ethrex_levm::gas_cost::STATE_BYTES_PER_NEW_ACCOUNT;

    // Outer initcode for the CREATE TX:
    //   CREATE(0,0,1) where memory[0]=0xfe   — 1st inner CREATE, child halts
    //   CREATE(0,0,1) where memory[0]=0xfe   — 2nd inner CREATE, child halts
    //   INVALID                              — top-level halt
    let mut initcode = create_failing_bytecode(0xfe);
    initcode.extend(create_failing_bytecode(0xfe));
    initcode.extend(invalid_bytecode());

    let report = TestRunner::create(initcode)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .run();

    assert!(
        !report.is_success(),
        "CREATE tx should halt on INVALID: {:?}",
        report.result
    );

    // CREATE tx: intrinsic_state_gas = STATE_NEW; survives top-level wipe.
    let cpsb = cost_per_state_byte(GAS_LIMIT * 2);
    let state_new = STATE_BYTES_PER_NEW_ACCOUNT * cpsb;
    assert_eq!(
        report.state_gas_used, state_new,
        "block state_gas_used should equal intrinsic_state (one NEW_ACCOUNT) for a halted CREATE tx"
    );

    // Block-level gas_used per EELS: every byte of regular gas was burned by
    // the halt and the gross-spill from step 1 is reclassified to regular.
    // tx_regular = gas_limit - intrinsic_state; tx_state = intrinsic_state
    // ⇒ report.gas_used = gas_limit.
    let expected_gas_used_eels = GAS_LIMIT;
    let expected_gas_used_ethrex_buggy = GAS_LIMIT - state_new;

    assert_eq!(
        report.gas_used, expected_gas_used_eels,
        "block gas_used divergence: ethrex={} expected_eels={} diff={} (== one NEW_ACCOUNT state-gas charge); \
         the phantom drain credit from refunding the reservoir-funded second inner CREATE \
         must not cancel the real spill from the first inner CREATE",
        report.gas_used,
        expected_gas_used_eels,
        expected_gas_used_eels.saturating_sub(report.gas_used),
    );

    let _ = expected_gas_used_ethrex_buggy;
}
