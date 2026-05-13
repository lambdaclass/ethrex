//! EIP-8037 top-level reservoir reset — ethrex-specific divergence guards.
//!
//! The general top-level failure semantics (revert/halt/OOG refund execution
//! state gas, zero block-level state gas, CREATE-tx intrinsic survives, etc.)
//! are covered by `tests/amsterdam/eip8037_state_creation_gas_cost_increase/
//! test_state_gas_reservoir.py` in EELS and run via the blockchain ef-tests.
//!
//! The two tests below stay because they assert ethrex-only invariants that
//! ef-tests cannot express: they pin the block-level `gas_used` on halt paths
//! where ethrex's `max(spill_outstanding, reservoir_surplus)` reclassification
//! formula previously drifted from EELS by exactly one state-gas charge.

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
    gas_cost::{STATE_BYTES_PER_STORAGE_SET, cost_per_state_byte},
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
const GAS_LIMIT: u64 = 500_000;

// ==================== Bytecode helpers ====================

/// PUSH1 value, PUSH1 slot, SSTORE
fn sstore_byte(slot: u8, value: u8) -> Vec<u8> {
    vec![0x60, value, 0x60, slot, 0x55]
}

/// INVALID (0xfe) — causes exceptional halt
fn invalid_bytecode() -> Vec<u8> {
    vec![0xfe]
}

/// Inline CREATE-with-failing-initcode bytecode.
fn create_failing_bytecode(initcode_byte: u8) -> Vec<u8> {
    vec![
        0x60,
        initcode_byte, // PUSH1 <byte>
        0x60,
        0x00, // PUSH1 0
        0x53, // MSTORE8 — memory[0] = byte
        0x60,
        0x01, // PUSH1 1   (size)
        0x60,
        0x00, // PUSH1 0   (offset)
        0x60,
        0x00, // PUSH1 0   (value)
        0xf0, // CREATE
        0x50, // POP
    ]
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
}

impl TestRunner {
    fn call(target: Address) -> Self {
        Self {
            accounts: Vec::new(),
            target,
            is_create: false,
            calldata: Bytes::new(),
        }
    }

    fn create(initcode: Vec<u8>) -> Self {
        Self {
            accounts: Vec::new(),
            target: Address::default(),
            is_create: true,
            calldata: Bytes::from(initcode),
        }
    }

    fn with_account(mut self, addr: Address, acc: Account) -> Self {
        self.accounts.push((addr, acc));
        self
    }

    fn run(self) -> ExecutionReport {
        let gas_limit = GAS_LIMIT;
        let block_gas_limit = GAS_LIMIT * 2;
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

// ==================== Test: partial credit-to-spill diverges from EELS ====================

/// Top-level halt after an SSTORE charge (spilled, since reservoir = 0) and a
/// failed inner CREATE. Pins `report.gas_used` to the EELS reference value so
/// that ethrex's halt-reclassification formula stays aligned.
///
/// Per EELS `total_state - reservoir`, every byte of charged state-gas burned
/// by the halt must surface in the regular dimension; the pre-fix ethrex
/// formula was dropping the residual outstanding spill that the credit didn't
/// cancel, producing `gas_limit - SSTORE_STATE` instead of `gas_limit`.
#[test]
fn test_top_halt_after_partial_credit_to_spill_diverges_from_eels() {
    use ethrex_levm::gas_cost::STATE_BYTES_PER_NEW_ACCOUNT;

    let addr_a = Address::from_low_u64_be(CONTRACT_A);

    // SSTORE(slot 0 = 5); CREATE(failing initcode); INVALID
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

    let cpsb = cost_per_state_byte(GAS_LIMIT * 2);
    let _state_new = STATE_BYTES_PER_NEW_ACCOUNT * cpsb;
    let sstore_state = STATE_BYTES_PER_STORAGE_SET * cpsb;
    let expected_gas_used_eels = GAS_LIMIT;

    assert!(
        sstore_state > 0,
        "test scenario requires nonzero SSTORE state-gas to leave residual spill after credit"
    );

    assert_eq!(
        report.gas_used,
        expected_gas_used_eels,
        "block gas_used divergence: ethrex={} expected_eels={} diff={} (== one SSTORE state-gas charge); \
         ethrex's `max(spill_outstanding, reservoir_surplus)` halt formula drops the \
         residual outstanding spill that wasn't cancelled by the CREATE-failure refund",
        report.gas_used,
        expected_gas_used_eels,
        expected_gas_used_eels.saturating_sub(report.gas_used),
    );
}

// ==================== Test: phantom drain credit must not cancel real spill ====================

/// Regression for the bal-devnet-6 block-21 fork between ethrex and geth on a
/// CREATE TX whose initcode performs two failing inner CREATEs.
///
/// Asserts the phantom-drain-credit from refunding the reservoir-funded second
/// inner CREATE does not cancel the real spill from the first inner CREATE.
/// Pre-fix ethrex reported `gas_limit - STATE_NEW` instead of `gas_limit`.
#[test]
fn test_top_halt_phantom_drain_does_not_cancel_real_spill() {
    use ethrex_levm::gas_cost::STATE_BYTES_PER_NEW_ACCOUNT;

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

    let expected_gas_used_eels = GAS_LIMIT;

    assert_eq!(
        report.gas_used,
        expected_gas_used_eels,
        "block gas_used divergence: ethrex={} expected_eels={} diff={} (== one NEW_ACCOUNT state-gas charge); \
         the phantom drain credit from refunding the reservoir-funded second inner CREATE \
         must not cancel the real spill from the first inner CREATE",
        report.gas_used,
        expected_gas_used_eels,
        expected_gas_used_eels.saturating_sub(report.gas_used),
    );
}
