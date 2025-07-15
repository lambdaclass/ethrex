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

pub fn effective_gas_price(test_env: &Env, test_case: &TestCase) -> Result<U256, RunnerError> {
    match test_case.gas_price {
        None => {
            let current_base_fee = test_env
                .current_base_fee
                .ok_or(RunnerError::CurrentBaseFeeMissing)?;
            let priority_fee = test_case
                .max_priority_fee_per_gas
                .ok_or(RunnerError::MaxPriorityFeePerGasMissing)?;
            let max_fee_per_gas = test_case
                .max_fee_per_gas
                .ok_or(RunnerError::MaxFeePerGasMissing)?;

            Ok(std::cmp::min(
                max_fee_per_gas,
                current_base_fee + priority_fee,
            ))
        }
        Some(price) => Ok(price),
    }
}

pub async fn load_initial_state(test: &Test) -> (GeneralizedDatabase, H256, Store) {
    let genesis = Genesis::from(test);
    let storage = Store::new("./temp", EngineType::InMemory).expect("Failed to create Store");

    storage.add_initial_state(genesis.clone()).await.unwrap();

    let block_hash = genesis.get_block().hash();
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(storage.clone(), block_hash));

    (
        GeneralizedDatabase::new(Arc::new(store), CacheDB::new()),
        block_hash,
        storage,
    )
}
