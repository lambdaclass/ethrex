//! EIP-8037 code-deposit state-gas discard tests (execution-specs PR #2595).
//!
//! Verifies that when a CREATE's code-deposit halts (oversized-code or deposit-OOG),
//! the state gas consumed during initcode execution is discarded from the block state
//! gas accumulator. Two source scenarios × two halt types = 4 tests.

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
    constants::AMSTERDAM_MAX_CODE_SIZE,
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    errors::{DatabaseError, ExecutionReport},
    gas_cost::{
        CODE_DEPOSIT_REGULAR_COST_PER_WORD, REGULAR_GAS_CREATE, STATE_BYTES_PER_NEW_ACCOUNT,
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
const CONTRACT_FACTORY: u64 = 0x2000;

// block_gas_limit = 1_000_000 → cost_per_state_byte(1_000_000) = 1
// state_gas_new_account = STATE_BYTES_PER_NEW_ACCOUNT * 1 = 112
const BLOCK_GAS_LIMIT: u64 = 1_000_000;

// TX base and CREATE constants
const TX_BASE: u64 = 21_000;
// Non-zero calldata byte cost (EIP-2028)
const CALLDATA_NONZERO: u64 = 16;
// Zero calldata byte cost
const CALLDATA_ZERO: u64 = 4;

// Code size for deposit-OOG test (64 bytes):
// - keccak regular = ceil(64/32)*6 = 12 gas
// - deposit state = 64 * 1 = 64 gas (spill when reservoir is 0)
// We need gas_remaining after keccak >= 0 but gas_remaining < deposit_state
const DEPOSIT_OOG_CODE_SIZE: u64 = 64;

// ==================== Bytecode helpers ====================

/// Initcode that returns AMSTERDAM_MAX_CODE_SIZE + 1 bytes (oversized).
/// Uses uninitialized memory (all zeros); no MSTORE needed.
/// Bytecode: PUSH3(size_hi, size_mid, size_lo), PUSH1(0), RETURN
fn oversized_initcode() -> Vec<u8> {
    let size = AMSTERDAM_MAX_CODE_SIZE + 1; // 32769 = 0x8001
    vec![
        0x62, // PUSH3
        ((size >> 16) & 0xff) as u8,
        ((size >> 8) & 0xff) as u8,
        (size & 0xff) as u8,
        0x60,
        0x00, // PUSH1 0 (offset)
        0xf3, // RETURN
    ]
}

/// Initcode that returns DEPOSIT_OOG_CODE_SIZE bytes (small valid code).
/// Uses uninitialized memory (all zeros).
fn deposit_oog_initcode() -> Vec<u8> {
    let size = DEPOSIT_OOG_CODE_SIZE as u8;
    vec![
        0x60, size, // PUSH1 size
        0x60, 0x00, // PUSH1 0 (offset)
        0xf3, // RETURN
    ]
}

/// Returns a factory contract that runs CREATE with the given initcode, then STOPs.
/// Memory layout: store initcode byte-by-byte, then CREATE.
fn factory_with_inner_create(initcode: &[u8]) -> Vec<u8> {
    let mut bytecode: Vec<u8> = Vec::new();

    // Store initcode in memory (byte by byte)
    for (i, byte) in initcode.iter().enumerate() {
        bytecode.extend_from_slice(&[0x60, *byte, 0x60, i as u8, 0x53]); // PUSH1 byte, PUSH1 i, MSTORE8
    }

    // CREATE: PUSH1 len, PUSH1 0 (offset), PUSH1 0 (value)
    bytecode.push(0x60);
    bytecode.push(initcode.len() as u8); // size
    bytecode.push(0x60);
    bytecode.push(0x00); // offset
    bytecode.push(0x60);
    bytecode.push(0x00); // value
    bytecode.push(0xf0); // CREATE — leaves address (or 0) on stack
    bytecode.push(0x50); // POP
    bytecode.push(0x00); // STOP

    bytecode
}

/// Returns a factory contract that runs CREATE with the given initcode, then STOPs WITHOUT
/// popping the CREATE result. The CREATE result (0 = failed, addr = success) remains on the
/// stack when STOP executes — STOP terminates successfully regardless.
///
/// This variant is used for tight gas calibration tests where there may not be enough gas
/// for a POP after the CREATE (the 63/64 rule leaves ceil(R/64) gas for the parent after
/// the inner frame, which for small R can be 0 or 1 — not enough for POP(2 gas)).
fn factory_with_inner_create_tight(initcode: &[u8]) -> Vec<u8> {
    let mut bytecode: Vec<u8> = Vec::new();

    // Store initcode in memory (byte by byte)
    for (i, byte) in initcode.iter().enumerate() {
        bytecode.extend_from_slice(&[0x60, *byte, 0x60, i as u8, 0x53]); // PUSH1 byte, PUSH1 i, MSTORE8
    }

    // CREATE: PUSH1 len, PUSH1 0 (offset), PUSH1 0 (value)
    bytecode.push(0x60);
    bytecode.push(initcode.len() as u8); // size
    bytecode.push(0x60);
    bytecode.push(0x00); // offset
    bytecode.push(0x60);
    bytecode.push(0x00); // value
    bytecode.push(0xf0); // CREATE — leaves result on stack (0=failed, addr=success)
    bytecode.push(0x00); // STOP (no POP; STOP exits successfully regardless of stack contents)

    bytecode
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

struct Runner {
    accounts: Vec<(Address, Account)>,
    gas_limit: u64,
    is_create: bool,
    initcode: Bytes,
    call_target: Option<Address>,
}

impl Runner {
    fn top_level_create(gas_limit: u64, initcode: Vec<u8>) -> Self {
        Self {
            accounts: Vec::new(),
            gas_limit,
            is_create: true,
            initcode: Bytes::from(initcode),
            call_target: None,
        }
    }

    fn call_to_factory(gas_limit: u64, factory_addr: Address) -> Self {
        Self {
            accounts: Vec::new(),
            gas_limit,
            is_create: false,
            initcode: Bytes::new(),
            call_target: Some(factory_addr),
        }
    }

    fn with_account(mut self, addr: Address, acc: Account) -> Self {
        self.accounts.push((addr, acc));
        self
    }

    fn run(self) -> ExecutionReport {
        let test_db = TestDatabase::new();
        let accounts_map: FxHashMap<Address, Account> = self.accounts.into_iter().collect();
        let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(test_db), accounts_map);

        let fork = Fork::Amsterdam;
        let blob_schedule = EVMConfig::canonical_values(fork);
        let env = Environment {
            origin: Address::from_low_u64_be(SENDER),
            gas_limit: self.gas_limit,
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
            block_gas_limit: BLOCK_GAS_LIMIT,
            is_privileged: false,
            fee_token: None,
            disable_balance_check: true,
            is_system_call: false,
        };

        let tx = if self.is_create {
            Transaction::EIP1559Transaction(EIP1559Transaction {
                to: TxKind::Create,
                value: U256::zero(),
                data: self.initcode,
                gas_limit: self.gas_limit,
                max_fee_per_gas: 0,
                max_priority_fee_per_gas: 0,
                ..Default::default()
            })
        } else {
            let target = self.call_target.unwrap_or_default();
            Transaction::EIP1559Transaction(EIP1559Transaction {
                to: TxKind::Call(target),
                value: U256::zero(),
                data: Bytes::new(),
                gas_limit: self.gas_limit,
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

// ==================== Helpers ====================

/// Returns the intrinsic state gas for a top-level CREATE under our test settings.
/// = STATE_BYTES_PER_NEW_ACCOUNT * cost_per_state_byte(BLOCK_GAS_LIMIT)
fn create_intrinsic_state_gas() -> u64 {
    let cpsb = cost_per_state_byte(BLOCK_GAS_LIMIT);
    STATE_BYTES_PER_NEW_ACCOUNT * cpsb
}

/// Returns the code-deposit state gas for N bytes.
fn deposit_state_gas(code_len: u64) -> u64 {
    let cpsb = cost_per_state_byte(BLOCK_GAS_LIMIT);
    code_len * cpsb
}

/// Returns the code-deposit regular gas (keccak cost) for N bytes.
fn deposit_regular_gas(code_len: u64) -> u64 {
    code_len.div_ceil(32) * CODE_DEPOSIT_REGULAR_COST_PER_WORD
}

// ==================== Tests ====================

// ---- Test 1: Top-level CREATE, oversized-code halt ----

/// Scenario: outer CALL = top-level CREATE, initcode returns oversized bytes.
/// The size check happens BEFORE any gas charges. No code-deposit state gas is charged.
/// Phase 5a (top-level failure) zeroes execution state gas, leaving only intrinsic.
/// Assert: state_gas_used == intrinsic_state_gas (new-account charge stays).
#[test]
fn test_top_level_create_oversized_code_discard() {
    let initcode = oversized_initcode();

    let report = Runner::top_level_create(500_000, initcode)
        .with_account(
            Address::from_low_u64_be(SENDER),
            eoa(U256::from(10_000_000)),
        )
        .run();

    // The CREATE fails due to oversized code.
    assert!(
        !report.is_success(),
        "CREATE should fail with oversized code: {:?}",
        report.result
    );

    // The code-deposit state gas would be (AMSTERDAM_MAX_CODE_SIZE + 1) * cpsb
    // if it were charged. It must NOT appear in state_gas_used.
    let intrinsic_state = create_intrinsic_state_gas();
    let would_be_deposit_state = deposit_state_gas(AMSTERDAM_MAX_CODE_SIZE + 1);

    assert!(
        would_be_deposit_state > 0,
        "sanity: deposit state gas should be positive"
    );

    // state_gas_used must equal intrinsic only (execution wiped by Phase 5a on failure).
    // Intrinsic state gas = state_gas_new_account (for the CREATE tx).
    assert_eq!(
        report.state_gas_used, intrinsic_state,
        "state_gas_used should equal intrinsic state gas only (code-deposit state gas discarded)"
    );
}

// ---- Test 2: Inner CREATE, oversized-code halt ----

/// Scenario: outer tx calls a factory contract, factory does CREATE that returns oversized code.
/// The inner CREATE fails, state_gas_used is restored to snapshot (which includes new-account
/// charge from CREATE setup). The code-deposit state gas is NOT charged (size check pre-gas).
/// Assert: state_gas_used == state_gas_new_account for the inner CREATE's account.
#[test]
fn test_inner_create_oversized_code_discard() {
    let factory_addr = Address::from_low_u64_be(CONTRACT_FACTORY);
    let initcode = oversized_initcode();
    let factory_code = factory_with_inner_create(&initcode);

    let report = Runner::call_to_factory(500_000, factory_addr)
        .with_account(
            Address::from_low_u64_be(SENDER),
            eoa(U256::from(10_000_000)),
        )
        .with_account(factory_addr, contract(factory_code))
        .run();

    // The outer tx CALL succeeds (factory continues after the CREATE fails).
    assert!(
        report.is_success(),
        "outer transaction should succeed: {:?}",
        report.result
    );

    // The inner CREATE failed (oversized code). The code-deposit state gas would be huge:
    // (AMSTERDAM_MAX_CODE_SIZE + 1) * cpsb. It must NOT appear in state_gas_used.
    let would_be_deposit_state = deposit_state_gas(AMSTERDAM_MAX_CODE_SIZE + 1);
    assert!(
        would_be_deposit_state > 0,
        "sanity: deposit state gas should be positive"
    );

    // Per EELS `credit_state_gas_refund(evm, create_account_state_gas)` on child error:
    // no account was created, so the CREATE new-account charge is refunded. Net
    // state_gas_used for the tx must be 0.
    assert_eq!(
        report.state_gas_used, 0,
        "state_gas_used should be 0: no account created, CREATE charge refunded"
    );
}

// ---- Test 3: Top-level CREATE, deposit-OOG halt ----

/// Scenario: top-level CREATE with initcode returning DEPOSIT_OOG_CODE_SIZE bytes,
/// gas_limit tuned so that keccak gas succeeds but deposit state gas OOGs.
///
/// Calibration (block_gas_limit = 1_000_000, cpsb = 1):
///   deposit_oog_initcode() = [0x60, 0x40, 0x60, 0x00, 0xf3] (5 bytes, all non-zero except 0x00)
///   Calldata gas: 0x60(16) + 0x40(16) + 0x60(16) + 0x00(4) + 0xf3(16) = 68
///   Initcode word gas (EIP-3860): ceil(5/32)*2 = 2
///   intrinsic_regular = TX_BASE(21_000) + REGULAR_GAS_CREATE(9_000) + 68 + 2 = 30_070
///   intrinsic_state = STATE_BYTES_PER_NEW_ACCOUNT(112) * cpsb(1) = 112
///   total_intrinsic = 30_182
///
///   Initcode execution: PUSH1(3) + PUSH1(3) + RETURN(memory_expansion_cost 0→64 = 6) = 12 gas
///   keccak_regular = ceil(64/32) * 6 = 12 gas
///   deposit_state = 64 * 1 = 64 gas (spills to gas_remaining since reservoir = 0)
///
///   reservoir formula: execution_gas = gas_limit - total_intrinsic = execution_margin
///     regular_gas_budget = TX_MAX_GAS_LIMIT_AMSTERDAM - intrinsic_regular = 16_747_146
///     reservoir = execution_gas - min(regular_gas_budget, execution_gas) = 0 (for small margins)
///
///   With execution_margin = 50:
///     gas_remaining after initcode  = 50 - 12 = 38
///     gas_remaining after keccak    = 38 - 12 = 26
///     deposit_state spill = 64 > 26 → OOG (deterministic)
///
///   After top-level failure: Phase 5a zeroes execution state gas → state_gas_used = intrinsic_state.
#[test]
fn test_top_level_create_deposit_oog_discard() {
    let cpsb = cost_per_state_byte(BLOCK_GAS_LIMIT);
    let initcode = deposit_oog_initcode();

    // Compute the precise calldata gas for our initcode
    let calldata_gas: u64 = initcode
        .iter()
        .map(|b| {
            if *b != 0 {
                CALLDATA_NONZERO
            } else {
                CALLDATA_ZERO
            }
        })
        .sum();

    // EIP-3860 initcode word cost: 2 * ceil(len / 32)
    let initcode_word_cost = 2 * initcode.len().div_ceil(32) as u64;

    let intrinsic_regular = TX_BASE + REGULAR_GAS_CREATE + calldata_gas + initcode_word_cost;
    let intrinsic_state = STATE_BYTES_PER_NEW_ACCOUNT * cpsb;
    let total_intrinsic = intrinsic_regular + intrinsic_state;

    let keccak_cost = deposit_regular_gas(DEPOSIT_OOG_CODE_SIZE);
    let deposit_state = deposit_state_gas(DEPOSIT_OOG_CODE_SIZE);

    // initcode execution gas: PUSH1(3) + PUSH1(3) + RETURN(mem_exp 0→64 = 6) = 12
    let initcode_exec_gas: u64 = 12;

    // execution_margin = 50 → gas_after_keccak = 50 - 12 - 12 = 26 < 64 → OOG on deposit state
    let execution_margin: u64 = 50;
    let gas_limit = total_intrinsic + execution_margin;

    // Sanity: gas_after_keccak must be < deposit_state to guarantee OOG
    let gas_after_keccak = execution_margin
        .saturating_sub(initcode_exec_gas)
        .saturating_sub(keccak_cost);
    assert!(
        gas_after_keccak < deposit_state,
        "calibration error: gas_after_keccak={gas_after_keccak} must be < deposit_state={deposit_state}"
    );
    // Sanity: gas_after_keccak must be >= 0 (keccak succeeds before OOG)
    assert!(
        execution_margin >= initcode_exec_gas + keccak_cost,
        "calibration error: initcode+keccak must fit in execution_margin"
    );

    let report = Runner::top_level_create(gas_limit, initcode)
        .with_account(
            Address::from_low_u64_be(SENDER),
            eoa(U256::from(10_000_000)),
        )
        .run();

    // With the calibrated gas_limit, deposit-OOG is deterministic.
    assert!(
        !report.is_success(),
        "CREATE must fail with deposit-OOG (gas_limit={gas_limit}): {:?}",
        report.result
    );
    // Phase 5a: top-level failure zeroes execution state gas; only intrinsic_state stays.
    assert_eq!(
        report.state_gas_used, intrinsic_state,
        "state_gas_used must equal intrinsic_state only (code-deposit state gas discarded on deposit-OOG)"
    );
}

// ---- Test 4: Inner CREATE, deposit-OOG halt ----

/// Scenario: factory contract does CREATE with DEPOSIT_OOG_CODE_SIZE bytes. The outer tx
/// gas_limit is calibrated so the inner CREATE frame gets exactly enough gas for initcode
/// execution and keccak, but not for the deposit state gas → deposit-OOG fires deterministically.
///
/// Calibration (block_gas_limit = 1_000_000, cpsb = 1, CALL to factory, no calldata):
///   intrinsic_regular (outer CALL) = TX_BASE = 21_000
///   factory execution before CREATE opcode:
///     5 MSTORE8 sequences:
///       i=0: PUSH1(3)+PUSH1(3)+MSTORE8(3+mem_exp(32,0)=3) = 12 gas
///       i=1..4: PUSH1(3)+PUSH1(3)+MSTORE8(3+0) = 9 gas each → 4×9 = 36 gas
///       total = 12 + 36 = 48 gas
///     3 PUSH1 ops (size=5, offset=0, value=0) = 9 gas
///     factory_before_CREATE = 48 + 9 = 57 gas
///   CREATE opcode regular gas:
///     gas_cost::create(32, 32, 5, Amsterdam):
///       memory_expansion_cost(32, 32) = 0
///       init_code_cost = ceil(5/32)*2 = 2
///       create_base_cost = REGULAR_GAS_CREATE = 9_000
///       total = 9_002
///   increase_state_gas(112) spills to gas_remaining (reservoir = 0 for tight gas_limit)
///   Total overhead = 21_000 + 57 + 9_002 + 112 = 30_171
///
///   R = outer_gas_limit - 30_171 (= gas_remaining at max_message_call_gas point)
///   inner_gas_limit = floor(R × 63 / 64)
///   parent_gas_remaining after CREATE reservation = ceil(R / 64)  [returned to parent on frame exit = 0 since inner OOGs]
///
///   Need inner_gas_limit in [24, 87) for deposit-OOG:
///     inner_gas_limit >= 24 (initcode=12 + keccak=12 succeeds)
///     inner_gas_limit < 88  (deposit_state=64 OOGs: inner_gas_limit - 24 < 64)
///
///   Use R = 49: inner_gas_limit = floor(49×63/64) = 48
///     gas_after_keccak = 48 - 12 - 12 = 24 < 64 → OOG deterministic ✓
///   parent gas after CREATE = ceil(49/64) = 1; STOP costs 0 gas → factory STOP succeeds ✓
///   outer_gas_limit = 30_171 + 49 = 30_220
///
/// Expected: outer CALL succeeds; inner CREATE fails with deposit-OOG; code-deposit state
/// gas (64) is discarded; state_gas_used = new_account_state (112).
#[test]
fn test_inner_create_deposit_oog_discard() {
    let cpsb = cost_per_state_byte(BLOCK_GAS_LIMIT);
    let new_account_state = STATE_BYTES_PER_NEW_ACCOUNT * cpsb;
    let deposit_state = deposit_state_gas(DEPOSIT_OOG_CODE_SIZE);
    let keccak_cost = deposit_regular_gas(DEPOSIT_OOG_CODE_SIZE);
    // initcode execution: PUSH1(3)+PUSH1(3)+RETURN(mem_exp 0→64 = 6) = 12 gas
    let initcode_exec_gas: u64 = 12;

    let factory_addr = Address::from_low_u64_be(CONTRACT_FACTORY);
    let initcode = deposit_oog_initcode();
    // Use the tight variant (no POP after CREATE) so STOP costs 0 and the parent
    // frame can succeed even when only 1 gas remains after CREATE reservation.
    let factory_code = factory_with_inner_create_tight(&initcode);

    // Overhead for outer CALL tx up to the max_message_call_gas point inside generic_create.
    // See calibration comment above for breakdown.
    let outer_overhead: u64 = 30_171;
    // R = 49 → inner_gas_limit = floor(49×63/64) = 48
    let r: u64 = 49;
    let outer_gas_limit = outer_overhead + r;

    // Compute inner gas limit to verify calibration
    let inner_gas_limit = r - r / 64; // floor(r * 63/64) = r - floor(r/64)
    let gas_after_keccak = inner_gas_limit
        .saturating_sub(initcode_exec_gas)
        .saturating_sub(keccak_cost);

    assert!(
        gas_after_keccak < deposit_state,
        "calibration error: gas_after_keccak={gas_after_keccak} must be < deposit_state={deposit_state} for OOG"
    );
    assert!(
        inner_gas_limit >= initcode_exec_gas + keccak_cost,
        "calibration error: inner frame must have enough gas for initcode+keccak"
    );

    let report = Runner::call_to_factory(outer_gas_limit, factory_addr)
        .with_account(
            Address::from_low_u64_be(SENDER),
            eoa(U256::from(10_000_000)),
        )
        .with_account(factory_addr, contract(factory_code))
        .run();

    // Outer CALL must succeed (factory reaches STOP).
    assert!(
        report.is_success(),
        "outer transaction must succeed: {:?}",
        report.result
    );

    // Per EELS `credit_state_gas_refund(evm, create_account_state_gas)` on child error:
    // inner CREATE's deposit-OOG is a child error, so the CREATE new-account charge is
    // refunded and the deposit charge never landed (OOG). Net state_gas_used = 0.
    assert_eq!(
        report.state_gas_used, 0,
        "state_gas_used must be 0: inner CREATE failed (deposit-OOG), account creation refunded; \
         sanity: deposit_state={deposit_state}, new_account_state={new_account_state}",
    );
}
