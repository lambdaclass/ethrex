use std::{collections::BTreeMap, sync::Arc};

use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    U256,
    types::{AccountUpdate, EIP1559Transaction, Genesis, GenesisAccount, Transaction},
};
use ethrex_levm::{
    EVMConfig, Environment,
    db::{CacheDB, gen_db::GeneralizedDatabase},
    tracing::LevmCallTracer,
    vm::VM,
};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::{DynVmDatabase, backends};
use keccak_hash::H256;
use spinoff::spinners::Runner;

use crate::runner_v2::{
    error::RunnerError,
    types::{Env, Test, TestCase},
};
pub async fn run_tests(tests: Vec<Test>) {
    for test in tests {
        run_test(&test).await;
    }
}

pub async fn run_test(test: &Test) -> Result<(), RunnerError> {
    for test_case in &test.test_cases {
        // new vm
        let (mut db, initial_block_hash, storage) = load_initial_state_levm(test).await;
        let env = get_vm_env_for_test(test.env, test_case);
        let tx = &get_tx_from_test_case(test_case);
        let tracer = LevmCallTracer::disabled();
        let vm_type = ethrex_levm::vm::VMType::L1;
        let mut vm = VM::new(env, &mut db, tx, tracer, vm_type);

        let execution_report = vm
            .execute()
            .map_err(|e| RunnerError::VMExecutionError(e.to_string()))?;
        check_test_case_results(&mut vm, initial_block_hash, storage, test_case).await?;
    }
    Ok(())
}

pub fn get_vm_env_for_test(test_env: Env, test_case: &TestCase) -> Environment {
    let blob_schedule = EVMConfig::canonical_values(test_case.fork);
    let config = EVMConfig::new(test_case.fork, blob_schedule);
    let gas_price = effective_gas_price(&test_case);
    let tx_blob_hashes = Vec::new();
    let tx_max_fee_per_gas = None;
    let tx_max_fee_per_blob_gas = None;
    let tx_max_priority_fee_per_gas = None;
    Environment {
        origin: test_case.sender,
        gas_limit: test_case.gas,
        config,
        block_number: test_env.current_number,
        coinbase: test_env.current_coinbase,
        timestamp: test_env.current_timestamp,
        prev_randao: test_env.current_random,
        difficulty: test_env.current_difficulty,
        chain_id: U256::from(1),
        base_fee_per_gas: test_env.current_base_fee.unwrap_or_default(),
        gas_price,
        block_excess_blob_gas: test_env.current_excess_blob_gas,
        block_blob_gas_used: None,
        tx_blob_hashes,
        tx_max_priority_fee_per_gas,
        tx_max_fee_per_gas,
        tx_max_fee_per_blob_gas,
        tx_nonce: test_case.nonce,
        block_gas_limit: test_env.current_gas_limit,
        is_privileged: false,
    }
}

pub fn get_tx_from_test_case(test_case: &TestCase) -> Transaction {
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        to: test_case.to.clone(),
        value: test_case.value,
        data: test_case.data.clone(),
        access_list: Vec::new(),
        ..Default::default()
    });
    tx
}

pub fn effective_gas_price(test_case: &TestCase) -> U256 {
    match test_case.gas_price {
        None => U256::zero(),
        Some(price) => price,
    }
}

pub async fn load_initial_state_levm(test: &Test) -> (GeneralizedDatabase, H256, Store) {
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

impl From<&Test> for Genesis {
    fn from(test: &Test) -> Self {
        Genesis {
            alloc: {
                let mut alloc = BTreeMap::new();
                for (account, account_state) in &test.pre {
                    alloc.insert(*account, GenesisAccount::from(account_state));
                }
                alloc
            },
            coinbase: test.env.current_coinbase,
            difficulty: test.env.current_difficulty,
            gas_limit: test.env.current_gas_limit,
            mix_hash: test.env.current_random.unwrap_or_default(),
            timestamp: test.env.current_timestamp.as_u64(),
            base_fee_per_gas: test.env.current_base_fee.map(|v| v.as_u64()),
            excess_blob_gas: test.env.current_excess_blob_gas.map(|v| v.as_u64()),
            ..Default::default()
        }
    }
}

pub async fn post_state_root(
    account_updates: &[AccountUpdate],
    initial_block_hash: H256,
    store: Store,
) -> H256 {
    let ret_account_updates_batch = store
        .apply_account_updates_batch(initial_block_hash, account_updates)
        .await
        .unwrap()
        .unwrap();
    ret_account_updates_batch.state_trie_hash
}

pub async fn check_test_case_results(
    vm: &mut VM<'_>,
    initial_block_hash: H256,
    store: Store,
    test_case: &TestCase,
) -> Result<(), RunnerError> {
    let account_updates = backends::levm::LEVM::get_state_transitions(vm.db)
        .map_err(|_| RunnerError::FailedToGetAccountsUpdates)?;
    let post_state_root = post_state_root(&account_updates, initial_block_hash, store).await;
    if post_state_root != test_case.post.hash {
        return Err(RunnerError::RootMismatch);
    }
    Ok(())
}
