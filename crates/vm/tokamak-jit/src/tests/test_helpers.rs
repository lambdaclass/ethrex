//! Shared test helpers for tokamak-jit tests.
//!
//! Consolidates duplicate DB setup patterns (Volkov R24 — R1, R3, R4).

use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::{
    Address, U256,
    constants::EMPTY_TRIE_HASH,
    types::{Account, BlockHeader, Code, EIP1559Transaction, Transaction, TxKind},
};
use ethrex_levm::{Environment, db::gen_db::GeneralizedDatabase};
use rustc_hash::FxHashMap;

use ethrex_common::H256;

/// Intrinsic gas for a basic EIP-1559 CALL transaction (R3: magic number extraction).
pub const INTRINSIC_GAS: u64 = 21_000;

/// Standard contract address used across tests.
pub const CONTRACT_ADDR: u64 = 0x42;

/// Standard sender address used across tests.
pub const SENDER_ADDR: u64 = 0x100;

/// Standard gas limit used across tests.
#[expect(clippy::as_conversions)]
pub const TEST_GAS_LIMIT: u64 = (i64::MAX - 1) as u64;

/// Account setup entry for [`make_test_db`].
pub struct TestAccount {
    pub address: Address,
    pub code: Code,
    pub storage: FxHashMap<H256, U256>,
}

/// Create an in-memory `GeneralizedDatabase` with pre-seeded accounts.
///
/// Each account gets `U256::MAX` balance and nonce 0.
/// This replaces the ~13-line boilerplate duplicated across 15+ test sites (R1).
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
            Account::new(U256::MAX, acct.code, 0, acct.storage),
        );
    }

    GeneralizedDatabase::new_with_account_state(Arc::new(vm_db), cache)
}

/// Create a standard test environment for a contract call.
pub fn make_test_env(sender: Address) -> Environment {
    Environment {
        origin: sender,
        gas_limit: TEST_GAS_LIMIT,
        block_gas_limit: TEST_GAS_LIMIT,
        ..Default::default()
    }
}

/// Create a standard EIP-1559 transaction for a contract call.
pub fn make_test_tx(contract: Address, calldata: Bytes) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        to: TxKind::Call(contract),
        data: calldata,
        ..Default::default()
    })
}

/// Create standard contract + sender accounts for a simple test.
///
/// Returns `(contract_addr, sender_addr, accounts)` ready for [`make_test_db`].
pub fn make_contract_accounts(
    code: Code,
    storage: FxHashMap<H256, U256>,
) -> (Address, Address, Vec<TestAccount>) {
    let contract_addr = Address::from_low_u64_be(CONTRACT_ADDR);
    let sender_addr = Address::from_low_u64_be(SENDER_ADDR);

    let accounts = vec![
        TestAccount {
            address: contract_addr,
            code,
            storage,
        },
        TestAccount {
            address: sender_addr,
            code: Code::from_bytecode(Bytes::new()),
            storage: FxHashMap::default(),
        },
    ];

    (contract_addr, sender_addr, accounts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_test_db_creates_accounts() {
        let contract_addr = Address::from_low_u64_be(CONTRACT_ADDR);
        let sender_addr = Address::from_low_u64_be(SENDER_ADDR);

        let mut storage = FxHashMap::default();
        storage.insert(H256::zero(), U256::from(5u64));

        let code = Code::from_bytecode(Bytes::from(vec![0x60, 0x00, 0xf3]));
        let (c, s, accounts) = make_contract_accounts(code, storage);
        assert_eq!(c, contract_addr);
        assert_eq!(s, sender_addr);

        let db = make_test_db(accounts);
        assert!(db.current_accounts_state.contains_key(&contract_addr));
        assert!(db.current_accounts_state.contains_key(&sender_addr));

        let contract_acct = &db.current_accounts_state[&contract_addr];
        assert_eq!(
            contract_acct.storage.get(&H256::zero()).copied(),
            Some(U256::from(5u64))
        );
    }

    #[test]
    fn test_make_test_env_sets_gas() {
        let sender = Address::from_low_u64_be(SENDER_ADDR);
        let env = make_test_env(sender);
        assert_eq!(env.origin, sender);
        assert_eq!(env.gas_limit, TEST_GAS_LIMIT);
        assert_eq!(env.block_gas_limit, TEST_GAS_LIMIT);
    }

    #[test]
    fn test_intrinsic_gas_constant() {
        assert_eq!(INTRINSIC_GAS, 21_000);
    }
}
