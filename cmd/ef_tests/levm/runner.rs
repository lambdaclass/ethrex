use crate::{
    report::EFTestsReport,
    types::{EFTest, EFTestPostValue, TransactionExpectedException},
    utils,
};
use ethrex_core::{
    types::{code_hash, AccountInfo},
    H256, U256,
};
use ethrex_levm::{
    db::Cache,
    errors::{TransactionReport, VMError},
    vm::VM,
    Environment,
};
use ethrex_storage::AccountUpdate;
use ethrex_vm::db::StoreWrapper;
use keccak_hash::keccak;
use spinoff::{spinners::Dots, Color, Spinner};
use std::{collections::HashMap, error::Error, sync::Arc};

pub fn run_ef_tests() -> Result<EFTestsReport, Box<dyn Error>> {
    let mut report = EFTestsReport::default();
    let cargo_manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ef_general_state_tests_path = cargo_manifest_dir.join("vectors/GeneralStateTests");
    let mut spinner = Spinner::new(Dots, report.progress(), Color::Cyan);
    for test_dir in std::fs::read_dir(ef_general_state_tests_path)?.flatten() {
        for test in std::fs::read_dir(test_dir.path())?
            .flatten()
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
        {
            // TODO: Figure out what to do with overflowed value: 0x10000000000000000000000000000000000000000000000000000000000000001.
            // Deserialization of ValueOverflowParis fails because the value is too big for U256.
            // Intrinsic is skipped because execution fails as access lists are not yet implemented.
            if test
                .path()
                .file_name()
                .is_some_and(|name| name == "ValueOverflowParis.json" || name == "intrinsic.json")
            {
                continue;
            }
            // 'If' for running a specific test when necessary.
            // if test
            //     .path()
            //     .file_name()
            //     .is_some_and(|name| name == "buffer.json")
            // {
            let test_result = run_ef_test(
                serde_json::from_reader(std::fs::File::open(test.path())?)?,
                &mut report,
            );
            if test_result.is_err() {
                continue;
            }
            // }
        }
        spinner.update_text(report.progress());
    }
    spinner.success(&report.progress());
    let mut spinner = Spinner::new(Dots, "Loading report...".to_owned(), Color::Cyan);
    spinner.success(&report.to_string());
    Ok(report)
}

pub fn run_ef_test_tx(
    tx_id: usize,
    test: &EFTest,
    report: &mut EFTestsReport,
) -> Result<(), Box<dyn Error>> {
    let mut evm = prepare_vm_for_tx(tx_id, test)?;
    ensure_pre_state(&evm, test)?;
    let execution_result = evm.transact();
    ensure_post_state(execution_result, test, report, tx_id)?;
    Ok(())
}

pub fn run_ef_test(test: EFTest, report: &mut EFTestsReport) -> Result<(), Box<dyn Error>> {
    println!("Running test: {}", &test.name);
    let mut failed = false;
    for (tx_id, (tx_indexes, _tx)) in test.transactions.iter().enumerate() {
        // Code for debugging a specific case.
        // if *tx_indexes != (346, 0, 0) {
        //     continue;
        // }
        match run_ef_test_tx(tx_id, &test, report) {
            Ok(_) => {}
            Err(e) => {
                failed = true;
                let error_message: &str = &e.to_string();
                report.register_fail(tx_indexes.to_owned(), &test.name, error_message);
            }
        }
    }
    if failed {
        report.register_group_fail();
    } else {
        report.register_group_pass();
    }
    Ok(())
}

pub fn prepare_vm_for_tx(tx_id: usize, test: &EFTest) -> Result<VM, Box<dyn Error>> {
    let (initial_state, block_hash) = utils::load_initial_state(test);
    let db = Arc::new(StoreWrapper {
        store: initial_state.database().unwrap().clone(),
        block_hash,
    });
    let vm_result = VM::new(
        test.transactions.get(tx_id).unwrap().1.to.clone(),
        Environment {
            origin: test.transactions.get(tx_id).unwrap().1.sender,
            consumed_gas: U256::default(),
            refunded_gas: U256::default(),
            gas_limit: test.transactions.get(tx_id).unwrap().1.gas_limit, // Gas limit of Tx
            block_number: test.env.current_number,
            coinbase: test.env.current_coinbase,
            timestamp: test.env.current_timestamp,
            prev_randao: Some(test.env.current_random),
            chain_id: U256::from(1729),
            base_fee_per_gas: test.env.current_base_fee,
            gas_price: test
                .transactions
                .get(tx_id)
                .unwrap()
                .1
                .gas_price
                .unwrap_or_default(), // or max_fee_per_gas?
            block_excess_blob_gas: Some(test.env.current_excess_blob_gas),
            block_blob_gas_used: None,
            tx_blob_hashes: test
                .transactions
                .get(tx_id)
                .unwrap()
                .1
                .blob_versioned_hashes
                .clone(),
            block_gas_limit: test.env.current_gas_limit,
            tx_max_priority_fee_per_gas: test
                .transactions
                .get(tx_id)
                .unwrap()
                .1
                .max_priority_fee_per_gas,
            tx_max_fee_per_gas: test.transactions.get(tx_id).unwrap().1.max_fee_per_gas,
            tx_max_fee_per_blob_gas: test.transactions.get(tx_id).unwrap().1.max_fee_per_blob_gas,
        },
        test.transactions.get(tx_id).unwrap().1.value,
        test.transactions.get(tx_id).unwrap().1.data.clone(),
        db,
        Cache::default(),
    );

    match vm_result {
        Ok(vm) => Ok(vm),
        Err(err) => {
            let error_reason = format!("VM initialization failed: {err:?}");
            Err(error_reason.into())
        }
    }
}

