use crate::{
    report::{EFTestReport, EFTestReportForkResult, TestVector},
    runner::{EFTestRunnerError, InternalError},
    types::{EFTest, EFTestTransaction, TransactionExpectedException},
    utils::{self, effective_gas_price, load_genesis, load_initial_state_store},
};

use ethrex_common::{
    serde_utils::blob::vec,
    types::{
        compute_transactions_root, transaction, tx_fields::*, AccountUpdate, BlobsBundle, Block, BlockBody, BlockHeader, EIP1559Transaction, EIP2930Transaction, EIP4844Transaction, EIP7702Transaction, Fork, Genesis, Signable, Transaction, TxKind, ELASTICITY_MULTIPLIER
    },
    H256, U256
};
use ethrex_levm::{
    db::gen_db::GeneralizedDatabase,
    errors::{ExecutionReport, TxValidationError, VMError},
    vm::VM,
    EVMConfig, Environment,
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_vm::backends::{self, levm::LEVM};
use keccak_hash::keccak;
use secp256k1::SecretKey;

use ethrex_blockchain::{
    payload::{create_payload, BuildPayloadArgs},
    Blockchain,
};

pub async fn run_ef_test(test: &EFTest) -> Result<EFTestReport, EFTestRunnerError> {
    // There are some tests that don't have a hash, unwrap will panic
    let hash = test
        ._info
        .generated_test_hash
        .or(test._info.hash)
        .unwrap_or_default();

    let mut ef_test_report = EFTestReport::new(test.name.clone(), test.dir.clone(), hash);
    for fork in test.post.forks.keys() {
        let mut ef_test_report_fork = EFTestReportForkResult::new();

        for (vector, _tx) in test.transactions.iter() {
            // This is because there are some test vectors that are not valid for the current fork.
            if !test.post.has_vector_for_fork(vector, *fork) {
                continue;
            }
            match run_ef_test_tx(vector, test, fork).await {
                Ok(_) => continue,
                Err(EFTestRunnerError::BlockBuildingFailure(reason)) => {
                    ef_test_report_fork.register_block_building_failure(reason, *vector);
                }
                Err(EFTestRunnerError::VMInitializationFailed(reason)) => {
                    ef_test_report_fork.register_vm_initialization_failure(reason, *vector);
                }
                Err(EFTestRunnerError::FailedToEnsurePreState(reason)) => {
                    ef_test_report_fork.register_pre_state_validation_failure(reason, *vector);
                }
                Err(EFTestRunnerError::ExecutionFailedUnexpectedly(error)) => {
                    ef_test_report_fork.register_unexpected_execution_failure(error, *vector);
                }
                Err(EFTestRunnerError::FailedToEnsurePostState(
                    transaction_report,
                    reason,
                    levm_cache,
                )) => {
                    ef_test_report_fork.register_post_state_validation_failure(
                        transaction_report,
                        reason,
                        *vector,
                        levm_cache,
                    );
                }
                Err(EFTestRunnerError::VMExecutionMismatch(_)) => {
                    return Err(EFTestRunnerError::Internal(InternalError::FirstRunInternal(
                        "VM execution mismatch errors should only happen when running with revm. This failed during levm's execution."
                            .to_owned(),
                    )));
                }
                Err(EFTestRunnerError::ExpectedExceptionDoesNotMatchReceived(reason)) => {
                    ef_test_report_fork
                        .register_post_state_validation_error_mismatch(reason, *vector);
                }
                Err(EFTestRunnerError::Internal(reason)) => {
                    return Err(EFTestRunnerError::Internal(reason));
                }
                | Err(EFTestRunnerError::GasLimitOverflow)
                | Err(EFTestRunnerError::EIP4844ShouldNotBeCreateType)
                | Err(EFTestRunnerError::EIP7702ShouldNotBeCreateType) => {
                    return Err(EFTestRunnerError::Internal(InternalError::Custom(
                        "This case should not happen".to_owned(),
                    )));
                }
            }
        }
        ef_test_report.register_fork_result(*fork, ef_test_report_fork);
    }
    Ok(ef_test_report)
}

pub async fn run_ef_test_tx(
    vector: &TestVector,
    test: &EFTest,
    fork: &Fork,
) -> Result<(), EFTestRunnerError> {
    let test_tx = test
        .transactions
        .get(vector)
        .ok_or(EFTestRunnerError::Internal(
            InternalError::FirstRunInternal("Failed to get transaction".to_owned()),
        ))?;
    let mut db = utils::load_initial_state_levm(test, fork).await;
    // For handling edge case in which there's a create in a Type 4 Transaction, that sadly is detected before actual execution of the vm, when building the "Transaction" for creating a new instance of vm.
    let (transaction, levm_execution_result) = match build_transaction(test_tx) {
        Err(EFTestRunnerError::EIP7702ShouldNotBeCreateType) => {
            let err = Err(VMError::TxValidation(
                TxValidationError::Type4TxContractCreation,
            ));
            (None, err)
        }
        Err(EFTestRunnerError::EIP4844ShouldNotBeCreateType) => {
            let err = Err(VMError::TxValidation(
                TxValidationError::Type3TxContractCreation,
            ));
            (None, err)
        }
        Err(EFTestRunnerError::GasLimitOverflow) => {
            let err = Err(VMError::TxValidation(
                TxValidationError::GasLimitPriceProductOverflow,
            ));
            (None, err)
        }
        Err(error) => return Err(error),
        Ok(tx) => {
            let mut levm = prepare_vm_for_tx(test, &tx, test_tx, fork, &mut db)
                .map_err(|e| EFTestRunnerError::VMInitializationFailed(e.to_string()))?;
            ensure_pre_state(&levm, test)?;
            (Some(tx), levm.execute())
        }
    };

    ensure_post_state(&levm_execution_result, vector, test, fork, &mut db).await?;

    let Some(transaction) = transaction else {
        // Might have failed due to Type4 transaction create
        return Ok(());
    };
    if test.post.vector_post_value(vector, *fork).expect_exception.is_some() {
        // Invalid transactions shouldn't be included in blocks
        return Ok(());
    }
    let genesis = load_genesis(test, fork);
    let genesis_hash = genesis.get_block().hash();

    let store = load_initial_state_store(&genesis).await;
    let blockchain = Blockchain::default_with_store(store.clone());

    let blobs_bundle = BlobsBundle::empty();
    blockchain.mempool.add_blobs_bundle(transaction.compute_hash(), blobs_bundle)
        .map_err(|e| EFTestRunnerError::BlockBuildingFailure(e.to_string()))?;
    

    let args = BuildPayloadArgs {
        parent: genesis_hash,
        timestamp: test.env.current_timestamp.try_into().unwrap(),
        fee_recipient: test.env.current_coinbase,
        random: test.env.current_random.unwrap_or_default(),
        withdrawals: None,
        beacon_root: None,
        version: 0,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
    };

    let mut block = create_payload(&args, &store)
        .map_err(|e| EFTestRunnerError::BlockBuildingFailure(e.to_string()))?;
    block.header.base_fee_per_gas = test.env.current_base_fee.map(|v| v.try_into().unwrap_or_default());
    block.header.difficulty = test.env.current_difficulty;
    block.header.gas_limit = test.env.current_gas_limit;
    block.header.excess_blob_gas = test.env.current_excess_blob_gas.map(|v| v.try_into().unwrap_or_default());
    
    let res = blockchain
        .build_payload_with_transactions(block, &vec![transaction])
        .await
        .map_err(|e| EFTestRunnerError::BlockBuildingFailure(e.to_string()))?;
    let block = res.payload;

    let mut db = utils::load_initial_state_levm(test, fork).await;
    LEVM::execute_block(&block, &mut db).map_err(|e| {
        EFTestRunnerError::Internal(InternalError::StatelessRunnerInternal(e.to_string()))
    })?;
    let updates = LEVM::get_state_transitions(&mut db).map_err(|e| {
        EFTestRunnerError::Internal(InternalError::StatelessRunnerInternal(e.to_string()))
    })?;
    let new_root = store
        .apply_account_updates(genesis_hash, &updates)
        .await
        .map_err(|e| {
            EFTestRunnerError::Internal(InternalError::StatelessRunnerInternal(e.to_string()))
        })?;
    let expected_root = test.post.vector_post_value(vector, fork.clone()).hash;
    if new_root != Some(expected_root) {
        return Err(EFTestRunnerError::Internal(
            InternalError::StatelessRunnerInternal("Stateful and stateless mismatch".to_string()),
        ));
    }
    Ok(())
}

pub fn prepare_vm_for_tx<'a>(
    test: &EFTest,
    tx: &Transaction,
    test_tx: &EFTestTransaction,
    fork: &Fork,
    db: &'a mut GeneralizedDatabase,
) -> Result<VM<'a>, EFTestRunnerError> {
    let blob_schedule = EVMConfig::canonical_values(*fork);
    let config = EVMConfig::new(*fork, blob_schedule);
    Ok(VM::new(
        Environment {
            origin: test_tx.sender,
            gas_limit: test_tx.gas_limit,
            config,
            block_number: test.env.current_number,
            coinbase: test.env.current_coinbase,
            timestamp: test.env.current_timestamp,
            prev_randao: test.env.current_random,
            difficulty: test.env.current_difficulty,
            chain_id: U256::from(1),
            base_fee_per_gas: test.env.current_base_fee.unwrap_or_default(),
            gas_price: effective_gas_price(test, &test_tx)?,
            block_excess_blob_gas: test.env.current_excess_blob_gas,
            block_blob_gas_used: None,
            tx_blob_hashes: test_tx.blob_versioned_hashes.clone(),
            tx_max_priority_fee_per_gas: test_tx.max_priority_fee_per_gas,
            tx_max_fee_per_gas: test_tx.max_fee_per_gas,
            tx_max_fee_per_blob_gas: test_tx.max_fee_per_blob_gas,
            tx_nonce: test_tx.nonce,
            block_gas_limit: test.env.current_gas_limit,
            is_privileged: false,
        },
        db,
        tx,
    ))
}

