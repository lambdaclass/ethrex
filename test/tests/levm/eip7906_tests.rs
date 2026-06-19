//! EIP-7906: TXTRACE (0xB5) and EVENTDATACOPY (0xB6)
//!
//! Full-VM bytecode tests exercising the two transaction-introspection opcodes
//! through a normal EIP-1559 transaction. Each test deploys a contract whose
//! bytecode optionally mutates state, then invokes the opcode and surfaces the
//! result either via `RETURN` (read from `report.output`) or via `SSTORE`
//! (read from the post-execution `current_accounts_state`). The harness returns
//! BOTH the report and the database so either assertion style works.
//!
//! Opcodes are gated at `Fork::Hegota`; one test asserts they are invalid on an
//! earlier fork.
//!
//! TXTRACE stack order: handler pops `[in2, param]` (in2 on top), so bytecode
//! pushes `param` first, then `in2`.
//! EVENTDATACOPY stack order: handler pops `[event_index, mem_offset,
//! data_offset, length]` (event_index on top), so bytecode pushes in reverse:
//! length, dataOffset, memOffset, event_index.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        Account, AuthorizationTuple, Code, EIP1559Transaction, EIP7702Transaction, Fork,
        Transaction, TxKind,
    },
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    db::gen_db::GeneralizedDatabase,
    environment::{EVMConfig, Environment},
    errors::ExecutionReport,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_rlp::encode::RLPEncode;
use rustc_hash::FxHashMap;
use secp256k1::{Message as SecpMessage, PublicKey, SECP256K1, SecretKey};
use std::sync::Arc;

use crate::levm::test_db::TestDatabase;

// ==================== Opcode bytes ====================

const TXTRACE: u8 = 0xB5;
const EVENTDATACOPY: u8 = 0xB6;

// EVM opcodes used by the bytecode builders.
const PUSH1: u8 = 0x60;
const PUSH20: u8 = 0x73;
const PUSH32: u8 = 0x7f;
const MSTORE: u8 = 0x52;
const MSTORE8: u8 = 0x53;
const MLOAD: u8 = 0x51;
const SSTORE: u8 = 0x55;
const RETURN: u8 = 0xf3;
const STOP: u8 = 0x00;
const POP: u8 = 0x50;
const GAS: u8 = 0x5a;
const CALL: u8 = 0xf1;
const STATICCALL: u8 = 0xfa;
const CREATE: u8 = 0xf0;
const LOG0: u8 = 0xa0;
const LOG1: u8 = 0xa1;
const REVERT: u8 = 0xfd;
const JUMPDEST: u8 = 0x5b;
const JUMPI: u8 = 0x57;
const EQ: u8 = 0x14;
const LOG2: u8 = 0xa2;

// ==================== Constants ====================

const DEFAULT_BALANCE: u64 = 1_000_000_000_000_000;
const GAS_LIMIT: u64 = 5_000_000;
const GAS_PRICE: u64 = 1000;

const SENDER: u64 = 0x1000;
const CONTRACT: u64 = 0x3000;
const OTHER: u64 = 0x5000;

fn sender_addr() -> Address {
    Address::from_low_u64_be(SENDER)
}
fn contract_addr() -> Address {
    Address::from_low_u64_be(CONTRACT)
}
fn other_addr() -> Address {
    Address::from_low_u64_be(OTHER)
}

// ==================== Account helpers ====================

fn eoa(balance: U256) -> Account {
    Account::new(balance, Code::default(), 0, FxHashMap::default())
}

fn contract_acct(code: Vec<u8>, balance: U256) -> Account {
    Account::new(
        balance,
        Code::from_bytecode(Bytes::from(code), &NativeCrypto),
        0,
        FxHashMap::default(),
    )
}

// ==================== Harness ====================

/// A normal EIP-1559 transaction harness. Seeds `accounts`, auto-seeds the
/// sender as a funded EOA (nonce 0) if absent, runs `VM::execute()` at the
/// chosen fork, and returns BOTH the report and the post-execution database.
struct Harness {
    accounts: Vec<(Address, Account)>,
    fork: Fork,
    sender: Address,
    to: Address,
    value: U256,
    gas_price: u64,
    sender_balance: U256,
    /// Optional pre-built transaction. When `Some`, it is executed verbatim and
    /// the harness skips building its default EIP-1559 transfer (used by the
    /// EIP-7702 delegation test, which needs a type-4 authorization-list tx).
    tx_override: Option<Transaction>,
    /// `tx_blob_hashes` placed in the env. Exercises the blob term of
    /// gas_pre_charge for a normal tx. Left empty for non-blob tests.
    tx_blob_hashes: Vec<H256>,
    /// `base_blob_fee_per_gas` placed in the env (defaults to 1).
    base_blob_fee_per_gas: U256,
}

impl Harness {
    fn new() -> Self {
        Self {
            accounts: Vec::new(),
            fork: Fork::Hegota,
            sender: sender_addr(),
            to: contract_addr(),
            value: U256::zero(),
            gas_price: GAS_PRICE,
            sender_balance: U256::from(DEFAULT_BALANCE),
            tx_override: None,
            tx_blob_hashes: Vec::new(),
            base_blob_fee_per_gas: U256::from(1),
        }
    }

    /// Place `hashes` in `env.tx_blob_hashes` and set `env.base_blob_fee_per_gas`
    /// so the blob term of gas_pre_charge is exercised. The tx stays a normal
    /// EIP-1559 transfer (no `tx_max_fee_per_blob_gas`), so 4844 validation does
    /// not run.
    fn blob_env(mut self, hashes: Vec<H256>, base_blob_fee: U256) -> Self {
        self.tx_blob_hashes = hashes;
        self.base_blob_fee_per_gas = base_blob_fee;
        self
    }

    /// Execute `tx` verbatim instead of the default EIP-1559 transfer. The
    /// transaction's `to` must still match the harness `to` so the seeded
    /// target contract is invoked.
    fn tx(mut self, tx: Transaction) -> Self {
        self.tx_override = Some(tx);
        self
    }

    fn fork(mut self, fork: Fork) -> Self {
        self.fork = fork;
        self
    }

    fn account(mut self, addr: Address, acc: Account) -> Self {
        self.accounts.push((addr, acc));
        self
    }

    #[allow(dead_code)]
    fn to(mut self, addr: Address) -> Self {
        self.to = addr;
        self
    }

    #[allow(dead_code)]
    fn value(mut self, v: U256) -> Self {
        self.value = v;
        self
    }

    fn gas_price(mut self, p: u64) -> Self {
        self.gas_price = p;
        self
    }

    fn run(self) -> (ExecutionReport, GeneralizedDatabase) {
        let mut accounts_map: FxHashMap<Address, Account> = self.accounts.into_iter().collect();
        accounts_map
            .entry(self.sender)
            .or_insert_with(|| eoa(self.sender_balance));

        let test_db = TestDatabase {
            accounts: FxHashMap::default(),
        };
        let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(test_db), accounts_map);

        let blob_schedule = EVMConfig::canonical_values(self.fork);
        let env = Environment {
            origin: self.sender,
            gas_limit: GAS_LIMIT,
            config: EVMConfig::new(self.fork, blob_schedule),
            block_number: 1,
            coinbase: Address::from_low_u64_be(0xCCC),
            timestamp: 1000,
            prev_randao: Some(H256::zero()),
            difficulty: U256::zero(),
            slot_number: U256::zero(),
            chain_id: U256::from(1),
            base_fee_per_gas: U256::from(self.gas_price.min(1)),
            base_blob_fee_per_gas: self.base_blob_fee_per_gas,
            gas_price: U256::from(self.gas_price),
            block_excess_blob_gas: None,
            block_blob_gas_used: None,
            tx_blob_hashes: self.tx_blob_hashes.clone(),
            tx_max_priority_fee_per_gas: None,
            tx_max_fee_per_gas: Some(U256::from(self.gas_price)),
            tx_max_fee_per_blob_gas: None,
            tx_nonce: 0,
            block_gas_limit: GAS_LIMIT * 2,
            is_privileged: false,
            fee_token: None,
            disable_balance_check: false,
            is_system_call: false,
        };

