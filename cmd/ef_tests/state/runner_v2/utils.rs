use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{U256, types::Genesis};
use ethrex_levm::db::{CacheDB, gen_db::GeneralizedDatabase};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::DynVmDatabase;
use keccak_hash::H256;

use std::sync::Arc;

use crate::runner_v2::{
    error::RunnerError,
    types::{Env, Test, TestCase},
};

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
pub async fn load_initial_state(test: &Test) -> (GeneralizedDatabase, H256, Store, Genesis) {
    let genesis = Genesis::from(test);
    let storage = Store::new("./temp", EngineType::InMemory).expect("Failed to create Store");

    storage.add_initial_state(genesis.clone()).await.unwrap();

    let block_hash = genesis.get_block().hash();
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(storage.clone(), block_hash));

    // We return some values that will be needed to calculate the post execution checks (original storage, genesis and blockhash)
    (
        GeneralizedDatabase::new(Arc::new(store), CacheDB::new()),
        block_hash,
        storage,
        genesis,
    )
}
