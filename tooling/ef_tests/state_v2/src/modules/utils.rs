use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::H256;
use ethrex_common::{
    Address, U256,
    types::{AccountState, ChainConfig, Code, CodeMetadata, Fork, Genesis},
    utils::keccak,
};
use ethrex_levm::db::Database as LevmDatabase;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::errors::DatabaseError;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::DynVmDatabase;

use std::sync::Arc;

use crate::modules::{
    error::RunnerError,
    types::{Env, Test, TestCase, genesis_from_test_and_fork},
};

/// Wraps an inner levm `Database` to enforce the EF state-test convention for
/// `BLOCKHASH(n)` = `keccak256(decimal_string(n))`, independent of underlying
/// storage. Matches geth's `vmTestBlockHash` in `tests/state_test_util.go`.
///
/// Without this override BLOCKHASH(0) at block 1 returns the genesis hash that
/// ethrex derives from the test's pre-state, which doesn't match the hash
/// fixtures put in `env.previousHash` (the EF convention). That trips
/// differential fuzzers like goevmlab on the very first block-hash lookup.
pub struct StatetestDatabase {
    inner: Arc<dyn LevmDatabase>,
}

impl StatetestDatabase {
    pub fn new(inner: Arc<dyn LevmDatabase>) -> Self {
        Self { inner }
    }
}

impl LevmDatabase for StatetestDatabase {
    fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
        self.inner.get_account_state(address)
    }
    fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
        self.inner.get_storage_value(address, key)
    }
    fn get_block_hash(&self, block_number: u64) -> Result<H256, DatabaseError> {
        Ok(keccak(block_number.to_string().as_bytes()))
    }
    fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
        self.inner.get_chain_config()
    }
    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        self.inner.get_account_code(code_hash)
    }
    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
        self.inner.get_code_metadata(code_hash)
    }
}

/// Calculates the price of the gas based on the fields the test case has. For transaction types
/// previous to EIP1559, the gas_price is explicit in the test. For later transaction types, it requires
/// to be calculated based on `current_base_fee`, `priority_fee` and `max_fee_per_gas` values.
pub fn effective_gas_price(test_env: &Env, test_case: &TestCase) -> Result<U256, RunnerError> {
    match test_case.gas_price {
        None => {
            let current_base_fee = test_env.current_base_fee.unwrap();
            let priority_fee = test_case.max_priority_fee_per_gas.unwrap();
            let max_fee_per_gas = test_case.max_fee_per_gas.unwrap();

            Ok(std::cmp::min(
                max_fee_per_gas,
                current_base_fee + priority_fee,
            ))
        }
        Some(price) => Ok(price),
    }
}

/// Loads the pre state of the test (the initial state of specific accounts) into the Genesis.
pub async fn load_initial_state(
    test: &Test,
    fork: &Fork,
) -> (GeneralizedDatabase, H256, Store, Genesis) {
    let genesis = genesis_from_test_and_fork(test, fork);
    let mut storage = Store::new("./temp", EngineType::InMemory).expect("Failed to create Store");

    storage.add_initial_state(genesis.clone()).await.unwrap();

    let block_hash = genesis.get_block().hash();
    let store: DynVmDatabase =
        Box::new(StoreVmDatabase::new(storage.clone(), genesis.get_block().header).unwrap());
    let inner: Arc<dyn LevmDatabase> = Arc::new(store);
    let wrapped: Arc<dyn LevmDatabase> = Arc::new(StatetestDatabase::new(inner));

    // We return some values that will be needed to calculate the post execution checks (original storage, genesis and blockhash)
    (
        GeneralizedDatabase::new(wrapped),
        block_hash,
        storage,
        genesis,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    /// Minimal LevmDatabase stub that returns a sentinel block hash, so the
    /// test can assert that `StatetestDatabase` overrides BLOCKHASH and never
    /// consults the inner DB for it.
    struct SentinelInner;
    impl LevmDatabase for SentinelInner {
        fn get_account_state(&self, _: Address) -> Result<AccountState, DatabaseError> {
            unreachable!("not exercised by BLOCKHASH test")
        }
        fn get_storage_value(&self, _: Address, _: H256) -> Result<U256, DatabaseError> {
            unreachable!("not exercised by BLOCKHASH test")
        }
        fn get_block_hash(&self, _: u64) -> Result<H256, DatabaseError> {
            // If the wrapper ever delegates BLOCKHASH downward, this sentinel
            // proves the bug — the override path is the only correct answer.
            Ok(H256::repeat_byte(0xff))
        }
        fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
            unreachable!("not exercised by BLOCKHASH test")
        }
        fn get_account_code(&self, _: H256) -> Result<Code, DatabaseError> {
            unreachable!("not exercised by BLOCKHASH test")
        }
        fn get_code_metadata(&self, _: H256) -> Result<CodeMetadata, DatabaseError> {
            unreachable!("not exercised by BLOCKHASH test")
        }
    }

    #[test]
    fn blockhash_zero_matches_ef_convention() {
        let db = StatetestDatabase::new(Arc::new(SentinelInner));
        // Per geth's vmTestBlockHash: keccak256("0").
        let expected = H256::from_str(
            "0x044852b2a670ade5407e78fb2863c51de9fcb96542a07186fe3aeda6bb8a116d",
        )
        .unwrap();
        assert_eq!(db.get_block_hash(0).unwrap(), expected);
    }

    #[test]
    fn blockhash_nonzero_matches_ef_convention() {
        let db = StatetestDatabase::new(Arc::new(SentinelInner));
        // keccak256("1") per geth's vmTestBlockHash.
        let expected = H256::from_str(
            "0xc89efdaa54c0f20c7adf612882df0950f5a951637e0307cdcb4c672f298b8bc6",
        )
        .unwrap();
        assert_eq!(db.get_block_hash(1).unwrap(), expected);
    }

    #[test]
    fn blockhash_uses_decimal_not_hex() {
        // Specifically pin the decimal-string convention. For n=10, ascii "10"
        // and ascii "a" would hash to different values; we must use decimal.
        let db = StatetestDatabase::new(Arc::new(SentinelInner));
        let got = db.get_block_hash(10).unwrap();
        let expected = keccak(b"10");
        assert_eq!(got, expected);
        assert_ne!(got, keccak(b"a"), "must hash decimal string, not hex");
    }
}