        let tx = self.tx_override.unwrap_or_else(|| {
            Transaction::EIP1559Transaction(EIP1559Transaction {
                to: TxKind::Call(self.to),
                value: self.value,
                data: Bytes::new(),
                gas_limit: GAS_LIMIT,
                max_fee_per_gas: self.gas_price,
                max_priority_fee_per_gas: 0,
                ..Default::default()
            })
        });

        let result = {
            let mut vm = VM::new(
                env,
                &mut db,
                &tx,
                LevmCallTracer::disabled(),
                VMType::L1,
                &NativeCrypto,
            )
            .expect("VM::new should succeed");
            vm.execute()
                .expect("execute() returns Ok even on revert/halt")
        };
        (result, db)
    }
}

/// Read storage slot `key` of `addr` from the post-execution cache.
fn storage_slot(db: &GeneralizedDatabase, addr: Address, key: H256) -> U256 {
    db.current_accounts_state
        .get(&addr)
        .and_then(|acc| acc.storage.get(&key).copied())
        .unwrap_or_default()
}

/// Read balance of `addr` from the post-execution cache.
#[allow(dead_code)]
fn balance_of(db: &GeneralizedDatabase, addr: Address) -> U256 {
    db.current_accounts_state
        .get(&addr)
        .map(|acc| acc.info.balance)
        .unwrap_or_default()
}

/// `report.output` as a U256 (asserts a 32-byte return).
fn output_word(report: &ExecutionReport) -> U256 {
    assert_eq!(
        report.output.len(),
        32,
        "expected a 32-byte RETURN, got {} bytes",
        report.output.len()
    );
    U256::from_big_endian(&report.output)
}

/// The standard 12-zero-prefixed address word that `address_to_u256` produces.
fn address_word(addr: Address) -> U256 {
    let mut buf = [0u8; 32];
    buf[12..].copy_from_slice(addr.as_bytes());
    U256::from_big_endian(&buf)
}

// ==================== Bytecode builders ====================

fn push1(v: u8) -> Vec<u8> {
    vec![PUSH1, v]
}

fn push32(v: U256) -> Vec<u8> {
    let mut out = vec![PUSH32];
    out.extend_from_slice(&v.to_big_endian());
    out
}

fn push20(addr: Address) -> Vec<u8> {
    let mut out = vec![PUSH20];
    out.extend_from_slice(addr.as_bytes());
    out
}

/// `... <body> ; PUSH1 0; MSTORE ; PUSH1 32; PUSH1 0; RETURN`.
/// Assumes `body` leaves exactly one 32-byte word on the stack.
fn wrap_return(mut body: Vec<u8>) -> Vec<u8> {
    body.extend(push1(0)); // mem offset for MSTORE
    body.push(MSTORE);
    body.extend(push1(32)); // size
    body.extend(push1(0)); // offset
    body.push(RETURN);
    body
}

/// TXTRACE(param, in2) leaving the result word on the stack.
/// Bytecode: PUSH1 param ; PUSH1 in2 ; TXTRACE.
fn txtrace(param: u8, in2: u8) -> Vec<u8> {
    let mut out = push1(param);
    out.extend(push1(in2));
    out.push(TXTRACE);
    out
}

/// TXTRACE(param, in2) with a U256 index operand.
fn txtrace_idx(param: u8, idx: U256) -> Vec<u8> {
    let mut out = push1(param);
    out.extend(push32(idx));
    out.push(TXTRACE);
    out
}

/// A contract that runs TXTRACE(param,in2) and RETURNs the 32-byte result.
fn txtrace_return_code(param: u8, in2: u8) -> Vec<u8> {
    wrap_return(txtrace(param, in2))
}

/// SSTORE val@slot.
fn sstore(slot: u8, val: U256) -> Vec<u8> {
    let mut out = push32(val);
    out.extend(push1(slot));
    out.push(SSTORE);
    out
}

/// LOG0 over memory [offset, offset+size).
fn log0(offset: u8, size: u8) -> Vec<u8> {
    let mut out = push1(size);
    out.extend(push1(offset));
    out.push(LOG0);
    out
}

/// LOG1(topic0) over memory [offset, offset+size).
fn log1(offset: u8, size: u8, topic0: U256) -> Vec<u8> {
    let mut out = push32(topic0);
    out.extend(push1(size));
    out.extend(push1(offset));
    out.push(LOG1);
    out
}

/// LOG2(topic0, topic1) over memory [offset, offset+size). Pushed bottom-up:
/// topic1, topic0, then size, offset (stack pops offset, size, topic0, topic1).
fn log2(offset: u8, size: u8, topic0: U256, topic1: U256) -> Vec<u8> {
    let mut out = push32(topic1);
    out.extend(push32(topic0));
    out.extend(push1(size));
    out.extend(push1(offset));
    out.push(LOG2);
    out
}

/// Store one 32-byte word `val` at memory offset 0 (so a following LOG can use
/// it as data).
fn mstore_word(val: U256) -> Vec<u8> {
    let mut out = push32(val);
    out.extend(push1(0));
    out.push(MSTORE);
    out
}

/// CALL `target` with no value/args, popping the success flag.
fn call(target: Address) -> Vec<u8> {
    let mut out = push1(0); // retSize
    out.extend(push1(0)); // retOffset
    out.extend(push1(0)); // argsSize
    out.extend(push1(0)); // argsOffset
    out.extend(push1(0)); // value
    out.extend(push20(target)); // address
    out.push(GAS);
    out.push(CALL);
    out.push(POP);
    out
}

/// CALL `target` with `value` (no args), popping the success flag.
fn call_value(target: Address, value: U256) -> Vec<u8> {
    let mut out = push1(0); // retSize
    out.extend(push1(0)); // retOffset
    out.extend(push1(0)); // argsSize
    out.extend(push1(0)); // argsOffset
    out.extend(push32(value)); // value
    out.extend(push20(target)); // address
    out.push(GAS);
    out.push(CALL);
    out.push(POP);
    out
}

/// CALL `target` with 32 bytes of calldata at mem[0..32]; REVERT the caller if
/// the subcall reverted. Models a wallet-injected assertion suffix.
fn call_assertion(target: Address) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(push1(32)); // retSize
    out.extend(push1(0)); // retOffset
    out.extend(push1(32)); // argsSize
    out.extend(push1(0)); // argsOffset
    out.extend(push1(0)); // value
    out.extend(push20(target));
    out.push(GAS);
    out.push(CALL);
    out.push(0x15); // ISZERO -> 1 if reverted
    out.extend(push1(0));
    let revert_dest_idx = out.len() - 1;
    out.push(JUMPI);
    out.push(STOP);
    let revert_dest = out.len();
    out[revert_dest_idx] = revert_dest as u8;
    out.push(JUMPDEST);
    out.extend(push1(0));
    out.extend(push1(0));
    out.push(REVERT);
    out
}

/// CREATE a contract from `init_code` with zero value, leaving the new address
/// on the stack.
fn create(init_code: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    // Store init_code into memory byte-by-byte.
    for (i, byte) in init_code.iter().enumerate() {
        out.extend(push1(*byte));
        out.extend(push1(i as u8));
        out.push(MSTORE8);
    }
    out.extend(push1(init_code.len() as u8)); // size
    out.extend(push1(0)); // offset
    out.extend(push1(0)); // value
    out.push(CREATE);
    out
}

// ==================== COUNTS & STATE DIFF ====================

#[test]
fn storage_count_zero_when_no_sstore() {
    // gas_price = 0 so the sender does not appear in any balance diff; the
    // contract writes nothing => slots_changed count (param 0x01) == 0.
    let code = txtrace_return_code(0x01, 0x00);
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .gas_price(0)
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    assert_eq!(output_word(&report), U256::zero());
}

#[test]
fn storage_count_n_after_n_writes() {
    // Three distinct slot writes, then TXTRACE(0x01) => count 3.
    let mut code = sstore(0x01, U256::from(11));
    code.extend(sstore(0x02, U256::from(22)));
    code.extend(sstore(0x03, U256::from(33)));
    code.extend(txtrace_return_code(0x01, 0x00));
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    assert_eq!(output_word(&report), U256::from(3));
}

