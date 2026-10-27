//! EIP-8298 `SETCODEFROM`: what the execution leaves for the rest of the client to
//! consume, the block access list entry, the state transition and the output of a
//! creation that adopted its code, plus the system-contract halt.

use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::BEACON_ROOTS_ADDRESS,
    evm::calculate_create_address,
    types::{
        Account, Code, EIP1559Transaction, Fork, Transaction, TxKind,
        block_access_list::{AccountChanges, CodeChange},
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
use rustc_hash::FxHashMap;

use super::test_db::TestDatabase;

const GAS_LIMIT: u64 = 1_000_000;

const SENDER: Address = Address::repeat_byte(0x10);
const ADOPTER: Address = Address::repeat_byte(0x20);
const TEMPLATE: Address = Address::repeat_byte(0x30);
const CALLER: Address = Address::repeat_byte(0x40);

/// `SSTORE(1, 0x42)`.
const TEMPLATE_CODE: &[u8] = &[0x60, 0x42, 0x60, 0x01, 0x55, 0x00];

fn push20(address: Address) -> Vec<u8> {
    let mut code = vec![0x73];
    code.extend_from_slice(address.as_bytes());
    code
}

/// `SSTORE(0, SETCODEFROM(source))`.
fn adopt_and_store(source: Address) -> Bytes {
    let mut code = push20(source);
    code.extend_from_slice(&[0xA7, 0x60, 0x00, 0x55, 0x00]);
    Bytes::from(code)
}

fn contract(code: Bytes) -> Account {
    Account::new(
        U256::zero(),
        Code::from_bytecode(code, &NativeCrypto),
        1,
        FxHashMap::default(),
    )
}

fn database(accounts: Vec<(Address, Account)>) -> GeneralizedDatabase {
    let mut accounts: FxHashMap<Address, Account> = accounts.into_iter().collect();
    accounts.insert(
        SENDER,
        Account::new(
            U256::from(10u64).pow(U256::from(18u64)),
            Code::default(),
            0,
            FxHashMap::default(),
        ),
    );
    accounts.insert(TEMPLATE, contract(Bytes::from_static(TEMPLATE_CODE)));
    let mut db =
        GeneralizedDatabase::new_with_account_state(Arc::new(TestDatabase::new()), accounts);
    db.enable_bal_recording();
    db.set_bal_index(1);
    db
}

fn execute(db: &mut GeneralizedDatabase, to: TxKind, data: Bytes) -> ExecutionReport {
    let fork = Fork::Hegota;
    let env = Environment {
        origin: SENDER,
        gas_limit: GAS_LIMIT,
        config: EVMConfig::new(fork, EVMConfig::canonical_values(fork)),
        block_number: 1,
        coinbase: Address::from_low_u64_be(0xCCC),
        timestamp: 1000,
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        slot_number: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::from(1000),
        base_blob_fee_per_gas: U256::from(1),
        gas_price: U256::from(1000),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: None,
        tx_max_fee_per_gas: Some(U256::from(1000)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: GAS_LIMIT * 2,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: false,
        disable_nonce_check: false,
        disable_gas_allowance_check: false,
        disable_sender_eoa_check: false,
        is_system_call: false,
    };
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to,
        data,
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: 1000,
        max_priority_fee_per_gas: 1,
        ..Default::default()
    });
    let mut vm = VM::new(
        env,
        db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .unwrap();
    vm.execute().unwrap()
}

fn code_changes(db: &mut GeneralizedDatabase, address: Address) -> Vec<CodeChange> {
    db.take_bal()
        .expect("BAL recording is enabled")
        .accounts()
        .iter()
        .find(|account| account.address == address)
        .map(|account| account.code_changes.clone())
        .unwrap_or_default()
}

/// The adopter's code change is recorded as the template's code hash, the template
/// is listed without changes, and the state transition carries the adopted code so
/// the store keeps it for the adopter.
#[test]
fn adoption_is_recorded_as_a_code_hash_and_carries_the_code() {
    let mut db = database(vec![(ADOPTER, contract(adopt_and_store(TEMPLATE)))]);
    let report = execute(&mut db, TxKind::Call(ADOPTER), Bytes::new());
    assert!(report.is_success(), "{:?}", report.result);

    let bal = db.take_bal().expect("BAL recording is enabled");
    let account = |address: Address| {
        bal.accounts()
            .iter()
            .find(|account| account.address == address)
            .cloned()
    };
    assert_eq!(
        account(ADOPTER).expect("adopter in BAL").code_changes,
        vec![CodeChange::adoption(1, keccak(TEMPLATE_CODE))]
    );
    assert_eq!(account(TEMPLATE), Some(AccountChanges::new(TEMPLATE)));

    let updates = db.get_state_transitions().unwrap();
    let adopter = updates
        .iter()
        .find(|update| update.address == ADOPTER)
        .expect("adopter updated");
    assert_eq!(
        adopter.info.as_ref().map(|info| info.code_hash),
        Some(keccak(TEMPLATE_CODE))
    );
    assert_eq!(
        adopter.code.as_ref().map(|code| code.code().to_vec()),
        Some(TEMPLATE_CODE.to_vec())
    );
    assert_eq!(adopter.added_storage.get(&H256::zero()), Some(&U256::one()));
}

/// Initcode that adopted code returns data that would fail code deposit (0xEF
/// prefix): the creation still succeeds, deploys the adopted code, and reports it as
/// its output.
#[test]
fn creation_that_adopted_deploys_the_adopted_code() {
    let mut initcode = push20(TEMPLATE);
    // POP(SETCODEFROM(TEMPLATE)), MSTORE8(0, 0xEF), RETURN(0, 1)
    initcode.extend_from_slice(&[0xA7, 0x50, 0x60, 0xEF, 0x60, 0x00, 0x53]);
    initcode.extend_from_slice(&[0x60, 0x01, 0x60, 0x00, 0xF3]);
    let created = calculate_create_address(SENDER, 0);

    let mut db = database(vec![]);
    let report = execute(&mut db, TxKind::Create, Bytes::from(initcode));
    assert!(report.is_success(), "{:?}", report.result);
    assert_eq!(report.output.as_ref(), TEMPLATE_CODE);
    assert_eq!(
        db.get_account(created).unwrap().info.code_hash,
        keccak(TEMPLATE_CODE)
    );
    assert_eq!(
        code_changes(&mut db, created),
        vec![CodeChange::adoption(1, keccak(TEMPLATE_CODE))]
    );
}

/// A system contract the block calls cannot replace its code, even when an ordinary
/// transaction calls it: the call halts.
#[test]
fn system_contract_cannot_adopt() {
    // SSTORE(0, CALL(GAS, BEACON_ROOTS_ADDRESS, 0, 0, 0, 0, 0))
    let mut caller_code = vec![0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00];
    caller_code.extend_from_slice(&push20(BEACON_ROOTS_ADDRESS));
    caller_code.extend_from_slice(&[0x5A, 0xF1, 0x60, 0x00, 0x55, 0x00]);
    let system_code = adopt_and_store(TEMPLATE);

    let mut db = database(vec![
        (CALLER, contract(Bytes::from(caller_code))),
        (BEACON_ROOTS_ADDRESS, contract(system_code.clone())),
    ]);
    let report = execute(&mut db, TxKind::Call(CALLER), Bytes::new());
    assert!(report.is_success(), "{:?}", report.result);

    let caller = db
        .current_accounts_state
        .get(&CALLER)
        .expect("caller loaded");
    assert_eq!(
        caller.storage.get(&H256::zero()),
        Some(&U256::zero()),
        "the call into the system contract must fail"
    );
    assert_eq!(
        db.get_account(BEACON_ROOTS_ADDRESS).unwrap().info.code_hash,
        keccak(&system_code)
    );
    assert!(code_changes(&mut db, BEACON_ROOTS_ADDRESS).is_empty());
}
