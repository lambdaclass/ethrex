//! EIP-7928: the coinbase belongs in the BAL of every user transaction.
//!
//! The spec reads the coinbase while disbursing a user transaction's fees, even when the
//! transaction pays nothing to it. A transaction with gas price zero (possible on a chain whose
//! base fee is zero, with no tip) used to leave the coinbase out of the BAL, because the record
//! was gated on a non-zero gas price instead of on the transaction not being a system call.
//! A system call never touches the coinbase, so the coinbase stays out of its BAL.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::{EMPTY_TRIE_HASH, SYSTEM_ADDRESS},
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
    hooks::default_hook::pay_coinbase,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

const GAS_LIMIT: u64 = 1_000_000;

struct TestDb;

impl Database for TestDb {
    fn get_account_state(&self, _address: Address) -> Result<AccountState, DatabaseError> {
        Ok(AccountState {
            storage_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        })
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

/// Runs `run` on a VM whose transaction calls an existing account, and returns the addresses in
/// the resulting BAL. A user transaction is a value transfer from a funded EOA at the given gas
/// price (base fee equal to it, no tip). A system call is set up the way
/// `generic_system_contract_levm` sets one up: from the system address, with no value, gas price
/// zero and the BAL recorder in system-call mode.
fn bal_addresses(
    gas_price: u64,
    is_system_call: bool,
    run: impl FnOnce(&mut VM<'_>),
) -> Vec<Address> {
    let sender = Address::from_low_u64_be(0x1000);
    let recipient = Address::from_low_u64_be(0x2000);
    let coinbase = Address::from_low_u64_be(0xCCC);

    // The recipient already exists, so the transfer creates no account.
    let accounts: FxHashMap<Address, Account> = [
        (
            sender,
            Account::new(
                U256::from(10_000_000_000u64),
                Code::default(),
                0,
                FxHashMap::default(),
            ),
        ),
        (
            recipient,
            Account::new(U256::from(1), Code::default(), 0, FxHashMap::default()),
        ),
    ]
    .into_iter()
    .collect();
    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(TestDb), accounts);
    db.enable_bal_recording();
    db.set_bal_index(1);
    if is_system_call && let Some(recorder) = db.bal_recorder.as_mut() {
        recorder.enter_system_call();
    }

    let (origin, value) = if is_system_call {
        (SYSTEM_ADDRESS, U256::zero())
    } else {
        (sender, U256::from(1))
    };
    let fork = Fork::Amsterdam;
    let env = Environment {
        origin,
        gas_limit: GAS_LIMIT,
        config: EVMConfig::new(fork, EVMConfig::canonical_values(fork)),
        block_number: 1,
        coinbase,
        timestamp: 1000,
        prev_randao: Some(H256::zero()),
        difficulty: U256::zero(),
        slot_number: U256::zero(),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::from(gas_price),
        base_blob_fee_per_gas: U256::from(1),
        gas_price: U256::from(gas_price),
        block_excess_blob_gas: None,
        block_blob_gas_used: None,
        tx_blob_hashes: vec![],
        tx_max_priority_fee_per_gas: Some(U256::zero()),
        tx_max_fee_per_gas: Some(U256::from(gas_price)),
        tx_max_fee_per_blob_gas: None,
        tx_nonce: 0,
        block_gas_limit: GAS_LIMIT * 2,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: false,
        disable_nonce_check: false,
        disable_gas_allowance_check: false,
        disable_sender_eoa_check: false,
        disable_execution_gas_cap: false,
        is_system_call,
    };
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(recipient),
        value,
        data: Bytes::new(),
        gas_limit: GAS_LIMIT,
        max_fee_per_gas: gas_price,
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
        None,
    )
    .expect("the VM accepts the transaction");
    run(&mut vm);

    db.take_bal()
        .expect("BAL recording was enabled")
        .accounts()
        .iter()
        .map(|account| account.address)
        .collect()
}

fn execute_successfully(vm: &mut VM<'_>) {
    let report = vm.execute().expect("the transaction executes");
    assert!(
        report.is_success(),
        "the transaction must succeed: {:?}",
        report.result
    );
}

#[test]
fn coinbase_is_in_the_bal_of_a_zero_gas_price_transaction() {
    let coinbase = Address::from_low_u64_be(0xCCC);
    let addresses = bal_addresses(0, false, execute_successfully);
    assert!(
        addresses.contains(&coinbase),
        "a user transaction with gas price zero must still record the coinbase, got {addresses:?}"
    );
}

#[test]
fn coinbase_is_in_the_bal_of_a_paying_transaction() {
    let coinbase = Address::from_low_u64_be(0xCCC);
    let addresses = bal_addresses(1000, false, execute_successfully);
    assert!(
        addresses.contains(&coinbase),
        "a user transaction paying fees must record the coinbase, got {addresses:?}"
    );
}

#[test]
fn coinbase_is_not_in_the_bal_of_a_system_call() {
    let coinbase = Address::from_low_u64_be(0xCCC);
    // finalize_execution returns before paying the coinbase for a system call, so pay_coinbase
    // is also called directly to cover its own system-call check.
    let addresses = bal_addresses(0, true, |vm| {
        execute_successfully(vm);
        pay_coinbase(vm, 0).expect("paying the coinbase succeeds");
    });
    assert!(
        !addresses.contains(&coinbase),
        "a system call must not record the coinbase, got {addresses:?}"
    );
}