#[test]
fn slots_returned_sorted_by_address_then_slot() {
    // The contract writes two slots (3 then 1); the gas-payer (sender) also has
    // a balance change but that is in balance_changes, not slot_changes.
    // slot_changes for one address must be sorted by slot uint256 asc:
    // index 0 -> slot 1, index 1 -> slot 3.
    let mut code = sstore(0x03, U256::from(99));
    code.extend(sstore(0x01, U256::from(77)));
    // Read slot_key at index 0 and index 1 into storage slots 0x10 / 0x11 so we
    // can compare ordering after execution.
    code.extend({
        let mut c = txtrace_idx(0x07, U256::zero()); // slot_key @ index 0
        c.extend(push1(0x10));
        c.push(SSTORE);
        c
    });
    code.extend({
        let mut c = txtrace_idx(0x07, U256::one()); // slot_key @ index 1
        c.extend(push1(0x11));
        c.push(SSTORE);
        c
    });
    code.push(STOP);
    let (report, db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    let first = storage_slot(&db, contract_addr(), H256::from_low_u64_be(0x10));
    let second = storage_slot(&db, contract_addr(), H256::from_low_u64_be(0x11));
    assert_eq!(
        first,
        U256::from(1),
        "index 0 should be the lowest slot key"
    );
    assert_eq!(
        second,
        U256::from(3),
        "index 1 should be the higher slot key"
    );
}

#[test]
fn slot_restored_to_original_excluded() {
    // Contract is seeded with slot 0x05 = 42. It overwrites to 99 then restores
    // to 42 within the tx => the slot's current value equals its prestate, so it
    // is NOT counted. Only the (gas_price=0) makes balances empty too => count 0.
    let mut storage = FxHashMap::default();
    storage.insert(H256::from_low_u64_be(0x05), U256::from(42));
    let mut code = sstore(0x05, U256::from(99));
    code.extend(sstore(0x05, U256::from(42))); // restore
    code.extend(txtrace_return_code(0x01, 0x00));
    let acct = Account::new(
        U256::zero(),
        Code::from_bytecode(Bytes::from(code), &NativeCrypto),
        0,
        storage,
    );
    let (report, _db) = Harness::new()
        .account(contract_addr(), acct)
        .gas_price(0)
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    assert_eq!(
        output_word(&report),
        U256::zero(),
        "slot restored to its prestate value must be excluded"
    );
}

#[test]
fn net_zero_balance_change_excluded() {
    // gas_price = 0 and no value transfer => no account's balance differs from
    // prestate => balances_changed count (param 0x00) == 0.
    let code = txtrace_return_code(0x00, 0x00);
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .gas_price(0)
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    assert_eq!(output_word(&report), U256::zero());
}

#[test]
fn balance_before_zero_for_newly_created_account() {
    // The contract transfers value to a brand-new OTHER account (absent from
    // prestate). Its balance_before (param 0x05 reads after; 0x04 reads before)
    // must be 0. We locate OTHER in balance_changes by scanning indices: OTHER
    // is created during the tx so its before == 0 and after == value.
    // Construction: contract holds funds, CALLs OTHER with value, then for the
    // balance change whose address == OTHER asserts before == 0.
    let value = U256::from(123_456u64);
    // Contract: CALL OTHER with `value`, then SSTORE balances_changed count to
    // slot 0, and for each index store (address, before) so the test can find
    // OTHER and check its before == 0.
    let mut code = Vec::new();
    // CALL OTHER with value.
    code.extend(push1(0)); // retSize
    code.extend(push1(0)); // retOffset
    code.extend(push1(0)); // argsSize
    code.extend(push1(0)); // argsOffset
    code.extend(push32(value)); // value
    code.extend(push20(other_addr())); // address
    code.push(GAS);
    code.push(CALL);
    code.push(POP);
    // Store count at slot 0.
    code.extend({
        let mut c = txtrace(0x00, 0x00);
        c.extend(push1(0));
        c.push(SSTORE);
        c
    });
    // At TXTRACE time (before coinbase payout) the balance changes are exactly
    // the sender (gas pre-charge), the contract (sent value), and OTHER
    // (received value): three entries, sorted ascending by address. Probe all
    // three; reading a non-existent index would itself halt.
    for i in 0u8..3 {
        code.extend({
            let mut c = txtrace_idx(0x03, U256::from(i)); // change address
            c.extend(push1(0x20 + i));
            c.push(SSTORE);
            c
        });
        code.extend({
            let mut c = txtrace_idx(0x04, U256::from(i)); // balance before
            c.extend(push1(0x30 + i));
            c.push(SSTORE);
            c
        });
    }
    code.push(STOP);
    // OTHER is deliberately NOT seeded: it is absent from the prestate, so it
    // exercises the "absent from initial state => balance_before == 0" path.
    let (report, db) = Harness::new()
        .account(
            contract_addr(),
            contract_acct(code, U256::from(DEFAULT_BALANCE)),
        )
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    let count = usize::try_from(storage_slot(&db, contract_addr(), H256::from_low_u64_be(0)))
        .expect("balance-change count must fit in usize");
    let other_word = address_word(other_addr());
    let mut found = false;
    for i in 0..count.min(3) {
        let addr = storage_slot(&db, contract_addr(), H256::from_low_u64_be(0x20 + i as u64));
        if addr == other_word {
            let before = storage_slot(&db, contract_addr(), H256::from_low_u64_be(0x30 + i as u64));
            assert_eq!(
                before,
                U256::zero(),
                "newly-created account balance_before must be 0"
            );
            found = true;
        }
    }
    assert!(
        found,
        "OTHER (created during tx) must appear in balance_changes"
    );
}

#[test]
fn balance_changes_multi_address_sorted() {
    // The contract transfers value to TWO recipients chosen so their natural
    // (uint160 ascending) order is non-trivial: LOW (0x0001) and HIGH (0x9999),
    // straddling the contract's own address (0x3000). With gas_price = 0 the
    // sender does not appear, so balance_changes is exactly:
    //   {LOW, contract, HIGH}  (3 entries, ascending by address).
    // We read the count (param 0x00) and the change_address (param 0x03) at
    // indices 0 and 1, and assert the result is strictly ascending — exercising
    // the uint160 sort across a low-vs-high recipient pair.
    let low = Address::from_low_u64_be(0x0001);
    let high = Address::from_low_u64_be(0x9999);
    let value = U256::from(7_777u64);

    let mut code = call_value(low, value);
    code.extend(call_value(high, value));
    // count @0
    code.extend({
        let mut c = txtrace(0x00, 0x00);
        c.extend(push1(0));
        c.push(SSTORE);
        c
    });
    // change_address at index 0 -> slot 1, index 1 -> slot 2.
    code.extend(store_param_idx(0x03, 0, 1));
    code.extend(store_param_idx(0x03, 1, 2));
    code.push(STOP);

    let (report, db) = Harness::new()
        .account(
            contract_addr(),
            contract_acct(code, U256::from(DEFAULT_BALANCE)),
        )
        .gas_price(0)
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    let get = |slot: u64| storage_slot(&db, contract_addr(), H256::from_low_u64_be(slot));
    assert_eq!(
        get(0),
        U256::from(3),
        "two recipients plus the contract changed balance"
    );
    let addr0 = get(1);
    let addr1 = get(2);
    assert!(
        addr0 < addr1,
        "balance_changes must be sorted ascending by address (uint160): \
         index 0 ({addr0:#x}) must be < index 1 ({addr1:#x})"
    );
    // index 0 must be the lowest of the three addresses (LOW = 0x0001).
    assert_eq!(
        addr0,
        address_word(low),
        "index 0 must be the lowest recipient address"
    );
}

// ==================== DEPLOYED CONTRACTS ====================

#[test]
fn create_appears_in_deployed_with_codehash() {
    // init_code returns a 1-byte body (0x00 STOP): PUSH1 0x00, PUSH1 0, MSTORE8,
    // PUSH1 1, PUSH1 0, RETURN.
    let init_code = {
        let mut c = push1(STOP);
        c.extend(push1(0));
        c.push(MSTORE8);
        c.extend(push1(1)); // size
        c.extend(push1(0)); // offset
        c.push(RETURN);
        c
    };
    // Contract CREATEs, then stores deployed count @0 and codehash@1.
    let mut code = create(&init_code);
    code.push(POP); // discard the new address
    code.extend({
        let mut c = txtrace(0x02, 0x00); // deployed count
        c.extend(push1(0));
        c.push(SSTORE);
        c
    });
    code.extend({
        let mut c = txtrace_idx(0x0B, U256::zero()); // codehash @ index 0
        c.extend(push1(1));
        c.push(SSTORE);
        c
    });
    code.push(STOP);
    let (report, db) = Harness::new()
        .account(
            contract_addr(),
            contract_acct(code, U256::from(DEFAULT_BALANCE)),
        )
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    assert_eq!(
        storage_slot(&db, contract_addr(), H256::zero()),
        U256::one(),
        "exactly one contract deployed"
    );
    // The expected codehash is keccak256 of the deployed body (single 0x00 byte).
    let expected = Code::from_bytecode(Bytes::from(vec![STOP]), &NativeCrypto).hash;
    assert_eq!(
        storage_slot(&db, contract_addr(), H256::from_low_u64_be(1)),
        U256::from_big_endian(expected.as_bytes()),
        "codehash_after must match the deployed body"
    );
}

#[test]
fn reverting_create_not_counted() {
    // init_code reverts: PUSH1 0, PUSH1 0, REVERT. CREATE returns 0 address and
    // deploys nothing => deployed count == 0.
    let init_code = {
        let mut c = push1(0);
        c.extend(push1(0));
        c.push(0xfd); // REVERT
        c
    };
    let mut code = create(&init_code);
    code.push(POP);
    code.extend(txtrace_return_code(0x02, 0x00));
    let (report, _db) = Harness::new()
        .account(
            contract_addr(),
            contract_acct(code, U256::from(DEFAULT_BALANCE)),
        )
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    assert_eq!(
        output_word(&report),
        U256::zero(),
        "a reverting CREATE must not be counted"
    );
}

#[test]
fn preexisting_code_account_not_counted() {
    // OTHER already has code in the prestate. The contract does nothing but read
    // the deployed count: OTHER had code at prestate => not a deployment => 0.
    let code = txtrace_return_code(0x02, 0x00);
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .account(other_addr(), contract_acct(vec![STOP], U256::zero()))
        .gas_price(0)
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    assert_eq!(
        output_word(&report),
        U256::zero(),
        "an account with prestate code must not be counted as deployed"
    );
}

// ==================== EIP-7702 DELEGATION EXCLUSION ====================

const EIP_7702_MAGIC: u8 = 0x05;

/// Fixed authority secret key (32 bytes of 0x42), distinct from the harness
/// sender/contract/other accounts. It signs the authorization tuple; the VM
/// recovers its address via ecrecover and writes the delegation designator
/// (`0xef0100 || target`) into its previously-empty account.
const AUTHORITY_SK_BYTES: [u8; 32] = [0x42u8; 32];

/// Delegation target written into the authority's code by the 7702 auth.
const DELEGATION_TARGET: u64 = 0xDEAD;

/// Recover the address that `secret_key` maps to (pubkey -> keccak -> last 20
/// bytes), mirroring `wrong_chain_id_tests::sender_address`.
fn address_from_sk(secret_key: &SecretKey) -> Address {
    let pk = PublicKey::from_secret_key(SECP256K1, secret_key);
    let hash = keccak(&pk.serialize_uncompressed()[1..]);
    Address::from_slice(&hash.as_bytes()[12..])
}

/// Sign an EIP-7702 authorization tuple over `(chain_id, address, nonce)`,
/// adapted from `eip7702_zero_transfer_tests::sign_auth_tuple`.
fn sign_auth_tuple(
    chain_id: u64,
    address: Address,
    nonce: u64,
    secret_key: &SecretKey,
) -> AuthorizationTuple {
    let mut rlp_buf = Vec::new();
    rlp_buf.push(EIP_7702_MAGIC);
    (U256::from(chain_id), address, nonce).encode(&mut rlp_buf);
    let hash = keccak(&rlp_buf);

    let msg = SecpMessage::from_digest(hash.0);
    let (recovery_id, sig) = SECP256K1
        .sign_ecdsa_recoverable(&msg, secret_key)
        .serialize_compact();

    let r = U256::from_big_endian(&sig[..32]);
    let s = U256::from_big_endian(&sig[32..64]);
    let y_parity = U256::from(Into::<i32>::into(recovery_id) as u64);

    AuthorizationTuple {
        chain_id: U256::from(chain_id),
        address,
        nonce,
        y_parity,
        r_signature: r,
        s_signature: s,
    }
}

#[test]
fn eip7702_delegation_excluded_but_create_counted() {
    // An account delegated via EIP-7702 DURING this tx (its code goes from empty
    // to a `0xef0100 || target` designator) must be EXCLUDED from
    // contracts_deployed, while a genuine CREATE in the same tx IS counted.
    //
    // The harness origin (sender) sends a type-4 tx whose single authorization
    // is signed by a third account (the authority). `prepare_execution` ->
    // `validate_type_4_tx` -> `eip7702_set_access_code` recovers the authority
    // and writes the delegation designator into its (empty) account before the
    // target contract runs.
    let authority_sk = SecretKey::from_slice(&AUTHORITY_SK_BYTES).expect("valid authority key");
    let authority = address_from_sk(&authority_sk);
    let target = Address::from_low_u64_be(DELEGATION_TARGET);

    assert_ne!(
        authority,
        sender_addr(),
        "authority must differ from sender"
    );
    assert_ne!(
        authority,
        contract_addr(),
        "authority must differ from the target contract"
    );

    // init_code returns a 1-byte body (0x00 STOP): PUSH1 0x00, PUSH1 0, MSTORE8,
    // PUSH1 1, PUSH1 0, RETURN. Identical to `create_appears_in_deployed`'s body.
    let init_code = {
        let mut c = push1(STOP);
        c.extend(push1(0));
        c.push(MSTORE8);
        c.extend(push1(1)); // size
        c.extend(push1(0)); // offset
        c.push(RETURN);
        c
    };
    // The CREATEd contract's address is deterministic (CREATE: keccak(rlp(sender,
    // nonce))). We assert the deployed address is NOT the authority rather than
    // hard-coding the CREATE address.
    let mut code = create(&init_code);
    code.push(POP); // discard the new address
    // deployed count @0
    code.extend({
        let mut c = txtrace(0x02, 0x00);
        c.extend(push1(0));
        c.push(SSTORE);
        c
    });
    // deployed_address @ index 0 -> slot 1
    code.extend({
        let mut c = txtrace_idx(0x0A, U256::zero());
        c.extend(push1(1));
        c.push(SSTORE);
        c
    });
    code.push(STOP);

    // Authority is seeded as an empty-code EOA at nonce 0 so the auth tuple
    // (nonce 0) is accepted.
    let authority_acct = eoa(U256::zero());

    let auth = sign_auth_tuple(1, target, 0, &authority_sk);
    let tx = Transaction::EIP7702Transaction(EIP7702Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: GAS_PRICE,
        gas_limit: GAS_LIMIT,
        to: contract_addr(),
        value: U256::zero(),
        data: Bytes::new(),
        access_list: Default::default(),
        authorization_list: vec![auth],
        ..Default::default()
    });

    let (report, db) = Harness::new()
        .account(
            contract_addr(),
            contract_acct(code, U256::from(DEFAULT_BALANCE)),
        )
        .account(authority, authority_acct)
        .tx(tx)
        .run();
    assert!(
        report.is_success(),
        "type-4 tx should succeed: {:?}",
        report.result
    );

    // Sanity: the authority really did get delegated during the tx (its live
    // code is the `0xef0100 || target` designator), so it is a genuine
    // exclusion candidate, not a no-op.
    let authority_code_hash = db
        .current_accounts_state
        .get(&authority)
        .map(|acc| acc.info.code_hash)
        .expect("authority must be in the post-state");
    let designator: Bytes = [&[0xefu8, 0x01, 0x00][..], target.as_bytes()]
        .concat()
        .into();
    let expected_designator_hash = Code::from_bytecode(designator, &NativeCrypto).hash;
    assert_eq!(
        authority_code_hash, expected_designator_hash,
        "authority must hold the 7702 delegation designator after the tx"
    );

    // contracts_deployed counts ONLY the CREATEd contract; the 7702-delegated
    // authority is excluded.
    assert_eq!(
        storage_slot(&db, contract_addr(), H256::zero()),
        U256::one(),
        "only the CREATEd contract is counted; the 7702 delegation is excluded"
    );

    // The single deployed address must NOT be the authority.
    let deployed_addr_word = storage_slot(&db, contract_addr(), H256::from_low_u64_be(1));
    assert_ne!(
        deployed_addr_word,
        address_word(authority),
        "the deployed address must be the CREATEd contract, not the 7702 authority"
    );
    assert_ne!(
        deployed_addr_word,
        U256::zero(),
        "a genuine CREATE must yield a non-zero deployed address"
    );
}

