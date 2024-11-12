use crate::ef::{report::EFTestsReport, test::EFTest};
use ethereum_rust_core::{H256, U256};
use ethereum_rust_levm::{
    db::{Cache, Db},
    errors::{TransactionReport, VMError},
    vm::VM,
    Environment,
};
use keccak_hash::keccak;
use spinoff::{spinners::Dots, Color, Spinner};
use std::{error::Error, sync::Arc};

pub fn run_ef_tests() -> Result<EFTestsReport, Box<dyn Error>> {
    let mut report = EFTestsReport::default();
    let cargo_manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ef_general_state_tests_path = cargo_manifest_dir.join("tests/ef/tests/GeneralStateTests");
    let mut spinner = Spinner::new(Dots, report.to_string(), Color::Cyan);
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
            // Deserialization fails because the value is too big for U256.
            if test
                .path()
                .file_name()
                .is_some_and(|name| name == "ValueOverflowParis.json")
            {
                continue;
            }
            let test_result = run_ef_test(
                serde_json::from_reader(std::fs::File::open(test.path())?)?,
                &mut report,
            );
            if test_result.is_err() {
                continue;
            }
        }
        spinner.update_text(report.progress());
    }
    spinner.success(&report.progress());
    let mut spinner = Spinner::new(Dots, "Loading report...".to_owned(), Color::Cyan);
    spinner.success(&report.to_string());
    Ok(report)
}

pub fn run_ef_test(test: EFTest, report: &mut EFTestsReport) -> Result<(), Box<dyn Error>> {
    dbg!(&test.name);
    let mut evm = prepare_vm(&test, report)?;
    ensure_pre_state(&evm, &test, report)?;
    let execution_result = evm.transact();
    ensure_post_state(execution_result, &evm, &test, report)?;
    Ok(())
}

pub fn prepare_vm(test: &EFTest, report: &mut EFTestsReport) -> Result<VM, Box<dyn Error>> {
    let vm_result = VM::new(
        test.transaction.to.clone(),
        Environment {
            origin: test.transaction.sender,
            consumed_gas: U256::default(),
            refunded_gas: U256::default(),
            gas_limit: test.env.current_gas_limit,
            block_number: test.env.current_number,
            coinbase: test.env.current_coinbase,
            timestamp: test.env.current_timestamp,
            prev_randao: Some(test.env.current_random),
            chain_id: U256::from(1729),
            base_fee_per_gas: test.env.current_base_fee,
            gas_price: test.transaction.gas_price.unwrap_or_default(), // or max_fee_per_gas?
            block_excess_blob_gas: Some(test.env.current_excess_blob_gas),
            block_blob_gas_used: None,
            tx_blob_hashes: None,
        },
        *test.transaction.value.first().unwrap(),
        test.transaction.data.first().unwrap().clone(),
        Arc::new(Db::from(test)),
        Cache::default(),
    );

    match vm_result {
        Ok(vm) => Ok(vm),
        Err(err) => {
            let error_reason = format!("VM initialization failed: {err:?}");
            report.register_fail(&test.name, &error_reason);
            Err(error_reason.into())
        }
    }
}

pub fn ensure_pre_state(
    evm: &VM,
    test: &EFTest,
    report: &mut EFTestsReport,
) -> Result<(), Box<dyn Error>> {
    let world_state = &evm.db;
    for (address, pre_value) in &test.pre.0 {
        let account = world_state.get_account_info(*address);
        ensure_pre_state_condition(
            account.nonce == pre_value.nonce.as_u64(),
            format!(
                "Nonce mismatch for account {:#x}: expected {}, got {}",
                address, pre_value.nonce, account.nonce
            ),
            test,
            report,
        )?;
        ensure_pre_state_condition(
            account.balance == pre_value.balance,
            format!(
                "Balance mismatch for account {:#x}: expected {}, got {}",
                address, pre_value.balance, account.balance
            ),
            test,
            report,
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
                test,
                report,
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
            test,
            report,
        )?;
    }
    Ok(())
}

fn ensure_pre_state_condition(
    condition: bool,
    error_reason: String,
    test: &EFTest,
    report: &mut EFTestsReport,
) -> Result<(), Box<dyn Error>> {
    if !condition {
        let error_reason = format!("Pre-state condition failed: {error_reason}");
        report.register_fail(&test.name, &error_reason);
        return Err(error_reason.into());
    }
    Ok(())
}

pub fn ensure_post_state(
    execution_result: Result<TransactionReport, VMError>,
    _evm: &VM,
    test: &EFTest,
    report: &mut EFTestsReport,
) -> Result<(), Box<dyn Error>> {
    match execution_result {
        Ok(_execution_report) => {
            match test
                .post
                .clone()
                .values()
                .first()
                .map(|v| v.clone().expect_exception)
            {
                // Execution result was successful but an exception was expected.
                Some(Some(expected_exception)) => {
                    let error_reason = format!("Expected exception: {expected_exception}");
                    report.register_fail(&test.name, &error_reason);
                    return Err(format!("Post-state condition failed: {error_reason}").into());
                }
                // Execution result was successful and no exception was expected.
                // TODO: Check that the post-state matches the expected post-state.
                None | Some(None) => {}
            }
        }
        Err(err) => {
            match test
                .post
                .clone()
                .values()
                .first()
                .map(|v| v.clone().expect_exception)
            {
                // Execution result was unsuccessful and an exception was expected.
                // TODO: Check that the exception matches the expected exception.
                Some(Some(_expected_exception)) => {}
                // Execution result was unsuccessful but no exception was expected.
                None | Some(None) => {
                    let error_reason = format!("Unexpected exception: {err:?}");
                    report.register_fail(&test.name, &error_reason);
                    return Err(format!("Post-state condition failed: {error_reason}").into());
                }
            }
        }
    };
    report.register_pass(&test.name);
    Ok(())
}