pub fn ensure_pre_state(evm: &VM, test: &EFTest) -> Result<(), Box<dyn Error>> {
    let world_state = &evm.db;
    for (address, pre_value) in &test.pre.0 {
        let account = world_state.get_account_info(*address);
        ensure_pre_state_condition(
            account.nonce == pre_value.nonce.as_u64(),
            format!(
                "Nonce mismatch for account {:#x}: expected {}, got {}",
                address, pre_value.nonce, account.nonce
            ),
        )?;
        ensure_pre_state_condition(
            account.balance == pre_value.balance,
            format!(
                "Balance mismatch for account {:#x}: expected {}, got {}",
                address, pre_value.balance, account.balance
            ),
        )?;
        for (k, v) in &pre_value.storage {
            let mut key_bytes = [0u8; 32];
            k.to_big_endian(&mut key_bytes);
            let storage_slot = world_state.get_storage_slot(*address, H256::from_slice(&key_bytes));
            ensure_pre_state_condition(
                &storage_slot == v,
                format!(
                    "Storage slot mismatch for account {:#x} at key {:?}: expected {}, got {}",
                    address, k, v, storage_slot
                ),
            )?;
        }
        ensure_pre_state_condition(
            keccak(account.bytecode.clone()) == keccak(pre_value.code.as_ref()),
            format!(
                "Code hash mismatch for account {:#x}: expected {}, got {}",
                address,
                keccak(pre_value.code.as_ref()),
                keccak(account.bytecode)
            ),
        )?;
    }
    Ok(())
}

fn ensure_pre_state_condition(condition: bool, error_reason: String) -> Result<(), Box<dyn Error>> {
    if !condition {
        let error_reason = format!("Pre-state condition failed: {error_reason}");
        return Err(error_reason.into());
    }
    Ok(())
}

fn get_indexes_tuple(post_value: &EFTestPostValue) -> Option<(usize, usize, usize)> {
    let data_index: usize = post_value.indexes.get("data")?.as_usize();
    let gas_index: usize = post_value.indexes.get("gas")?.as_usize();
    let value_index: usize = post_value.indexes.get("value")?.as_usize();
    Some((data_index, gas_index, value_index))
}

fn get_post_value(test: &EFTest, tx_id: usize) -> Option<EFTestPostValue> {
    if let Some(transaction) = test.transactions.get(tx_id) {
        let indexes = transaction.0;
        test.post
            .clone()
            .iter()
            .find(|post_value| {
                if let Some(post_indexes) = get_indexes_tuple(post_value) {
                    indexes == post_indexes
                } else {
                    false
                }
            })
            .cloned()
    } else {
        None
    }
}

fn exception_is_expected(
    expected_exceptions: Vec<TransactionExpectedException>,
    returned_error: VMError,
) -> bool {
    for expected_exception in expected_exceptions {
        match expected_exception {
            // Returned OutOfGas(MaxGasLimitExceeded) but expected IntrinsicGasTooLow
            TransactionExpectedException::IntrinsicGasTooLow => {
                if returned_error == VMError::IntrinsicGasTooLow {
                    return true;
                }
            }
            TransactionExpectedException::InsufficientAccountFunds => {
                if returned_error == VMError::InsufficientAccountFunds {
                    return true;
                }
            }
            TransactionExpectedException::PriorityGreaterThanMaxFeePerGas => {
                if returned_error == VMError::PriorityGreaterThanMaxFeePerGas {
                    return true;
                }
            }
            TransactionExpectedException::GasLimitPriceProductOverflow => {
                if returned_error == VMError::GasLimitPriceProductOverflow {
                    return true;
                }
            }
            TransactionExpectedException::SenderNotEoa => {
                if returned_error == VMError::SenderNotEOA {
                    return true;
                }
            }
            TransactionExpectedException::InsufficientMaxFeePerGas => {
                if returned_error == VMError::InsufficientMaxFeePerGas {
                    return true;
                }
            }
            TransactionExpectedException::NonceIsMax => {
                if returned_error == VMError::NonceIsMax {
                    return true;
                }
            }
            TransactionExpectedException::GasAllowanceExceeded => {
                if returned_error == VMError::GasAllowanceExceeded {
                    return true;
                }
            }
            _ => {
                return false;
            }
        }
    }
    false
}