// ==================== EVENTS ====================

#[test]
fn events_count_address_topics_and_data_len() {
    // Emit: LOG0 (no topics, data = 32-byte word 0xAA..), LOG2 with two topics
    // and the same data. Then read events_count, and for each event the address,
    // topic_count, topics, and data_len.
    let data_word = U256::from(0xAABBu64);
    let topic0 = U256::from(0x1111u64);
    let topic1 = U256::from(0x2222u64);

    let mut code = mstore_word(data_word);
    code.extend(log0(0, 32)); // event 0: 0 topics, 32 data bytes
    code.extend(log2(0, 32, topic0, topic1)); // event 1: 2 topics, 32 data bytes

    // events_count @0
    code.extend({
        let mut c = txtrace(0x0C, 0x00);
        c.extend(push1(0));
        c.push(SSTORE);
        c
    });
    // event0 address @1, topic_count @2, data_len @3
    code.extend(store_param_idx(0x0D, 0, 1));
    code.extend(store_param_idx(0x0E, 0, 2));
    code.extend(store_param_idx(0x13, 0, 3));
    // event1 address @4, topic_count @5, topic0 @6, topic1 @7, data_len @8
    code.extend(store_param_idx(0x0D, 1, 4));
    code.extend(store_param_idx(0x0E, 1, 5));
    code.extend(store_param_idx(0x0F, 1, 6));
    code.extend(store_param_idx(0x10, 1, 7));
    code.extend(store_param_idx(0x13, 1, 8));
    code.push(STOP);

    let (report, db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );

    let get = |slot: u64| storage_slot(&db, contract_addr(), H256::from_low_u64_be(slot));
    assert_eq!(get(0), U256::from(2), "events_count");
    assert_eq!(get(1), address_word(contract_addr()), "event0 address");
    assert_eq!(get(2), U256::zero(), "event0 topic_count");
    assert_eq!(get(3), U256::from(32), "event0 data_len");
    assert_eq!(get(4), address_word(contract_addr()), "event1 address");
    assert_eq!(get(5), U256::from(2), "event1 topic_count");
    assert_eq!(get(6), topic0, "event1 topic0");
    assert_eq!(get(7), topic1, "event1 topic1");
    assert_eq!(get(8), U256::from(32), "event1 data_len");
}

