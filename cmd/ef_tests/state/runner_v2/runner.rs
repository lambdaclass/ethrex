use std::{collections::BTreeMap, sync::Arc};

use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::{
    types::{AccountUpdate, EIP1559Transaction, EIP7702Transaction, Genesis, GenesisAccount, Transaction, TxKind}, H160, U256
};
use ethrex_levm::{
    EVMConfig, Environment,
    db::{CacheDB, gen_db::GeneralizedDatabase},
    errors::{ExecutionReport, TxValidationError, VMError},
    tracing::LevmCallTracer,
    vm::VM,
};
use ethrex_storage::{EngineType, Store};
use ethrex_vm::{DynVmDatabase, backends};
use keccak_hash::H256;

use crate::{
    runner_v2::{
        error::RunnerError,
        types::{Env, Test, TestCase},
    },
    types::TransactionExpectedException,
};

pub async fn run_tests(tests: Vec<Test>) -> Result<(), RunnerError> {
    for test in tests {
        run_test(&test).await?;
    }
    Ok(())
}

pub async fn run_test(test: &Test) -> Result<(), RunnerError> {
    for test_case in &test.test_cases {
        // new vm
        let (mut db, initial_block_hash, storage) = load_initial_state_levm(test).await;
        let env = get_vm_env_for_test(test.env, test_case)?;

        let tx = &get_tx_from_test_case(test_case)?;
        let tracer = LevmCallTracer::disabled();
        let vm_type = ethrex_levm::vm::VMType::L1;
        let mut vm = VM::new(env.clone(), &mut db, tx, tracer, vm_type);

        let execution_report = vm.execute();
        let res = check_test_case_results(
            &mut vm,
            initial_block_hash,
            storage,
            test_case,
            execution_report,
        )
        .await;

        if res.is_err() {
            println!("Error: {:?}", res.err());
            println!("enviroment: {:?}", env);
            println!("initial state: {:?}",vm.db.initial_accounts_state);
            println!("current state: {:?}", vm.db.current_accounts_state);
        } else {
            println!("checks succeded");
        }
    }
    Ok(())
}

pub fn get_vm_env_for_test(
    test_env: Env,
    test_case: &TestCase,
) -> Result<Environment, RunnerError> {
    let blob_schedule = EVMConfig::canonical_values(test_case.fork);
    let config = EVMConfig::new(test_case.fork, blob_schedule);
    let gas_price = effective_gas_price(&test_env, &test_case)?;
    Ok(Environment {
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
        tx_blob_hashes: test_case.blob_versioned_hashes.clone(),
        tx_max_priority_fee_per_gas: test_case.max_priority_fee_per_gas,
        tx_max_fee_per_gas: test_case.max_fee_per_gas,
        tx_max_fee_per_blob_gas: test_case.max_fee_per_blob_gas,
        tx_nonce: test_case.nonce,
        block_gas_limit: test_env.current_gas_limit,
        is_privileged: false,
    })
}

pub fn get_tx_from_test_case(test_case: &TestCase) -> Result<Transaction, RunnerError> {
    let value = test_case.value;
    let data = test_case.data.clone();
    let access_list  = test_case.access_list.iter().map(|list_item| (list_item.address, list_item.storage_keys.clone())).collect();
    let tx = match &test_case.authorization_list {
        Some(list) => Transaction::EIP7702Transaction(EIP7702Transaction {
            to: match test_case.to {
                TxKind::Call(to) => to,
                TxKind::Create => return Err(RunnerError::EIP7702ShouldNotBeCreateType),
            },
            value,
            data,
            access_list,
            authorization_list: list.iter().map(|auth_tuple| auth_tuple.clone().into_authorization_tuple()).collect(),
            ..Default::default()
        }),
        None => Transaction::EIP1559Transaction(EIP1559Transaction {
            to: test_case.to.clone(),
            value,
            data,
            access_list,
            ..Default::default()
        }),
    };
    Ok(tx)

}

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
    execution_result: Result<ExecutionReport, VMError>,
) -> Result<(), RunnerError> {
    if test_case.expects_exception() {
        println!("test case expects exception");
        check_exception(
            test_case.post.expected_exception.clone().unwrap(),
            execution_result,
        )?;
    } else {
        println!("test case does not expect exception");
        check_root(vm, initial_block_hash, store, test_case).await?;
    }
    Ok(())
}

