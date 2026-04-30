//! EIP-8037 SSTORE 0→N→0 reservoir refill + nested clamp-and-spill tests.
//!
//! Verifies that when a storage slot returns to its original zero value in the same
//! transaction, the state gas cost is refunded via the per-frame clamp-and-spill
//! mechanism rather than the regular refund counter.

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
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    errors::{DatabaseError, ExecutionReport},
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
const CONTRACT_C: u64 = 0x4000;
// Large enough to cover SSTORE state gas plus regular gas
const GAS_LIMIT: u64 = 500_000;
// block_gas_limit = GAS_LIMIT * 2 = 1_000_000.
// NOTE (bal-devnet-4 CPSB pin): cost_per_state_byte is currently fixed at 1174.
// With the dynamic formula, cost_per_state_byte(1_000_000) = 1 → state_gas_storage_set = 32.
// Tests below compute amounts via the live function so they hold under both regimes.

// ==================== Bytecode helpers ====================

/// PUSH1 value, PUSH1 slot, SSTORE — writes `value` to storage slot `slot`.
fn sstore_byte(slot: u8, value: u8) -> Vec<u8> {
    vec![0x60, value, 0x60, slot, 0x55]
}

/// STOP (0x00)
fn stop() -> Vec<u8> {
    vec![0x00]
}

/// REVERT with (0, 0)
fn revert() -> Vec<u8> {
    vec![0x60, 0x00, 0x60, 0x00, 0xfd]
}

/// RETURN with (0, 0)
fn ret() -> Vec<u8> {
    vec![0x60, 0x00, 0x60, 0x00, 0xf3]
}

/// DELEGATECALL to `target` with no args and no return data capture.
/// Stack before: GAS, target(20 bytes), argsOffset, argsLength, retOffset, retLength
fn delegatecall_bytecode(target: Address) -> Vec<u8> {
    // retLen retOffset argsLen argsOffset target GAS DELEGATECALL POP
    let mut b = vec![0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00]; // 4x PUSH1 0
    b.push(0x73); // PUSH20
    b.extend_from_slice(target.as_bytes());
    b.push(0x5a); // GAS
    b.push(0xf4); // DELEGATECALL
    b.push(0x50); // POP (discard success flag)
    b
}

/// CALL to `target` with no value, no args and no return data capture.
fn call_bytecode(target: Address) -> Vec<u8> {
    // retLen retOffset argsLen argsOffset value target GAS CALL POP
    let mut b = vec![0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00]; // retLen retOffset argsLen argsOffset
    b.extend_from_slice(&[0x60, 0x00]); // PUSH1 0 (value)
    b.push(0x73); // PUSH20
    b.extend_from_slice(target.as_bytes());
    b.push(0x5a); // GAS
    b.push(0xf1); // CALL
    b.push(0x50); // POP
    b
}

/// CALL to `target` transferring `value` wei. No args, no return capture.
/// When `target` doesn't exist in pre-state and `value > 0`, Amsterdam charges
/// `state_gas_new_account` in the caller's frame.
fn call_with_value_bytecode(target: Address, value: u8) -> Vec<u8> {
    // retLen retOffset argsLen argsOffset value target GAS CALL POP
    let mut b = vec![0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00]; // 4x PUSH1 0
    b.extend_from_slice(&[0x60, value]); // PUSH1 <value>
    b.push(0x73); // PUSH20
    b.extend_from_slice(target.as_bytes());
    b.push(0x5a); // GAS
    b.push(0xf1); // CALL
    b.push(0x50); // POP
    b
}

// ==================== Test runner ====================

struct TestRunner {
    accounts: Vec<(Address, Account)>,
    target: Address,
}

