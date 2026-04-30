//! EIP-7976 calldata floor 64/64 + EIP-7981 access-list floor tests.
//!
//! EIP-7976 (Amsterdam+): raises `TOTAL_COST_FLOOR_PER_TOKEN` from 10 (EIP-7623) to 16,
//! yielding an effective floor of 64 gas per calldata byte for both zero and non-zero bytes
//! (since `16 * STANDARD_TOKEN_COST(4) = 64`).
//!
//! EIP-7981 (Amsterdam+): access-list data bytes fold into the floor-token count.
//! Each address entry contributes 20 bytes and each storage key contributes 32 bytes;
//! these are divided by `STANDARD_TOKEN_COST` (4) to convert to tokens before multiplying
//! by the floor rate.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        Account, AccountState, ChainConfig, Code, CodeMetadata, EIP1559Transaction, Fork,
        Transaction, TxKind,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    errors::DatabaseError,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ==================== Test Database ====================

struct TestDatabase;

impl Database for TestDatabase {
    fn get_account_state(&self, _address: Address) -> Result<AccountState, DatabaseError> {
        Ok(AccountState::default())
    }

    fn get_storage_value(&self, _address: Address, _key: H256) -> Result<U256, DatabaseError> {
        Ok(U256::zero())
    }

    fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
        Ok(H256::zero())
    }

    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        Ok(ChainConfig::default())
    }

    fn get_account_code(&self, _code_hash: H256) -> Result<Code, DatabaseError> {
        Ok(Code::default())
    }

    fn get_code_metadata(&self, _code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        Ok(CodeMetadata { length: 0 })
    }
}

// ==================== Helpers ====================

const SENDER: u64 = 0x1000;
const RECIPIENT: u64 = 0x2000;
// TX_BASE_COST = 21000, STANDARD_TOKEN_COST = 4
const TX_BASE_COST: u64 = 21_000;

fn sender_addr() -> Address {
    Address::from_low_u64_be(SENDER)
}

fn recipient_addr() -> Address {
    Address::from_low_u64_be(RECIPIENT)
}

fn make_db() -> GeneralizedDatabase {
    let mut accounts: FxHashMap<Address, Account> = FxHashMap::default();
    accounts.insert(
        sender_addr(),
        Account::new(
            U256::from(10_000_000_000u64),
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );
    GeneralizedDatabase::new_with_account_state(Arc::new(TestDatabase), accounts)
}

fn make_env(fork: Fork) -> Environment {
    let blob_schedule = EVMConfig::canonical_values(fork);
    Environment {
        origin: sender_addr(),
        gas_limit: 10_000_000,
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
        block_gas_limit: 30_000_000,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: true,
        is_system_call: false,
    }
}

/// Build an EIP-1559 transaction with the given calldata and access list.
fn make_tx(calldata: Bytes, access_list: Vec<(Address, Vec<H256>)>) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 10_000_000,
        to: TxKind::Call(recipient_addr()),
        value: U256::zero(),
        data: calldata,
        access_list,
        ..Default::default()
    })
}

/// Returns `get_min_gas_used()` for the given transaction and fork.
fn get_floor(fork: Fork, tx: &Transaction) -> u64 {
    let env = make_env(fork);
    let mut db = make_db();
    let vm = VM::new(
        env,
        &mut db,
        tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
    )
    .expect("VM::new failed");
    vm.get_min_gas_used().expect("get_min_gas_used failed")
}

// ==================== Tests ====================

/// Pre-Amsterdam regression: calldata floor at Prague and Cancun must be identical.
///
/// Input: 100 non-zero bytes + access list with 2 addresses and 3 storage keys.
/// Pre-Amsterdam uses TOTAL_COST_FLOOR_PER_TOKEN = 10 and ignores access-list bytes.
///
/// Arithmetic:
///   tokens_in_calldata = (100 * 16) / 4 = 400  [CALLDATA_COST_NON_ZERO_BYTE=16]
///   min_gas = TX_BASE_COST + 400 * 10 = 21000 + 4000 = 25000
#[test]
fn test_pre_amsterdam_floor_unchanged() {
    let calldata = Bytes::from(vec![0xAA; 100]); // 100 non-zero bytes
    let access_list = vec![
        (
            Address::from_low_u64_be(0xA1),
            vec![H256::zero(), H256::zero()],
        ),
        (Address::from_low_u64_be(0xA2), vec![H256::zero()]),
    ];
    let tx = make_tx(calldata, access_list);

    let floor_prague = get_floor(Fork::Prague, &tx);
    let floor_cancun = get_floor(Fork::Cancun, &tx);

    // tokens = 400, pre-Amsterdam floor rate = 10
    let expected = TX_BASE_COST + 400 * 10;
    assert_eq!(
        floor_prague, expected,
        "Prague floor mismatch: got {floor_prague}, expected {expected}"
    );
    assert_eq!(
        floor_cancun, floor_prague,
        "Prague and Cancun floors must be identical, got Prague={floor_prague} Cancun={floor_cancun}"
    );
}