pub fn build_transaction(
    test_tx: &EFTestTransaction
) -> Result<Transaction, EFTestRunnerError> {
    let access_list = test_tx
        .access_list
        .iter()
        .map(|arg| (arg.address, arg.storage_keys.clone()))
        .collect();

    // Check if the tx has the authorization_lists field implemented by eip7702.
    let authorization_list = test_tx.authorization_list.clone().map(|list| {
        list.iter()
            .map(|auth_tuple| AuthorizationTuple {
                chain_id: auth_tuple.chain_id,
                address: auth_tuple.address,
                nonce: auth_tuple.nonce,
                y_parity: auth_tuple.v,
                r_signature: auth_tuple.r,
                s_signature: auth_tuple.s,
            })
            .collect::<Vec<AuthorizationTuple>>()
    });
    
    let private_key  = SecretKey::from_slice(&test_tx.secret_key.0).map_err(|_| EFTestRunnerError::Internal(InternalError::Custom("failed to decode secret key".to_string())))?;
    let max_priority_fee_per_gas = test_tx.max_priority_fee_per_gas.unwrap_or_default().try_into().map_err(|_| EFTestRunnerError::Internal(InternalError::Custom("failed to convert max_priority_fee_per_gas".to_string())))?;
    let max_fee_per_gas = test_tx.max_fee_per_gas.unwrap_or_default().try_into().map_err(|_| EFTestRunnerError::Internal(InternalError::Custom("failed to convert max_fee_per_gas".to_string())))?;
    let gas_limit = test_tx.gas_limit.try_into().map_err(|_| EFTestRunnerError::Internal(InternalError::Custom("failed to convert gas_limit".to_string())))?;
    let max_fee_per_blob_gas = test_tx.max_fee_per_blob_gas.unwrap_or_default().try_into().map_err(|_| EFTestRunnerError::Internal(InternalError::Custom("failed to convert max_fee_per_blob_gas".to_string())))?;
    let gas_price = test_tx.gas_price.unwrap_or_default().try_into().map_err(|_| EFTestRunnerError::GasLimitOverflow)?;

    if test_tx.max_fee_per_gas.is_none() {
        return Ok(Transaction::EIP2930Transaction(EIP2930Transaction {
            to: test_tx.to.clone(),
            value: test_tx.value,
            data: test_tx.data.clone(),
            access_list,
            nonce: test_tx.nonce,
            gas_limit,
            gas_price,
            ..Default::default()
        }.sign(&private_key)))
    }
    if !test_tx.blob_versioned_hashes.is_empty() {
        return Ok(Transaction::EIP4844Transaction(EIP4844Transaction {
            to: match test_tx.to {
                TxKind::Call(to) => to,
                TxKind::Create => return Err(EFTestRunnerError::EIP4844ShouldNotBeCreateType),
            },
            value: test_tx.value,
            data: test_tx.data.clone(),
            access_list,
            nonce: test_tx.nonce,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas: gas_limit,
            max_fee_per_blob_gas: max_fee_per_blob_gas,
            blob_versioned_hashes: test_tx.blob_versioned_hashes.clone(),
            ..Default::default()
        }.sign(&private_key)))
    }
    Ok(match authorization_list {
        Some(list) => Transaction::EIP7702Transaction(EIP7702Transaction {
            to: match test_tx.to {
                TxKind::Call(to) => to,
                TxKind::Create => return Err(EFTestRunnerError::EIP7702ShouldNotBeCreateType),
            },
            value: test_tx.value,
            data: test_tx.data.clone(),
            access_list,
            authorization_list: list,
            nonce: test_tx.nonce,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas_limit,
            ..Default::default()
        }.sign(&private_key)),
        None => Transaction::EIP1559Transaction(EIP1559Transaction {
            to: test_tx.to.clone(),
            value: test_tx.value,
            data: test_tx.data.clone(),
            access_list,
            nonce: test_tx.nonce,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas_limit,
            ..Default::default()
        }.sign(&private_key)),
    })
}