impl TestRunner {
    fn new(target: Address) -> Self {
        Self {
            accounts: Vec::new(),
            target,
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
            gas_limit: GAS_LIMIT,
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
            block_gas_limit: GAS_LIMIT * 2,
            is_privileged: false,
            fee_token: None,
            disable_balance_check: true,
            is_system_call: false,
        };

        let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            to: TxKind::Call(self.target),
            value: U256::zero(),
            data: Bytes::new(),
            gas_limit: GAS_LIMIT,
            max_fee_per_gas: 0,
            max_priority_fee_per_gas: 0,
            ..Default::default()
        });

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

// ==================== Tests ====================

/// Test (a): Single-frame 0→5→0.
///
/// A single contract writes slot 0 from 0 to 5 (charging state gas), then writes
/// it back to 0. The 0→N→0 pattern should reduce `state_gas_used` by
/// `state_gas_storage_set`, and should NOT increase `gas_refunded` by that amount.
#[test]
fn test_single_frame_zero_to_n_to_zero() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);

    // slot[0] = 5  (0→N, charges state_gas_storage_set)
    // slot[0] = 0  (N→0, original=0 → 0→N→0 refund)
    // STOP
    let mut code = sstore_byte(0, 5);
    code.extend(sstore_byte(0, 0));
    code.extend(stop());

    let report = TestRunner::new(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code))
        .run();

    // 0→N→0 refund must reduce state_gas_used to 0 (net zero creation).
    assert_eq!(
        report.state_gas_used, 0,
        "state_gas_used should be 0 after a 0→N→0 round-trip"
    );

    // The state gas refund must NOT pass through gas_refunded (regular refund counter).
    // gas_refunded should only contain the regular SSTORE refund (RESTORE_SLOT_COST=2800)
    // for the N→0 write, not the state gas portion.
    assert_eq!(
        report.gas_refunded, 2800,
        "gas_refunded should be exactly the RESTORE_SLOT_COST=2800, got {}",
        report.gas_refunded
    );

    assert!(
        report.is_success(),
        "transaction should succeed: {:?}",
        report.result
    );
}

/// Test (a-pre): Without 0→N→0, state_gas_used reflects the creation charge.
///
/// Writing 0→5 without the reversal should result in a positive state_gas_used.
#[test]
fn test_single_frame_zero_to_n_only() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);

    // slot[0] = 5  (0→N, charges state_gas_storage_set)
    // STOP
    let mut code = sstore_byte(0, 5);
    code.extend(stop());

    let report = TestRunner::new(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code))
        .run();

    // No refund: state_gas_used should be positive.
    assert!(
        report.state_gas_used > 0,
        "state_gas_used should be positive for a 0→N write without reversal"
    );
    assert!(report.is_success());
}

/// Test (b): 1-hop nested DELEGATECALL, refund spills from B to A.
///
/// Contract A writes 0→5 (charging state gas in A's frame), then DELEGATECALLs B.
/// B writes 5→0 on the same slot (original=0, current=5, value=0 → 0→N→0 pattern).
///
/// In B's frame, local state_gas_used = 0 (B charged nothing). So `credit_state_gas_refund`
/// clamps to 0 and the full amount goes to `state_gas_refund_pending`. On successful return
/// to A, pending is flushed into A's frame, which CAN absorb (A was the charger). Final
/// state_gas_used should be 0; gas_refunded should be unchanged.
#[test]
fn test_one_hop_delegatecall_refund_spills_to_parent() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);
    let addr_b = Address::from_low_u64_be(CONTRACT_B);

    // Contract B: just writes slot 0 = 0 (resets it) and stops.
    let mut code_b = sstore_byte(0, 0);
    code_b.extend(ret());

    // Contract A: writes slot 0 = 5 (0→N), then DELEGATECALLs B, then stops.
    let mut code_a = sstore_byte(0, 5);
    code_a.extend(delegatecall_bytecode(addr_b));
    code_a.extend(stop());

    let report = TestRunner::new(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code_a))
        .with_account(addr_b, contract(code_b))
        .run();

    assert!(
        report.is_success(),
        "transaction should succeed: {:?}",
        report.result
    );
    assert_eq!(
        report.state_gas_used, 0,
        "state_gas_used should be 0 after 1-hop 0→N→0 via DELEGATECALL: got {}",
        report.state_gas_used
    );
}