/// EIP-7976 Amsterdam calldata floor: 1000 non-zero bytes.
///
/// Arithmetic (Amsterdam, TOTAL_COST_FLOOR_PER_TOKEN = 16):
///   tokens_in_calldata = (1000 * 16) / 4 = 4000
///   min_gas = TX_BASE_COST + 4000 * 16 = 21000 + 64000 = 85000
///
/// This yields an effective floor of 64 gas/byte (16 * 4 = 64).
#[test]
fn test_amsterdam_calldata_floor_64_per_byte() {
    let calldata = Bytes::from(vec![0xAA; 1000]); // 1000 non-zero bytes
    let tx = make_tx(calldata, vec![]);

    let floor = get_floor(Fork::Amsterdam, &tx);

    // tokens = 4000, Amsterdam floor rate = 16
    let expected = TX_BASE_COST + 4000 * 16;
    assert_eq!(
        floor, expected,
        "Amsterdam calldata floor: got {floor}, expected {expected} (64 gas/byte effective)"
    );
}

/// EIP-7981 Amsterdam access-list floor folding: 3 addresses, 5 storage keys, zero calldata.
///
/// Arithmetic:
///   access_list_bytes = 3 * 20 + 5 * 32 = 60 + 160 = 220
///   floor_tokens_in_access_list = 220 * 4 = 880  (EIP-7981: bytes * STANDARD_TOKEN_COST)
///   tokens_in_calldata (calldata = 0) = 0
///   total_tokens = 0 + 880 = 880
///   min_gas = TX_BASE_COST + 880 * 16 = 21000 + 14080 = 35080
///
/// Access-list *charge* (ACCESS_LIST_ADDRESS_COST / ACCESS_LIST_STORAGE_KEY_COST) is unchanged.
#[test]
fn test_amsterdam_access_list_floor_folding() {
    // 3 addresses: addr1 has 2 keys, addr2 has 2 keys, addr3 has 1 key → 5 keys total
    let access_list = vec![
        (
            Address::from_low_u64_be(0xA1),
            vec![H256::zero(), H256::zero()],
        ),
        (
            Address::from_low_u64_be(0xA2),
            vec![H256::zero(), H256::zero()],
        ),
        (Address::from_low_u64_be(0xA3), vec![H256::zero()]),
    ];
    let tx = make_tx(Bytes::new(), access_list);

    let floor = get_floor(Fork::Amsterdam, &tx);

    // 3 * 20 + 5 * 32 = 220 bytes → 220 * 4 = 880 tokens (EIP-7981: multiply, not divide)
    let expected = TX_BASE_COST + 880 * 16;
    assert_eq!(
        floor, expected,
        "Amsterdam access-list floor: got {floor}, expected {expected}"
    );
}

/// EIP-7976 + EIP-7981 combined: calldata + access list, no double-counting.
///
/// Input: 100 non-zero bytes calldata + 2 addresses + 3 storage keys.
///
/// Arithmetic:
///   floor_tokens_in_calldata = 100 * 4 = 400  (EIP-7976: unweighted, all bytes * STANDARD_TOKEN_COST)
///   access_list_bytes = 2 * 20 + 3 * 32 = 40 + 96 = 136
///   floor_tokens_in_access_list = 136 * 4 = 544  (EIP-7981: bytes * STANDARD_TOKEN_COST)
///   total_tokens = 400 + 544 = 944
///   min_gas = TX_BASE_COST + 944 * 16 = 21000 + 15104 = 36104
#[test]
fn test_amsterdam_combined_calldata_and_access_list() {
    let calldata = Bytes::from(vec![0xAA; 100]); // 100 non-zero bytes
    let access_list = vec![
        (
            Address::from_low_u64_be(0xB1),
            vec![H256::zero(), H256::zero()],
        ),
        (Address::from_low_u64_be(0xB2), vec![H256::zero()]),
    ];
    let tx = make_tx(calldata, access_list);

    let floor = get_floor(Fork::Amsterdam, &tx);

    // calldata floor tokens = 400, access_list floor tokens = 544, total = 944
    let expected = TX_BASE_COST + 944 * 16;
    assert_eq!(
        floor, expected,
        "Amsterdam combined floor: got {floor}, expected {expected}"
    );
}