/// Helper: TXTRACE(param, index) and SSTORE the result at `slot`.
fn store_param_idx(param: u8, index: u64, slot: u64) -> Vec<u8> {
    let mut c = txtrace_idx(param, U256::from(index));
    c.extend(push32(U256::from(slot)));
    c.push(SSTORE);
    c
}

#[test]
fn topic0_access_on_log0_halts() {
    // LOG0 has zero topics. Reading topic0 (param 0x0F) at its index must halt.
    let mut code = log0(0, 0); // event 0: zero topics
    code.extend(txtrace_return_code(0x0F, 0x00)); // topic0 @ event 0 -> halt
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .run();
    assert!(
        !report.is_success(),
        "reading topic0 of a zero-topic LOG0 must halt"
    );
}

#[test]
fn topic_index_beyond_count_halts() {
    // LOG1 has one topic (index 0). Reading topic1 (param 0x10) must halt.
    let mut code = log1(0, 0, U256::from(0x99u64));
    code.extend(txtrace_return_code(0x10, 0x00)); // topic1 @ event 0 -> halt
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .run();
    assert!(!report.is_success(), "topic index >= topic_count must halt");
}

#[test]
fn emission_order_preserved_across_subcall() {
    // Contract emits LOG1(topic0=0xA), CALLs OTHER (which emits LOG1(topic0=0xB)),
    // then emits LOG1(topic0=0xC). events_count == 3 and topic0 of events 0/1/2
    // is A/B/C in emission order.
    let other_code = log1(0, 0, U256::from(0xB));
    let other_acct = contract_acct(
        {
            let mut c = other_code;
            c.push(STOP);
            c
        },
        U256::zero(),
    );

    let mut code = log1(0, 0, U256::from(0xA));
    code.extend(call(other_addr()));
    code.extend(log1(0, 0, U256::from(0xC)));
    // events_count @0; topic0 of events 0/1/2 @ slots 1/2/3.
    code.extend({
        let mut c = txtrace(0x0C, 0x00);
        c.extend(push1(0));
        c.push(SSTORE);
        c
    });
    code.extend(store_param_idx(0x0F, 0, 1));
    code.extend(store_param_idx(0x0F, 1, 2));
    code.extend(store_param_idx(0x0F, 2, 3));
    code.push(STOP);

    let (report, db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .account(other_addr(), other_acct)
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    let get = |slot: u64| storage_slot(&db, contract_addr(), H256::from_low_u64_be(slot));
    assert_eq!(get(0), U256::from(3), "events_count across subcall");
    assert_eq!(get(1), U256::from(0xA), "event0 topic0");
    assert_eq!(get(2), U256::from(0xB), "event1 topic0 (from subcall)");
    assert_eq!(get(3), U256::from(0xC), "event2 topic0");
}

#[test]
fn reverted_subcall_log_excluded() {
    // OTHER emits a log then REVERTs. The caller emits one log, CALLs OTHER
    // (reverts), then reads events_count: only the caller's log survives => 1.
    let other_code = {
        let mut c = log1(0, 0, U256::from(0xBAD));
        c.extend(push1(0));
        c.extend(push1(0));
        c.push(0xfd); // REVERT
        c
    };
    let mut code = log1(0, 0, U256::from(0x600D));
    code.extend(call(other_addr())); // call reverts; POP swallows the 0 flag
    code.extend(txtrace_return_code(0x0C, 0x00));
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .account(other_addr(), contract_acct(other_code, U256::zero()))
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    assert_eq!(
        output_word(&report),
        U256::one(),
        "a log emitted in a reverted subcall must be excluded"
    );
}

// ==================== EVENTDATACOPY ====================

/// EVENTDATACOPY(event_index, mem_offset, data_offset, length): push reverse
/// (length, dataOffset, memOffset, event_index).
fn eventdatacopy(event_index: u8, mem_offset: u8, data_offset: u8, length: u8) -> Vec<u8> {
    let mut out = push1(length);
    out.extend(push1(data_offset));
    out.extend(push1(mem_offset));
    out.extend(push1(event_index));
    out.push(EVENTDATACOPY);
    out
}

#[test]
fn eventdatacopy_copies_exact_bytes() {
    // Emit LOG0 with a known 32-byte data word, then EVENTDATACOPY all 32 bytes
    // of event 0 into memory offset 0x80, MLOAD it and RETURN to verify.
    let data_word = U256::from_big_endian(&[
        0xDE, 0xAD, 0xBE, 0xEF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0xCA, 0xFE,
    ]);
    let mut code = mstore_word(data_word);
    code.extend(log0(0, 32)); // event 0 data = data_word
    code.extend(eventdatacopy(0, 0x80, 0, 32)); // copy 32 bytes -> mem[0x80]
    // MLOAD mem[0x80] and RETURN it.
    code.extend(push1(0x80));
    code.push(MLOAD);
    code.extend(push1(0)); // mem offset for MSTORE
    code.push(MSTORE);
    code.extend(push1(32));
    code.extend(push1(0));
    code.push(RETURN);
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    assert_eq!(
        output_word(&report),
        data_word,
        "EVENTDATACOPY must copy the event data verbatim"
    );
}

#[test]
fn eventdatacopy_event_index_oob_halts_even_with_zero_length() {
    // No events emitted. EVENTDATACOPY(event_index = 0, length = 0) must still
    // halt because event_index is validated before the length-0 short-circuit.
    let mut code = eventdatacopy(0, 0, 0, 0);
    code.push(STOP);
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .run();
    assert!(
        !report.is_success(),
        "event_index >= events_count must halt even with length 0"
    );
}

#[test]
fn eventdatacopy_data_offset_plus_length_overflow_halts() {
    // Event 0 has 32 data bytes. Copying length 32 from data_offset 16
    // (16+32 > 32) must halt: no zero-fill past the end.
    let mut code = mstore_word(U256::from(0x1234u64));
    code.extend(log0(0, 32));
    code.extend(eventdatacopy(0, 0, 16, 32));
    code.push(STOP);
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .run();
    assert!(
        !report.is_success(),
        "data_offset + length > data_len must halt"
    );
}

#[test]
fn eventdatacopy_zero_length_is_noop() {
    // Event 0 exists; EVENTDATACOPY length 0 after a valid event_index succeeds
    // (no-op). The contract then RETURNs a sentinel to prove it ran to the end.
    let mut code = log0(0, 0); // event 0 with empty data
    code.extend(eventdatacopy(0, 0, 0, 0)); // valid index, zero length -> no-op
    code.extend(mstore_word(U256::from(0x5AFEu64)));
    code.extend(push1(32));
    code.extend(push1(0));
    code.push(RETURN);
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .run();
    assert!(
        report.is_success(),
        "zero-length EVENTDATACOPY must succeed"
    );
    assert_eq!(output_word(&report), U256::from(0x5AFEu64));
}

// ==================== HALTS / VALIDATION ====================

#[test]
fn undefined_param_halts() {
    // 0x16 is past the defined param range (0x00..=0x15) => halt.
    let code = txtrace_return_code(0x16, 0x00);
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .run();
    assert!(!report.is_success(), "undefined param must halt");
}

#[test]
fn nonzero_in2_on_scalar_param_halts() {
    // param 0x00 (balances_changed count) requires in2 == 0; in2 = 1 => halt.
    let code = txtrace_return_code(0x00, 0x01);
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .run();
    assert!(
        !report.is_success(),
        "nonzero in2 on a must-be-0 param must halt"
    );
}

#[test]
fn nonzero_in2_halts_for_all_scalar_params() {
    // Every scalar (must-be-0) param: 0x00 (balances count), 0x01 (slots count),
    // 0x02 (deployed count), 0x0C (events count), 0x14 (gas_pre_charge), 0x15
    // (gas_payer_address). Each with in2 = 1 must halt with InvalidOpcode.
    for param in [0x00u8, 0x01, 0x02, 0x0C, 0x14, 0x15] {
        let code = txtrace_return_code(param, 0x01);
        let (report, _db) = Harness::new()
            .account(contract_addr(), contract_acct(code, U256::zero()))
            .run();
        assert!(
            !report.is_success(),
            "nonzero in2 on must-be-0 param {param:#04x} must halt"
        );
    }
}

#[test]
fn operand_overflow_halts() {
    // An index operand exceeding u64 (here 2^200) cannot be converted by the
    // `u64::try_from` / `index_to_usize` path, so TXTRACE must halt with
    // InvalidOpcode rather than panic. param 0x03 (balance change_address) takes
    // an index in `in2`, so push 2^200 as the index.
    let huge = U256::one() << 200;
    let code = wrap_return(txtrace_idx(0x03, huge));
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .gas_price(0)
        .run();
    assert!(
        !report.is_success(),
        "an index operand exceeding u64 must halt, not panic"
    );
}

#[test]
fn index_out_of_bounds_halts() {
    // No balance changes can be indexed at a huge index: param 0x03 with a giant
    // in2 must halt (idx >= count).
    let code = wrap_return(txtrace_idx(0x03, U256::from(1000u64)));
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .gas_price(0)
        .run();
    assert!(!report.is_success(), "index >= count must halt");
}

// ==================== WALLET ASSERTION SUFFIX ====================

// Wallet injects a trailing CALL to an assertion contract that reads the tx's
// slot_changes count via TXTRACE and reverts if it exceeds the expected count.
// Models the hidden-approval-drain PoC.

/// Assertion contract: REVERT if slot_changes count != calldata[0..32].
fn assertion_suffix_code() -> Vec<u8> {
    let mut code = Vec::new();
    code.extend(push1(0));
    code.push(0x35); // CALLDATALOAD
    code.extend(txtrace(0x01, 0x00));
    code.push(EQ);
    code.push(0x15); // ISZERO
    code.extend(push1(0));
    let revert_dest_idx = code.len() - 1;
    code.push(JUMPI);
    code.push(STOP);
    let revert_dest = code.len();
    code[revert_dest_idx] = revert_dest as u8;
    code.push(JUMPDEST);
    code.extend(push1(0));
    code.extend(push1(0));
    code.push(REVERT);
    code
}

#[test]
fn assertion_suffix_passes_when_no_hidden_write() {
    // Target writes one slot, CALLs assertion with expected count == 1. Live
    // count == 1, assertion passes.
    let assertion_addr = other_addr();
    let target_code = {
        let mut c = sstore(0x01, U256::from(42));
        c.extend(mstore_word(U256::one()));
        c.extend(call_assertion(assertion_addr));
        c
    };

    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(target_code, U256::zero()))
        .account(assertion_addr, contract_acct(assertion_suffix_code(), U256::zero()))
        .run();
    assert!(
        report.is_success(),
        "tx should succeed when no hidden write: {:?}",
        report.result
    );
}