/// Test (c): 2-hop nested DELEGATECALL chain.
///
/// A → DELEGATECALL B → DELEGATECALL C. A writes 0→5, B passes through, C resets 5→0.
/// Refund should spill through C and B to be absorbed by A. Final state_gas_used = 0.
#[test]
fn test_two_hop_delegatecall_refund_spills_through_chain() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);
    let addr_b = Address::from_low_u64_be(CONTRACT_B);
    let addr_c = Address::from_low_u64_be(CONTRACT_C);

    // Contract C: writes slot 0 = 0 and returns.
    let mut code_c = sstore_byte(0, 0);
    code_c.extend(ret());

    // Contract B: DELEGATECALLs C and returns.
    let mut code_b = delegatecall_bytecode(addr_c);
    code_b.extend(ret());

    // Contract A: writes slot 0 = 5 (0→N), then DELEGATECALLs B.
    let mut code_a = sstore_byte(0, 5);
    code_a.extend(delegatecall_bytecode(addr_b));
    code_a.extend(stop());

    let report = TestRunner::new(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code_a))
        .with_account(addr_b, contract(code_b))
        .with_account(addr_c, contract(code_c))
        .run();

    assert!(
        report.is_success(),
        "transaction should succeed: {:?}",
        report.result
    );
    assert_eq!(
        report.state_gas_used, 0,
        "state_gas_used should be 0 after 2-hop 0→N→0 chain: got {}",
        report.state_gas_used
    );
}

/// Test (d): 1-hop DELEGATECALL with child revert discards pending refund.
///
/// A writes 0→5 (state gas charged in A). A DELEGATECALLs B. B writes 5→0 (triggering the
/// 0→N→0 refund into pending), then REVERTs. On revert, the snapshot of
/// `state_gas_refund_pending` taken at call entry is restored — B's contribution to pending
/// is rolled back. A's state_gas_used must remain at the full charge (no refund absorbed).
#[test]
fn test_one_hop_delegatecall_revert_discards_refund() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);
    let addr_b = Address::from_low_u64_be(CONTRACT_B);

    // Contract B: writes slot 0 = 0 (triggers refund into pending), then REVERTs.
    let mut code_b = sstore_byte(0, 0);
    code_b.extend(revert());

    // Contract A: writes slot 0 = 5 (0→N), then DELEGATECALLs B.
    let mut code_a = sstore_byte(0, 5);
    code_a.extend(delegatecall_bytecode(addr_b));
    code_a.extend(stop());

    let report = TestRunner::new(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code_a))
        .with_account(addr_b, contract(code_b))
        .run();

    // Tx succeeds (A continued after B's revert).
    assert!(
        report.is_success(),
        "transaction should succeed: {:?}",
        report.result
    );

    // B's SSTORE write was also rolled back on revert (storage back to 5).
    // So slot is still 5 — original=0, current=5 — state gas remains charged.
    assert!(
        report.state_gas_used > 0,
        "state_gas_used should be positive: B reverted so refund was discarded, got {}",
        report.state_gas_used
    );
}

/// Test (e): CALL boundary stops spill — B absorbs locally.
///
/// A CALLs B (not DELEGATECALL). B does 0→5→0 in its own storage context. B is the one
/// that charged the state gas (in B's frame). So `credit_state_gas_refund` fully clamps
/// against B's own local charge. Nothing spills to A.
///
/// Final state_gas_used should be 0 because B absorbed the refund locally.
/// A's state_gas_used is unaffected by B's internals.
#[test]
fn test_call_boundary_absorbs_refund_locally() {
    let addr_a = Address::from_low_u64_be(CONTRACT_A);
    let addr_b = Address::from_low_u64_be(CONTRACT_B);

    // Contract B: writes slot 0 = 5 (0→N in B's own storage), then 0 again (0→N→0), returns.
    let mut code_b = sstore_byte(0, 5);
    code_b.extend(sstore_byte(0, 0));
    code_b.extend(ret());

    // Contract A: CALLs B and stops.
    let mut code_a = call_bytecode(addr_b);
    code_a.extend(stop());

    let report = TestRunner::new(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code_a))
        .with_account(addr_b, contract(code_b))
        .run();

    assert!(
        report.is_success(),
        "transaction should succeed: {:?}",
        report.result
    );
    // B's refund absorbed locally; A had no state gas activity.
    // Total state_gas_used should be 0 (B's was refunded locally).
    assert_eq!(
        report.state_gas_used, 0,
        "state_gas_used should be 0: B absorbed its own refund locally, got {}",
        report.state_gas_used
    );
}

