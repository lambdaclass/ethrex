use std::{str::FromStr, sync::Arc};

use crate::{
    runner::{EFTestRunnerError, InternalError},
    types::{EFTest, EFTestTransaction},
};
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{types::{BlobSchedule, ChainConfig, Fork, Genesis}, H160, H256, U256};
use ethrex_levm::db::{gen_db::GeneralizedDatabase, CacheDB};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::{
    backends::revm::db::{evm_state, EvmState},
    DynVmDatabase,
};

fn enable_if_posterior(current: &Fork, target: Fork) -> Option<u64> {
    if *current >= target {
        Some(0)
    } else {
        None
    }
}

pub fn make_chainconfig(test: &EFTest, fork: &Fork) -> ChainConfig {
    ChainConfig {
        chain_id: 1,
        homestead_block: enable_if_posterior(fork, Fork::Homestead),
        dao_fork_block: enable_if_posterior(fork, Fork::DaoFork),
        dao_fork_support: *fork >= Fork::DaoFork,
        eip150_block: enable_if_posterior(fork, Fork::Tangerine),
        eip155_block: enable_if_posterior(fork, Fork::SpuriousDragon),
        eip158_block: enable_if_posterior(fork, Fork::SpuriousDragon), // replaced by EIP161, part of SD
        byzantium_block: enable_if_posterior(fork, Fork::Byzantium),
        constantinople_block: enable_if_posterior(fork, Fork::Constantinople),
        petersburg_block: enable_if_posterior(fork, Fork::Petersburg),
        istanbul_block: enable_if_posterior(fork, Fork::Istanbul),
        muir_glacier_block: enable_if_posterior(fork, Fork::MuirGlacier),
        berlin_block: enable_if_posterior(fork, Fork::Berlin),
        london_block: enable_if_posterior(fork, Fork::London),
        arrow_glacier_block: enable_if_posterior(fork, Fork::ArrowGlacier),
        gray_glacier_block: enable_if_posterior(fork, Fork::GrayGlacier),
        merge_netsplit_block: enable_if_posterior(fork, Fork::Paris),
        terminal_total_difficulty: Some(0),
        shanghai_time: enable_if_posterior(fork, Fork::Shanghai),
        cancun_time: enable_if_posterior(fork, Fork::Cancun),
        prague_time: enable_if_posterior(fork, Fork::Prague),
        terminal_total_difficulty_passed: false,
        verkle_time: None,
        blob_schedule: test.config.blob_schedule,
        // Mainnet address
        deposit_contract_address: H160::from_str("0x00000000219ab540356cbb839cbe05303d7705fa")
            .expect("Invalid deposit contract address"),
    }
}

pub fn load_genesis(test: &EFTest, fork: &Fork) -> Genesis {
    let mut genesis = Genesis::from(test);
    genesis.config = make_chainconfig(test, fork);
    genesis
}

pub async fn load_initial_state_store(genesis: &Genesis) -> Store {
    let storage = Store::new("./temp", EngineType::InMemory).expect("Failed to create Store");
    storage.add_initial_state(genesis.clone()).await.unwrap();
    storage
}

/// Loads initial state, used for REVM as it contains EvmState.
pub async fn load_initial_state(test: &EFTest, fork: &Fork) -> (EvmState, H256, Store) {
    let genesis = load_genesis(test, fork);

    let storage = load_initial_state_store(&genesis).await;

    let vm_db: DynVmDatabase = Box::new(StoreVmDatabase::new(
        storage.clone(),
        genesis.get_block().hash(),
    ));

    (evm_state(vm_db), genesis.get_block().hash(), storage)
}

/// Loads initial state, function for LEVM as it does not require EvmState
pub async fn load_initial_state_levm(test: &EFTest, fork: &Fork) -> GeneralizedDatabase {
    let genesis = load_genesis(test, fork);

    let storage = load_initial_state_store(&genesis).await;

    let block_hash = genesis.get_block().hash();

    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(storage, block_hash));

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