/// Access-list with no storage keys: only address bytes count.
///
/// Arithmetic:
///   access_list_bytes = 2 * 20 + 0 * 32 = 40
///   floor_tokens_in_access_list = 40 * 4 = 160  (EIP-7981: bytes * STANDARD_TOKEN_COST)
///   min_gas = TX_BASE_COST + 160 * 16 = 21000 + 2560 = 23560
#[test]
fn test_amsterdam_access_list_addresses_only() {
    let access_list = vec![
        (Address::from_low_u64_be(0xC1), vec![]),
        (Address::from_low_u64_be(0xC2), vec![]),
    ];
    let tx = make_tx(Bytes::new(), access_list);

    let floor = get_floor(Fork::Amsterdam, &tx);

    // 2 * 20 = 40 bytes → 40 * 4 = 160 tokens (EIP-7981: multiply, not divide)
    let expected = TX_BASE_COST + 160 * 16;
    assert_eq!(
        floor, expected,
        "Amsterdam addresses-only floor: got {floor}, expected {expected}"
    );
}

/// EIP-7976 mixed zero/non-zero calldata: floor uses unweighted byte count.
///
/// Input: 500 zero bytes + 500 non-zero bytes = 1000 bytes total, Amsterdam, no access list.
///
/// Arithmetic (EIP-7976 floor arm, unweighted):
///   floor_tokens_in_calldata = 1000 * 4 = 4000
///   min_gas = TX_BASE_COST + 4000 * 16 = 21000 + 64000 = 85000
///
/// Under the wrong weighted formula it would be:
///   tokens = (500 * 16 + 500 * 4) / 4 = (8000 + 2000) / 4 = 2500
///   wrong_floor = 21000 + 2500 * 16 = 61000
/// This test specifically catches Bug 2 (weighted vs. unweighted).
#[test]
fn test_amsterdam_mixed_zero_nonzero_calldata_floor() {
    // 500 zero bytes followed by 500 non-zero bytes
    let mut data = vec![0u8; 500];
    data.extend(vec![0xAA; 500]);
    let calldata = Bytes::from(data);
    let tx = make_tx(calldata, vec![]);

    let floor = get_floor(Fork::Amsterdam, &tx);

    // EIP-7976 unweighted: 1000 bytes * 4 = 4000 tokens; floor = 21000 + 4000 * 16 = 85000
    let expected = TX_BASE_COST + 4000 * 16;
    assert_eq!(
        floor, expected,
        "Amsterdam mixed calldata floor (unweighted): got {floor}, expected {expected}"
    );
}

/// Pre-Amsterdam does NOT include access-list bytes in the floor.
///
/// Same input as test_amsterdam_access_list_floor_folding but at Prague.
/// Floor tokens = 0 (no calldata), floor rate = 10.
/// min_gas = TX_BASE_COST + 0 * 10 = 21000
#[test]
fn test_pre_amsterdam_access_list_not_in_floor() {
    let access_list = vec![
        (
            Address::from_low_u64_be(0xA1),
            vec![H256::zero(), H256::zero()],
        ),
        (
            Address::from_low_u64_be(0xA2),
            vec![H256::zero(), H256::zero()],
        ),
        (Address::from_low_u64_be(0xA3), vec![H256::zero()]),
    ];
    let tx = make_tx(Bytes::new(), access_list);

    let floor = get_floor(Fork::Prague, &tx);

    // Pre-Amsterdam: no access-list bytes in floor, no calldata → floor = TX_BASE_COST
    assert_eq!(
        floor, TX_BASE_COST,
        "Pre-Amsterdam floor must equal TX_BASE_COST when calldata is empty, got {floor}"
    );
}