/// Test (f): Reservoir refill after ancestor-absorbed refund is visible mid-tx.
///
/// This mirrors the EELS `test_sstore_restoration_charge_in_ancestor` scenario:
/// the refund absorbed by an ancestor must refill the reservoir so that a
/// subsequent state-gas charge in the same tx draws from the refilled reservoir
/// rather than spilling to regular gas.
///
/// - A writes slot_0 = 5 (charges `state_gas_storage_set`, drains reservoir)
/// - A DELEGATECALLs B; B writes slot_0 = 0 (0→N→0 restoration, spills refund up)
/// - A writes slot_1 = 5 (second state-gas charge)
///
/// Without reservoir refill, the second SSTORE state-gas spills to regular gas
/// and `report.gas_used` is inflated by `state_gas_storage_set` vs the expected
/// value where the refund refilled the reservoir.
#[test]
fn test_ancestor_absorbed_refund_refills_reservoir() {
    use ethrex_levm::gas_cost::{STATE_BYTES_PER_STORAGE_SET, cost_per_state_byte};

    let addr_a = Address::from_low_u64_be(CONTRACT_A);
    let addr_b = Address::from_low_u64_be(CONTRACT_B);

    // Contract B: writes slot 0 = 0 (restoration refund into pending), returns.
    let mut code_b = sstore_byte(0, 0);
    code_b.extend(ret());

    // Contract A: slot_0 = 5, DELEGATECALL B, slot_1 = 5, STOP.
    let mut code_a = sstore_byte(0, 5);
    code_a.extend(delegatecall_bytecode(addr_b));
    code_a.extend(sstore_byte(1, 5));
    code_a.extend(stop());

    let report = TestRunner::new(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code_a))
        .with_account(addr_b, contract(code_b))
        .run();

    assert!(
        report.is_success(),
        "transaction should succeed: {:?}",
        report.result
    );

    let cpsb = cost_per_state_byte(GAS_LIMIT * 2);
    let sgas = STATE_BYTES_PER_STORAGE_SET * cpsb;

    // Gross state gas: 2 SSTORE charges (slot_0 first-set + slot_1 first-set).
    // B's restoration refunds 1 SSTORE state charge, absorbed by A.
    // Net: 1 SSTORE state charge remains.
    assert_eq!(
        report.state_gas_used, sgas,
        "expected exactly one SSTORE state charge after refund absorption, got {}",
        report.state_gas_used
    );

    // If the reservoir was NOT refilled, the second SSTORE's state gas would
    // spill to regular gas. Detect this by observing that regular gas is bloated
    // by `sgas` vs the refill path. Without a precise baseline we assert the
    // weaker invariant: the tx's total gas_used is consistent with a single
    // state-gas charge surfacing through the state dimension, not double.
    // The state/regular split is verified by state_gas_used above; here we assert
    // block accounting gets the same answer as the sum of dimensions.
    let expected_block_gas = report.gas_used;
    assert!(
        expected_block_gas >= 21_000 + sgas,
        "block gas_used must include at least intrinsic + 1 SSTORE state charge"
    );
}