pub fn ensure_pre_state(evm: &VM, test: &EFTest) -> Result<(), EFTestRunnerError> {
    let world_state = &evm.db.store;
    for (address, pre_value) in &test.pre.0 {
        let account = world_state.get_account(*address).map_err(|e| {
            EFTestRunnerError::Internal(InternalError::Custom(format!(
                "Failed to get account info when ensuring pre state: {}",
                e
            )))
        })?;
        ensure_pre_state_condition(
            account.info.nonce == pre_value.nonce,
            format!(
                "Nonce mismatch for account {:#x}: expected {}, got {}",
                address, pre_value.nonce, account.info.nonce
            ),
        )?;
        ensure_pre_state_condition(
            account.info.balance == pre_value.balance,
            format!(
                "Balance mismatch for account {:#x}: expected {}, got {}",
                address, pre_value.balance, account.info.balance
            ),
        )?;
        for (k, v) in &pre_value.storage {
            let storage_slot = world_state
                .get_storage_value(*address, H256::from_slice(&k.to_big_endian()))
                .unwrap();
            ensure_pre_state_condition(
                &storage_slot == v,
                format!(
                    "Storage slot mismatch for account {:#x} at key {:?}: expected {}, got {}",
                    address, k, v, storage_slot
                ),
            )?;
        }
        ensure_pre_state_condition(
            account.info.code_hash == keccak(pre_value.code.as_ref()),
            format!(
                "Code hash mismatch for account {:#x}: expected {}, got {}",
                address,
                keccak(pre_value.code.as_ref()),
                account.info.code_hash
            ),
        )?;
    }
    Ok(())
}

