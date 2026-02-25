//! Shared test helpers for tokamak-debugger tests.
//!
//! Re-uses the same patterns as `tokamak-jit/src/tests/test_helpers.rs`.

use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::{
    Address, U256,
    constants::EMPTY_TRIE_HASH,
    types::{Account, BlockHeader, Code, EIP1559Transaction, Transaction, TxKind},
};
use ethrex_levm::{Environment, db::gen_db::GeneralizedDatabase};
use rustc_hash::FxHashMap;

/// Standard gas limit — large enough to avoid OOG in tests.
#[expect(clippy::as_conversions)]
pub const TEST_GAS_LIMIT: u64 = (i64::MAX - 1) as u64;

/// Standard contract address.
pub const CONTRACT_ADDR: u64 = 0x42;

/// Standard sender address.
pub const SENDER_ADDR: u64 = 0x100;

pub struct TestAccount {
    pub address: Address,
    pub code: Code,
}

/// Create an in-memory DB with pre-seeded accounts.
pub fn make_test_db(accounts: Vec<TestAccount>) -> GeneralizedDatabase {
    let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
        .expect("in-memory store");
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let vm_db: ethrex_vm::DynVmDatabase = Box::new(
        ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
    );

    let mut cache = FxHashMap::default();
    for acct in accounts {
        cache.insert(
            acct.address,
            Account::new(U256::MAX, acct.code, 0, FxHashMap::default()),
        );
    }

    GeneralizedDatabase::new_with_account_state(Arc::new(vm_db), cache)
}

/// Create a standard test environment.
pub fn make_test_env(sender: Address) -> Environment {
    Environment {
        origin: sender,
        gas_limit: TEST_GAS_LIMIT,
        block_gas_limit: TEST_GAS_LIMIT,
        ..Default::default()
    }
}

/// Create a standard EIP-1559 transaction calling a contract.
pub fn make_test_tx(contract: Address) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        data: Bytes::new(),
        ..Default::default()
    })
}

/// Build standard contract + sender accounts for a simple test.
pub fn setup_contract(bytecode: Vec<u8>) -> (Address, Address, GeneralizedDatabase) {
    let contract_addr = Address::from_low_u64_be(CONTRACT_ADDR);
    let sender_addr = Address::from_low_u64_be(SENDER_ADDR);

    let accounts = vec![
        TestAccount {
            address: contract_addr,
            code: Code::from_bytecode(Bytes::from(bytecode)),
        },
        TestAccount {
            address: sender_addr,
            code: Code::from_bytecode(Bytes::new()),
        },
    ];

    let db = make_test_db(accounts);
    (contract_addr, sender_addr, db)
}