#[test]
fn assertion_suffix_reverts_on_hidden_slot_write() {
    // Target writes one slot + a hidden slot (simulating approve(MAX_UINT)),
    // CALLs assertion with expected count == 1. Live count == 2, assertion
    // reverts, tx fails.
    let assertion_addr = other_addr();
    let target_code = {
        let mut c = sstore(0x01, U256::from(42));
        c.extend(sstore(0x02, U256::MAX)); // hidden approval
        c.extend(mstore_word(U256::one()));
        c.extend(call_assertion(assertion_addr));
        c
    };

    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(target_code, U256::zero()))
        .account(assertion_addr, contract_acct(assertion_suffix_code(), U256::zero()))
        .run();
    assert!(
        !report.is_success(),
        "tx must fail when assertion detects a hidden slot write: {:?}",
        report.result
    );
}

// ==================== GAS PAYER ====================

#[test]
fn gas_payer_is_origin_for_normal_tx() {
    // param 0x15 (gas_payer_address) == the tx origin (sender) for a normal tx.
    let code = txtrace_return_code(0x15, 0x00);
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    assert_eq!(
        output_word(&report),
        address_word(sender_addr()),
        "gas_payer must be the origin for a normal tx"
    );
}

#[test]
fn gas_pre_charge_equals_gas_limit_times_price() {
    // param 0x14 (gas_pre_charge) == gas_limit * gas_price (no blobs).
    let code = txtrace_return_code(0x14, 0x00);
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    assert_eq!(
        output_word(&report),
        U256::from(GAS_LIMIT) * U256::from(GAS_PRICE),
        "gas_pre_charge must equal gas_limit * gas_price"
    );
}

#[test]
fn gas_precharge_includes_blob_fee() {
    // The normal-tx gas_pre_charge (param 0x14) includes the blob term:
    //   gas_limit * gas_price + blob_count * BLOB_GAS_PER_BLOB * base_blob_fee.
    // We give the env ONE blob hash and a non-zero base_blob_fee_per_gas, then
    // assert the returned pre-charge equals the full formula. The tx remains a
    // normal EIP-1559 transfer (`tx_max_fee_per_blob_gas` is None), so 4844
    // validation does not run.
    let base_blob_fee = U256::from(7u64);
    let code = txtrace_return_code(0x14, 0x00);
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .blob_env(vec![H256::repeat_byte(1)], base_blob_fee)
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    let blob_term =
        U256::from(1u64) * U256::from(ethrex_levm::gas_cost::BLOB_GAS_PER_BLOB) * base_blob_fee;
    let expected = U256::from(GAS_LIMIT) * U256::from(GAS_PRICE) + blob_term;
    assert_eq!(
        output_word(&report),
        expected,
        "gas_pre_charge must include the blob fee: \
         gas_limit*price + 1*BLOB_GAS_PER_BLOB*base_blob_fee"
    );
}

