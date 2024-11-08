use crate::ef::{report::EFTestsReport, test::EFTest};
use ethereum_rust_core::{H256, U256};
use ethereum_rust_levm::{
    db::{Cache, Db},
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
        spinner.update_text(report.to_string());
    }

    Ok(report)
}

pub fn run_ef_test(test: EFTest, report: &mut EFTestsReport) -> Result<(), Box<dyn Error>> {
    let evm = prepare_vm(&test);
    ensure_pre_state(&evm, &test, report)?;
    // let _transaction_report = evm.transact().unwrap();
    ensure_post_state(&evm, &test, report)?;
    report.register_pass(&test.name);
    Ok(())
}

pub fn prepare_vm(test: &EFTest) -> VM {
    VM::new(
        test.transaction.to.clone(),
        Environment {
            origin: test.transaction.sender,
            consumed_gas: test.env.current_gas_limit,
            refunded_gas: U256::default(),
            gas_limit: *test.transaction.gas_limit.first().unwrap(),
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
    )
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
        report.register_fail(&test.name, "Pre-state condition failed");
        return Err(error_reason.into());
    }
    Ok(())
}

pub fn ensure_post_state(
    _evm: &VM,
    _test: &EFTest,
    _report: &mut EFTestsReport,
) -> Result<(), Box<dyn Error>> {
    Ok(())
}