pub fn ensure_post_state(
    execution_result: Result<TransactionReport, VMError>,
    test: &EFTest,
    report: &mut EFTestsReport,
    tx_id: usize,
) -> Result<(), Box<dyn Error>> {
    let post_value = get_post_value(test, tx_id);
    match execution_result {
        Ok(execution_report) => {
            match post_value.clone().map(|v| v.clone().expect_exception) {
                // Execution result was successful but an exception was expected.
                Some(Some(expected_exceptions)) => {
                    let error_reason = match expected_exceptions.get(1) {
                        Some(second_exception) => {
                            format!(
                                "Expected exception: {:?} or {:?}",
                                expected_exceptions.first().unwrap(),
                                second_exception
                            )
                        }
                        None => {
                            format!(
                                "Expected exception: {:?}",
                                expected_exceptions.first().unwrap()
                            )
                        }
                    };

                    return Err(format!("Post-state condition failed: {error_reason}").into());
                }
                // Execution result was successful and no exception was expected.
                // TODO: Check that the post-state matches the expected post-state.
                None | Some(None) => {
                    let pos_state_root = post_state_root(execution_report, test);
                    if let Some(expected_post_state_root_hash) = post_value {
                        let expected_post_state_root_hash = expected_post_state_root_hash.hash;
                        if expected_post_state_root_hash != pos_state_root {
                            let error_reason = format!(
                                "Post-state root mismatch: expected {expected_post_state_root_hash:#x}, got {pos_state_root:#x}",
                            );
                            return Err(
                                format!("Post-state condition failed: {error_reason}").into()
                            );
                        }
                    } else {
                        let error_reason = "No post-state root hash provided";
                        return Err(format!("Post-state condition failed: {error_reason}").into());
                    }
                }
            }
        }
        Err(err) => {
            match post_value.map(|v| v.clone().expect_exception) {
                // Execution result was unsuccessful and an exception was expected.
                // TODO: Check that the exception matches the expected exception.
                Some(Some(expected_exceptions)) => {
                    println!("Expected exception is {:?}", expected_exceptions);

                    // Instead of cloning could use references
                    if !exception_is_expected(expected_exceptions.clone(), err.clone()) {
                        let error_reason = match expected_exceptions.get(1) {
                            Some(second_exception) => {
                                format!(
                                    "Returned exception is not the expected: Returned {:?} but expected {:?} or {:?}",
                                    err,
                                    expected_exceptions.first().unwrap(),
                                    second_exception
                                )
                            }
                            None => {
                                format!(
                                    "Returned exception is not the expected: Returned {:?} but expected {:?}",
                                    err,
                                    expected_exceptions.first().unwrap()
                                )
                            }
                        };
                        return Err(format!("Post-state condition failed: {error_reason}").into());
                    }
                }
                // Execution result was unsuccessful but no exception was expected.
                None | Some(None) => {
                    let error_reason = format!("Unexpected exception: {err:?}");
                    return Err(format!("Post-state condition failed: {error_reason}").into());
                }
            }
        }
    };
    report.register_pass(&test.name);
    Ok(())
}

pub fn post_state_root(execution_report: TransactionReport, test: &EFTest) -> H256 {
    let (initial_state, block_hash) = utils::load_initial_state(test);

    let mut account_updates: Vec<AccountUpdate> = vec![];
    for (address, account) in execution_report.new_state {
        let mut added_storage = HashMap::new();

        for (key, value) in account.storage {
            added_storage.insert(key, value.current_value);
        }

        let code = if account.info.bytecode.is_empty() {
            None
        } else {
            Some(account.info.bytecode.clone())
        };

        let account_update = AccountUpdate {
            address,
            removed: false,
            info: Some(AccountInfo {
                code_hash: code_hash(&account.info.bytecode),
                balance: account.info.balance,
                nonce: account.info.nonce,
            }),
            code,
            added_storage,
        };

        account_updates.push(account_update);
    }

    initial_state
        .database()
        .unwrap()
        .apply_account_updates(block_hash, &account_updates)
        .unwrap()
        .unwrap()
}