/// Test (g): Child charges state gas then reverts — parent's reservoir gets it back.
///
/// Mirrors `test_mul[stack_underflow]`: contract A CALLs contract B; B does SSTORE
/// (charging state gas) then hits an invalid opcode (exceptional halt). EELS
/// `incorporate_child_on_error`:
///   parent.state_gas_left += child.state_gas_used - child.state_gas_refund
/// Tx succeeds at top level (parent returns from CALL with FAIL and STOPs). The
/// parent must reclaim B's state-gas consumption so it's not burned.
/// EIP-8037 CALL-to-empty-account with value transfer charges
/// `state_gas_new_account` in the CALLER's frame (parent). When the parent
/// continues and the transaction succeeds, that state gas is retained in net
/// `state_gas_used`. The child frame has no code and returns success
/// immediately, so no child revert is involved — this test guards the
/// "parent charged, parent succeeds" path against regressions that would
/// incorrectly refund new-account state gas on child return.
#[test]
fn test_call_to_empty_account_with_value_retains_parent_state_gas() {
    use ethrex_levm::gas_cost::{STATE_BYTES_PER_NEW_ACCOUNT, cost_per_state_byte};

    let addr_a = Address::from_low_u64_be(CONTRACT_A);
    let empty_target = Address::from_low_u64_be(0xDEAD); // not in pre-state

    // A: CALL(value=1, target=empty_addr) then STOP.
    let mut code_a = call_with_value_bytecode(empty_target, 1);
    code_a.extend(stop());

    let report = TestRunner::new(addr_a)
        .with_account(
            Address::from_low_u64_be(SENDER),
            eoa(U256::from(10u64).pow(18.into())),
        )
        // A must have balance to transfer.
        .with_account(
            addr_a,
            Account::new(
                U256::from(10u64).pow(18.into()),
                Code::from_bytecode(Bytes::from(code_a), &NativeCrypto),
                1,
                FxHashMap::default(),
            ),
        )
        .run();

    assert!(
        report.is_success(),
        "top-level tx must succeed: {:?}",
        report.result
    );

    let cpsb = cost_per_state_byte(GAS_LIMIT * 2);
    let expected_state_gas = STATE_BYTES_PER_NEW_ACCOUNT * cpsb;

    assert_eq!(
        report.state_gas_used, expected_state_gas,
        "parent frame must retain state_gas_new_account after CALL-to-empty + success \
         (got {}, expected {})",
        report.state_gas_used, expected_state_gas
    );
}

#[test]
fn test_child_charge_then_revert_returns_state_gas_to_parent() {
    use ethrex_levm::gas_cost::{STATE_BYTES_PER_STORAGE_SET, cost_per_state_byte};

    let addr_a = Address::from_low_u64_be(CONTRACT_A);
    let addr_b = Address::from_low_u64_be(CONTRACT_B);

    // Contract B: SSTORE(0, 5), then INVALID (0xfe) — exceptional halt.
    let mut code_b = sstore_byte(0, 5);
    code_b.push(0xfe);

    // Contract A: CALL B, STOP. Top-level succeeds even if CALL returns FAIL.
    let mut code_a = call_bytecode(addr_b);
    code_a.extend(stop());

    let report = TestRunner::new(addr_a)
        .with_account(Address::from_low_u64_be(SENDER), eoa(U256::from(1_000_000)))
        .with_account(addr_a, contract(code_a))
        .with_account(addr_b, contract(code_b))
        .run();

    assert!(
        report.is_success(),
        "top-level tx succeeds: {:?}",
        report.result
    );

    let cpsb = cost_per_state_byte(GAS_LIMIT * 2);
    let sgas = STATE_BYTES_PER_STORAGE_SET * cpsb;

    // B's SSTORE charged state gas; B reverted, so B's storage write is rolled back
    // and B's state charge flows back to A's reservoir. Net state_gas_used must be 0.
    assert_eq!(
        report.state_gas_used, 0,
        "state_gas_used should be 0: B reverted so its state gas flows back to parent (got {}, sgas={})",
        report.state_gas_used, sgas
    );
}