pub async fn check_root(
    vm: &mut VM<'_>,
    initial_block_hash: H256,
    store: Store,
    test_case: &TestCase,
) -> Result<(), RunnerError> {
    let account_updates = backends::levm::LEVM::get_state_transitions(vm.db)
        .map_err(|_| RunnerError::FailedToGetAccountsUpdates)?;
    let post_state_root = post_state_root(&account_updates, initial_block_hash, store).await;
    println!("Real post state root    : {:?}", post_state_root);
    println!("Expected post state root: {:?}", test_case.post.hash);
    if post_state_root != test_case.post.hash {
        return Err(RunnerError::RootMismatch);
    }
    Ok(())
}
pub fn check_exception(
    expected_exceptions: Vec<TransactionExpectedException>,
    execution_result: Result<ExecutionReport, VMError>,
) -> Result<(), RunnerError> {
    if execution_result.is_ok() {
        return Err(RunnerError::TxSucceededAndExceptionWasExpected);
    } else if !exception_is_expected(expected_exceptions, execution_result.err().unwrap()) {
        return Err(RunnerError::DifferentExceptionWasExpected);
    }
    Ok(())
}

// Exceptions not covered: RlpInvalidValue
fn exception_is_expected(
    expected_exceptions: Vec<TransactionExpectedException>,
    returned_error: VMError,
) -> bool {
    expected_exceptions.iter().any(|exception| {
        matches!(
            (exception, &returned_error),
            (
                TransactionExpectedException::IntrinsicGasTooLow,
                VMError::TxValidation(TxValidationError::IntrinsicGasTooLow)
            ) | (
                TransactionExpectedException::InsufficientAccountFunds,
                VMError::TxValidation(TxValidationError::InsufficientAccountFunds)
            ) | (
                TransactionExpectedException::PriorityGreaterThanMaxFeePerGas,
                VMError::TxValidation(TxValidationError::PriorityGreaterThanMaxFeePerGas)
            ) | (
                TransactionExpectedException::GasLimitPriceProductOverflow,
                VMError::TxValidation(TxValidationError::GasLimitPriceProductOverflow)
            ) | (
                TransactionExpectedException::SenderNotEoa,
                VMError::TxValidation(TxValidationError::SenderNotEOA)
            ) | (
                TransactionExpectedException::InsufficientMaxFeePerGas,
                VMError::TxValidation(TxValidationError::InsufficientMaxFeePerGas)
            ) | (
                TransactionExpectedException::NonceIsMax,
                VMError::TxValidation(TxValidationError::NonceIsMax)
            ) | (
                TransactionExpectedException::GasAllowanceExceeded,
                VMError::TxValidation(TxValidationError::GasAllowanceExceeded)
            ) | (
                TransactionExpectedException::Type3TxPreFork,
                VMError::TxValidation(TxValidationError::Type3TxPreFork)
            ) | (
                TransactionExpectedException::Type3TxBlobCountExceeded,
                VMError::TxValidation(TxValidationError::Type3TxBlobCountExceeded)
            ) | (
                TransactionExpectedException::Type3TxZeroBlobs,
                VMError::TxValidation(TxValidationError::Type3TxZeroBlobs)
            ) | (
                TransactionExpectedException::Type3TxContractCreation,
                VMError::TxValidation(TxValidationError::Type3TxContractCreation)
            ) | (
                TransactionExpectedException::Type3TxInvalidBlobVersionedHash,
                VMError::TxValidation(TxValidationError::Type3TxInvalidBlobVersionedHash)
            ) | (
                TransactionExpectedException::InsufficientMaxFeePerBlobGas,
                VMError::TxValidation(TxValidationError::InsufficientMaxFeePerBlobGas)
            ) | (
                TransactionExpectedException::InitcodeSizeExceeded,
                VMError::TxValidation(TxValidationError::InitcodeSizeExceeded)
            ) | (
                TransactionExpectedException::Type4TxContractCreation,
                VMError::TxValidation(TxValidationError::Type4TxContractCreation)
            ) | (
                TransactionExpectedException::Other,
                VMError::TxValidation(_) //TODO: Decide whether to support more specific errors, I think this is enough.
            )
        )
    })
}