#[test]
fn gas_payer_present_in_balances_and_delta_equals_precharge() {
    // EIP headline guarantee: with gas_price > 0 and no other transfer, the
    // sender is the only balance change, its index-0 address is the sender, and
    // (balance_before - balance_after) == gas_pre_charge.
    //
    // The contract stores: balances count @0, address[0] @1, before[0] @2,
    // after[0] @3, gas_pre_charge @4.
    let mut code = Vec::new();
    code.extend({
        let mut c = txtrace(0x00, 0x00); // count
        c.extend(push1(0));
        c.push(SSTORE);
        c
    });
    code.extend(store_param_idx(0x03, 0, 1)); // address[0]
    code.extend(store_param_idx(0x04, 0, 2)); // before[0]
    code.extend(store_param_idx(0x05, 0, 3)); // after[0]
    code.extend({
        let mut c = txtrace(0x14, 0x00); // gas_pre_charge
        c.extend(push1(4));
        c.push(SSTORE);
        c
    });
    code.push(STOP);

    // Run with the contract being the tx target; the SENDER is the gas payer and
    // the ONLY account whose balance changes (no value transfer). The contract
    // does not move value, so the only balance diff is the sender's pre-charge.
    let (report, db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    let get = |slot: u64| storage_slot(&db, contract_addr(), H256::from_low_u64_be(slot));
    assert_eq!(get(0), U256::one(), "only the gas payer changed balance");
    assert_eq!(
        get(1),
        address_word(sender_addr()),
        "balance change is the sender"
    );
    let before = get(2);
    let after = get(3);
    let pre_charge = get(4);
    assert_eq!(
        before - after,
        pre_charge,
        "payer balance delta must equal the gas pre-charge"
    );
    assert_eq!(
        pre_charge,
        U256::from(GAS_LIMIT) * U256::from(GAS_PRICE),
        "gas_pre_charge must equal gas_limit * gas_price"
    );
}

// ==================== CONTEXT ====================

#[test]
fn txtrace_works_inside_staticcall() {
    // The caller STATICCALLs OTHER, which runs TXTRACE(0x15) (gas payer) and
    // RETURNs it. TXTRACE only reads/pushes, so it is legal under STATICCALL.
    // The caller MLOADs the returned word and RETURNs it.
    let other_code = txtrace_return_code(0x15, 0x00);
    // STATICCALL OTHER capturing 32 return bytes into mem[0], then RETURN them.
    let mut code = Vec::new();
    code.extend(push1(32)); // retSize
    code.extend(push1(0)); // retOffset
    code.extend(push1(0)); // argsSize
    code.extend(push1(0)); // argsOffset
    code.extend(push20(other_addr())); // address
    code.push(GAS);
    code.push(STATICCALL);
    // Assert success: if STATICCALL failed (0), MLOAD(0) would be 0; we instead
    // RETURN the retOffset buffer (mem[0..32]) which holds OTHER's TXTRACE word.
    code.push(POP);
    code.extend(push1(32));
    code.extend(push1(0));
    code.push(RETURN);

    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .account(other_addr(), contract_acct(other_code, U256::zero()))
        .run();
    assert!(
        report.is_success(),
        "TXTRACE under STATICCALL must succeed: {:?}",
        report.result
    );
    assert_eq!(
        output_word(&report),
        address_word(sender_addr()),
        "TXTRACE in a STATICCALL must observe the origin as gas payer"
    );
}

#[test]
fn nested_call_observes_committed_sstore() {
    // The caller SSTOREs a slot (committed), then CALLs OTHER which reads the
    // whole-tx slot_changes count and RETURNs it. The nested frame must observe
    // the caller's already-committed write => count >= 1.
    let other_code = txtrace_return_code(0x01, 0x00); // slots_changed count
    let mut code = sstore(0x07, U256::from(55)); // committed write by the caller
    // STATICCALL OTHER capturing 32 return bytes into mem[0], then RETURN them.
    code.extend(push1(32)); // retSize
    code.extend(push1(0)); // retOffset
    code.extend(push1(0)); // argsSize
    code.extend(push1(0)); // argsOffset
    code.extend(push20(other_addr()));
    code.push(GAS);
    code.push(STATICCALL);
    code.push(POP);
    code.extend(push1(32));
    code.extend(push1(0));
    code.push(RETURN);

    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .account(other_addr(), contract_acct(other_code, U256::zero()))
        .gas_price(0)
        .run();
    assert!(
        report.is_success(),
        "tx should succeed: {:?}",
        report.result
    );
    assert_eq!(
        output_word(&report),
        U256::one(),
        "nested frame must observe the caller's committed SSTORE"
    );
}

// ==================== FORK GATING ====================

#[test]
fn txtrace_invalid_before_hegota() {
    // The same TXTRACE bytecode at Amsterdam (pre-Hegota) must halt: the opcode
    // is not installed in the pre-Hegota table.
    let code = txtrace_return_code(0x15, 0x00);
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .fork(Fork::Amsterdam)
        .run();
    assert!(
        !report.is_success(),
        "TXTRACE must be invalid before Hegota"
    );
}

#[test]
fn eventdatacopy_invalid_before_hegota() {
    let mut code = log0(0, 0);
    code.extend(eventdatacopy(0, 0, 0, 0));
    code.push(STOP);
    let (report, _db) = Harness::new()
        .account(contract_addr(), contract_acct(code, U256::zero()))
        .fork(Fork::Amsterdam)
        .run();
    assert!(
        !report.is_success(),
        "EVENTDATACOPY must be invalid before Hegota"
    );
}

// ==================== FRAME-TX GAS PRECHARGE / PAYER ====================
//
// TXTRACE in a frame transaction: gas_payer (param 0x15) and gas_pre_charge
// (param 0x14) must resolve through `frame_tx_context`, not the normal-tx env
// path. This module builds a minimal frame-tx harness mirroring
// `eip8141_tests.rs` (`run_frame_tx_with_fees`, `frame_tx_with_frames`,
// `verify_frame`, `APPROVE_PAYMENT_CODE`).

mod frame_tx {
    use super::{TXTRACE, sender_addr};
    use bytes::Bytes;
    use ethrex_blockchain::vm::StoreVmDatabase;
    use ethrex_common::types::{
        Account, BlockHeader, Code, Fork, Frame, FrameMode, FrameTransaction, Transaction,
    };
    use ethrex_common::{Address, H256, U256, constants::EMPTY_TRIE_HASH};
    use ethrex_crypto::NativeCrypto;
    use ethrex_levm::db::gen_db::GeneralizedDatabase;
    use ethrex_levm::environment::{EVMConfig, Environment};
    use ethrex_levm::errors::{ExecutionReport, VMError};
    use ethrex_levm::tracing::LevmCallTracer;
    use ethrex_levm::vm::{VM, VMType};
    use ethrex_storage::Store;
    use ethrex_vm::DynVmDatabase;
    use rustc_hash::FxHashMap;
    use std::sync::Arc;

    const HARNESS_CHAIN_ID: u64 = 1;
    /// Funded sender for frame txs. Must be non-zero (static-constraint check).
    const FUNDED_SENDER: Address = Address::repeat_byte(0xAA);
    const AUTO_SEED_SENDER_BALANCE: U256 = U256::MAX;
    const HARNESS_BASE_FEE: u64 = 1;

    /// APPROVE(scope=1) -- payment approval. The frame's target becomes the payer.
    const APPROVE_PAYMENT_CODE: &[u8] = &[0x60, 0x01, 0x60, 0x00, 0x60, 0x00, 0xAA];
    /// APPROVE(scope=2) -- sender (execution) approval. Must run in a frame whose
    /// target IS the tx sender.
    const APPROVE_EXECUTION_CODE: &[u8] = &[0x60, 0x02, 0x60, 0x00, 0x60, 0x00, 0xAA];

    /// (address, balance, nonce, code).
    type SeededAccount = (Address, U256, u64, Bytes);

    fn seeded_db(accounts: &[SeededAccount]) -> GeneralizedDatabase {
        let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, header).unwrap());

        let mut cache: FxHashMap<Address, Account> = FxHashMap::default();
        for (address, balance, nonce, code) in accounts {
            cache.insert(
                *address,
                Account::new(
                    *balance,
                    Code::from_bytecode(code.clone(), &NativeCrypto),
                    *nonce,
                    FxHashMap::default(),
                ),
            );
        }
        GeneralizedDatabase::new_with_account_state(Arc::new(store), cache)
    }

    fn frame_tx_env(tx: &FrameTransaction) -> Environment {
        Environment {
            origin: tx.sender,
            gas_limit: tx.total_gas_limit(),
            block_gas_limit: (i64::MAX - 1) as u64,
            config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
            chain_id: U256::from(HARNESS_CHAIN_ID),
            base_fee_per_gas: U256::from(HARNESS_BASE_FEE),
            gas_price: U256::from(tx.max_fee_per_gas),
            tx_nonce: tx.nonce,
            ..Default::default()
        }
    }

    fn frame_tx_with_frames(frames: Vec<Frame>) -> FrameTransaction {
        FrameTransaction {
            chain_id: HARNESS_CHAIN_ID,
            nonce: 0,
            sender: FUNDED_SENDER,
            frames,
            signatures: Vec::new(),
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: HARNESS_BASE_FEE + 1_000,
            max_fee_per_blob_gas: U256::zero(),
            blob_versioned_hashes: Vec::new(),
            inner_hash: Default::default(),
            cached_canonical: Default::default(),
        }
    }

    /// Run `tx` with the env built around `base_fee`; the effective gas price is
    /// derived exactly like production (`min(base + priority, max_fee)`). Returns
    /// the result and post-execution db.
    fn run_frame_tx_with_fees(
        accounts: &[SeededAccount],
        tx: FrameTransaction,
        base_fee: u64,
    ) -> (Result<ExecutionReport, VMError>, GeneralizedDatabase) {
        let mut seeded: Vec<SeededAccount> = accounts.to_vec();
        if !seeded.iter().any(|(addr, ..)| *addr == tx.sender) {
            seeded.push((tx.sender, AUTO_SEED_SENDER_BALANCE, tx.nonce, Bytes::new()));
        }

        let mut db = seeded_db(&seeded);
        let mut env = frame_tx_env(&tx);
        env.base_fee_per_gas = U256::from(base_fee);
        let effective = base_fee
            .saturating_add(tx.max_priority_fee_per_gas)
            .min(tx.max_fee_per_gas);
        env.gas_price = U256::from(effective);
        let transaction = Transaction::FrameTransaction(tx);

        let result = {
            let mut vm = VM::new(
                env,
                &mut db,
                &transaction,
                LevmCallTracer::disabled(),
                VMType::L1,
                &NativeCrypto,
            )
            .expect("VM::new should succeed for a frame tx");
            vm.execute()
        };
        (result, db)
    }

    /// A VERIFY frame targeting `target` (gas_limit 100_000, flags 0x03 so the
    /// target's APPROVE code may grant payment/execution).
    fn verify_frame(target: Address) -> Frame {
        Frame {
            mode: u8::from(FrameMode::Verify),
            flags: 0x03,
            target: Some(target),
            gas_limit: 100_000,
            value: U256::zero(),
            data: Bytes::new(),
        }
    }

    fn storage_slot(db: &GeneralizedDatabase, addr: Address, key: H256) -> U256 {
        db.current_accounts_state
            .get(&addr)
            .and_then(|acc| acc.storage.get(&key).copied())
            .unwrap_or_default()
    }

    /// The 12-zero-prefixed address word that `address_to_u256` produces.
    fn address_word(addr: Address) -> U256 {
        let mut buf = [0u8; 32];
        buf[12..].copy_from_slice(addr.as_bytes());
        U256::from_big_endian(&buf)
    }

    /// TXTRACE(param=0x15) SSTORE@0 ; TXTRACE(param=0x14) SSTORE@1 ; STOP.
    /// Reads gas_payer_address and gas_pre_charge and surfaces them via storage.
    /// Built from raw opcodes: PUSH1 param; PUSH1 0 (in2); TXTRACE; PUSH1 slot; SSTORE.
    fn read_payer_and_precharge_code() -> Vec<u8> {
        const PUSH1: u8 = 0x60;
        const SSTORE: u8 = 0x55;
        const STOP: u8 = 0x00;
        let mut c = Vec::new();
        // gas_payer_address (0x15) -> slot 0
        c.extend_from_slice(&[PUSH1, 0x15, PUSH1, 0x00, TXTRACE, PUSH1, 0x00, SSTORE]);
        // gas_pre_charge (0x14) -> slot 1
        c.extend_from_slice(&[PUSH1, 0x14, PUSH1, 0x00, TXTRACE, PUSH1, 0x01, SSTORE]);
        c.push(STOP);
        c
    }

    #[test]
    fn frame_tx_gas_precharge_and_payer() {
        // Frame ordering (mirrors eip8141's pay-before-verify pattern):
        //   frame0: VERIFY -> paymaster runs APPROVE_PAYMENT (scope 1) => payer = paymaster.
        //   frame1: VERIFY -> FUNDED_SENDER runs APPROVE_EXECUTION (scope 2) => sender_approved.
        //   frame2: DEFAULT -> reader contract runs TXTRACE 0x15 / 0x14 and SSTOREs them.
        // The reader frame runs AFTER APPROVE_PAYMENT, so the live
        // frame_tx_context already carries payer_address = paymaster.
        let paymaster = Address::from_low_u64_be(0x9A);
        let reader = Address::from_low_u64_be(0x9B);

        // Sanity: the harness sender is FUNDED_SENDER, distinct from the paymaster,
        // so asserting gas_payer == paymaster (not sender) is meaningful.
        assert_ne!(FUNDED_SENDER, paymaster);
        assert_ne!(FUNDED_SENDER, sender_addr());

        let base_fee: u64 = 10;
        let mut tx = frame_tx_with_frames(vec![
            verify_frame(paymaster),
            verify_frame(FUNDED_SENDER),
            Frame {
                mode: u8::from(FrameMode::Default),
                flags: 0x00,
                target: Some(reader),
                gas_limit: 300_000,
                value: U256::zero(),
                data: Bytes::new(),
            },
        ]);
        tx.max_fee_per_gas = 100;
        tx.max_priority_fee_per_gas = 2;

        // Effective price = min(base + priority, max_fee) = min(12, 100) = 12.
        let effective = base_fee
            .saturating_add(tx.max_priority_fee_per_gas)
            .min(tx.max_fee_per_gas);
        // Expected pre-charge = total_gas_limit * effective_price (no blobs, so
        // blob_gas_cost == 0). `total_gas_limit` is computed from the tx itself,
        // exactly as `compute_tx_cost` reads `ctx.total_gas_limit`.
        let expected_precharge = U256::from(tx.total_gas_limit()) * U256::from(effective);

        let accounts = [
            (
                paymaster,
                U256::from(10u64).pow(U256::from(18u64)),
                0,
                Bytes::from(APPROVE_PAYMENT_CODE.to_vec()),
            ),
            (
                FUNDED_SENDER,
                AUTO_SEED_SENDER_BALANCE,
                0,
                Bytes::from(APPROVE_EXECUTION_CODE.to_vec()),
            ),
            (
                reader,
                U256::zero(),
                0,
                Bytes::from(read_payer_and_precharge_code()),
            ),
        ];

        let (result, db) = run_frame_tx_with_fees(&accounts, tx, base_fee);
        let report = result.expect("valid frame tx: payer approved, sender approved");
        assert_eq!(
            report.payer_address,
            Some(paymaster),
            "paymaster must be the payer"
        );

        // gas_payer_address (0x15) == the APPROVE_PAYMENT target (paymaster),
        // NOT the tx sender.
        let payer_word = storage_slot(&db, reader, H256::zero());
        assert_eq!(
            payer_word,
            address_word(paymaster),
            "TXTRACE gas_payer must be the paymaster (APPROVE_PAYMENT target), not the sender"
        );
        assert_ne!(
            payer_word,
            address_word(FUNDED_SENDER),
            "gas_payer must NOT be the tx sender when a paymaster approved payment"
        );

        // gas_pre_charge (0x14) == total_gas_limit * effective_price (no blobs).
        let precharge = storage_slot(&db, reader, H256::from_low_u64_be(1));
        assert_eq!(
            precharge, expected_precharge,
            "TXTRACE gas_pre_charge must equal total_gas_limit * effective_price"
        );
    }
}
