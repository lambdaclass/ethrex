use crate::types::{EFTest, EFTestTransaction};
use ethrex_core::{types::Genesis, H256, U256};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::{evm_state, EvmState};

pub fn load_initial_state(test: &EFTest) -> (EvmState, H256) {
    let genesis = Genesis::from(test);

    let storage = Store::new("./temp", EngineType::InMemory).expect("Failed to create Store");
    storage.add_initial_state(genesis.clone()).unwrap();

    (
        evm_state(
            storage.clone(),
            genesis.get_block().header.compute_block_hash(),
        ),
        genesis.get_block().header.compute_block_hash(),
    )
}

// If gas price is not provided, calculate it with current base fee and priority fee
pub fn effective_gas_price(test: &EFTest, tx: &&EFTestTransaction) -> U256 {
    match tx.gas_price {
        None => {
            let current_base_fee = test.env.current_base_fee.unwrap();
            let priority_fee = tx.max_priority_fee_per_gas.unwrap();
            let max_fee_per_gas = tx.max_fee_per_gas.unwrap();
            std::cmp::min(max_fee_per_gas, current_base_fee + priority_fee)
        }
        Some(price) => price,
    }
}
