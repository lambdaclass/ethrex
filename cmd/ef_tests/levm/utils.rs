use crate::types::EFTest;
use ethrex_core::{types::Genesis, H256};
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