fn ensure_pre_state_condition(
    condition: bool,
    error_reason: String,
) -> Result<(), EFTestRunnerError> {
    if !condition {
        return Err(EFTestRunnerError::FailedToEnsurePreState(error_reason));
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

pub async fn ensure_post_state(
    levm_execution_result: &Result<ExecutionReport, VMError>,
    vector: &TestVector,
    test: &EFTest,
    fork: &Fork,
    db: &mut GeneralizedDatabase,
) -> Result<(), EFTestRunnerError> {
    let cache = db.cache.clone();
    match levm_execution_result {
        Ok(execution_report) => {
            match test.post.vector_post_value(vector, *fork).expect_exception {
                // Execution result was successful but an exception was expected.
                Some(expected_exceptions) => {
                    let error_reason = format!("Expected exception: {:?}", expected_exceptions);
                    return Err(EFTestRunnerError::FailedToEnsurePostState(
                        execution_report.clone(),
                        error_reason,
                        cache,
                    ));
                }
                // Execution result was successful and no exception was expected.
                None => {
                    let levm_account_updates = backends::levm::LEVM::get_state_transitions(db)
                        .map_err(|_| {
                            InternalError::Custom(
                                "Error at LEVM::get_state_transitions in ensure_post_state()"
                                    .to_owned(),
                            )
                        })?;
                    let vector_post_value = test.post.vector_post_value(vector, *fork);

                    // 1. Compare the post-state root hash with the expected post-state root hash
                    if vector_post_value.hash != post_state_root(&levm_account_updates, test, fork).await
                    {
                        return Err(EFTestRunnerError::FailedToEnsurePostState(
                            execution_report.clone(),
                            "Post-state root mismatch".to_string(),
                            cache,
                        ));
                    }

                    // 2. Compare keccak of logs with test's expected logs hash.

                    // Do keccak of the RLP of logs
                    let keccak_logs = {
                        let logs = execution_report.logs.clone();
                        let mut encoded_logs = Vec::new();
                        logs.encode(&mut encoded_logs);
                        keccak(encoded_logs)
                    };

                    if keccak_logs != vector_post_value.logs {
                        return Err(EFTestRunnerError::FailedToEnsurePostState(
                            execution_report.clone(),
                            "Logs mismatch".to_string(),
                            cache,
                        ));
                    }
                }
            }
        }
        Err(err) => {
            match test.post.vector_post_value(vector, *fork).expect_exception {
                // Execution result was unsuccessful and an exception was expected.
                Some(expected_exceptions) => {
                    if !exception_is_expected(expected_exceptions.clone(), err.clone()) {
                        let error_reason = format!(
                            "Returned exception {:?} does not match expected {:?}",
                            err, expected_exceptions
                        );
                        return Err(EFTestRunnerError::ExpectedExceptionDoesNotMatchReceived(
                            error_reason,
                        ));
                    }
                }
                // Execution result was unsuccessful but no exception was expected.
                None => {
                    return Err(EFTestRunnerError::ExecutionFailedUnexpectedly(err.clone()));
                }
            }
        }
    };
    Ok(())
}

pub async fn post_state_root(account_updates: &[AccountUpdate], test: &EFTest, fork: &Fork) -> H256 {
    let (_initial_state, block_hash, store) = utils::load_initial_state(test, fork).await;
    store
        .apply_account_updates(block_hash, account_updates)
        .await
        .unwrap()
        .unwrap()
}
