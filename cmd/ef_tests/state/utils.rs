use std::sync::Arc;

use crate::{
    runner::{EFTestRunnerError, InternalError},
    types::{EFTest, EFTestTransaction},
};
use ethrex_common::{types::Genesis, H256, U256};
use ethrex_levm::db::{gen_db::GeneralizedDatabase, CacheDB};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::{
    backends::revm::db::{evm_state, EvmState},
    StoreWrapper,
};

/// Loads initial state, used for REVM as it contains EvmState.
pub async fn load_initial_state(test: &EFTest) -> (EvmState, H256) {
    let genesis = Genesis::from(test);

    let storage = Store::new("./temp", EngineType::InMemory).expect("Failed to create Store");
    storage.add_initial_state(genesis.clone()).await.unwrap();

    (
        evm_state(
            storage.clone(),
            genesis.get_block().header.compute_block_hash(),
        ),
        genesis.get_block().header.compute_block_hash(),
    )
}

/// Loads initial state, function for LEVM as it does not require EvmState
pub async fn load_initial_state_levm(test: &EFTest) -> GeneralizedDatabase {
    let genesis = Genesis::from(test);

    let storage = Store::new("./temp", EngineType::InMemory).expect("Failed to create Store");
    storage.add_initial_state(genesis.clone()).await.unwrap();

    let block_hash = genesis.get_block().header.compute_block_hash();

    let store = StoreWrapper {
        store: storage,
        block_hash,
    };

    GeneralizedDatabase::new(Arc::new(store), CacheDB::new())
}

// If gas price is not provided, calculate it with current base fee and priority fee
pub fn effective_gas_price(
    test: &EFTest,
    tx: &&EFTestTransaction,
) -> Result<U256, EFTestRunnerError> {
    match tx.gas_price {
        None => {
            let current_base_fee = test
                .env
                .current_base_fee
                .ok_or(EFTestRunnerError::Internal(
                    InternalError::FirstRunInternal("current_base_fee not found".to_string()),
                ))?;
            let priority_fee = tx
                .max_priority_fee_per_gas
                .ok_or(EFTestRunnerError::Internal(
                    InternalError::FirstRunInternal(
                        "max_priority_fee_per_gas not found".to_string(),
                    ),
                ))?;
            let max_fee_per_gas = tx.max_fee_per_gas.ok_or(EFTestRunnerError::Internal(
                InternalError::FirstRunInternal("max_fee_per_gas not found".to_string()),
            ))?;

            Ok(std::cmp::min(
                max_fee_per_gas,
                current_base_fee + priority_fee,
            ))
        }
        Some(price) => Ok(price),
    }
}
