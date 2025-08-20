use std::{fmt, fs::OpenOptions, io::Write, path::PathBuf};

use crate::modules::{error::RunnerError, result_check::PostCheckResult, types::Test};

pub fn add_test_to_report(test_result: (&Test, Vec<PostCheckResult>)) -> Result<(), RunnerError> {
    let (test, failed_test_cases) = test_result;
    if failed_test_cases.is_empty() {
        write_passing_test_to_report(test);
    } else {
        write_failing_test_to_report(test, failed_test_cases);
    }
    Ok(())
}
pub fn write_passing_test_to_report(test: &Test) {
    let successful_report_path = PathBuf::from("./success_report.txt");
    let mut report = OpenOptions::new()
        .append(true)
        .create(true)
        .open(successful_report_path)
        .unwrap();
    let content = format!("Test {:?} - Path {:?}\n", test.name, test.path);
    report.write_all(content.as_bytes()).unwrap()
}
pub fn write_failing_test_to_report(test: &Test, failing_test_cases: Vec<PostCheckResult>) {
    let failing_report_path = PathBuf::from("./failure_report.txt");
    let mut report = OpenOptions::new()
        .append(true)
        .create(true)
        .open(failing_report_path)
        .unwrap();
    let content = format!(
        "Test checks failed for test: {:?}. \nTest path: {:?}\nTest description/comment: {}\nTest doc reference: {}\n ",
        test.name,
        test.path,
        test._info.description.clone().unwrap_or(
            test._info
                .comment
                .clone()
                .unwrap_or("This test has no description or comment".to_string())
        ),
        test._info
            .reference_spec
            .clone()
            .unwrap_or("This test has no reference spec".to_string())
    );
    report.write_all(content.as_bytes()).unwrap();

    for check_result in failing_test_cases {
        let content = format!("\n{}", check_result);
        report.write_all(content.as_bytes()).unwrap();
    }
    let dividing_line = "-----------------------------------------------------\n\n".to_string();
    let _ = report.write_all(dividing_line.as_bytes());
}

impl fmt::Display for PostCheckResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Fork: {:?} - indexes (data: {}, gas: {}, value: {})\n",
            self.fork, self.vector.0, self.vector.1, self.vector.2
        )?;
        if let Some(root_mismatch) = self.root_diff {
            let (expected_root, actual_root) = root_mismatch;
            writeln!(
                f,
                "  ERR - ROOT MISMATCH:\n    Expected root: {:?}\n    Actual   root: {:?}",
                expected_root, actual_root
            )?;
        }

        if let Some(exception_diff) = self.exception_diff.clone() {
            let (expected_exception, actual_exception) = exception_diff;
            writeln!(
                f,
                "  ERR - EXCEPTION MISMATCH:\n    Expected exception: {:?}\n    Actual   exception: {:?}",
                expected_exception, actual_exception
            )?;
        }

        if let Some(logs_mismatch) = self.logs_diff {
            let (expected_log_hash, actual_log_hash) = logs_mismatch;
            writeln!(
                f,
                "  ERR - LOGS MISMATCH:\n    Expected logs hash: {:?}\n    Actual   logs hash: {:?}",
                expected_log_hash, actual_log_hash
            )?;
        }

        if let Some(account_mismatches) = self.accounts_diff.clone() {
            for acc_mismatch in account_mismatches {
                writeln!(
                    f,
                    "  ERR - ACCOUNT STATE MISMATCH:\n    Address: {:?}\n",
                    acc_mismatch.address,
                )?;
                if let Some(balance_diff) = acc_mismatch.balance_diff {
                    let (expected_balance, actual_balance) = balance_diff;
                    let net_difference = expected_balance.abs_diff(actual_balance);
                    let difference_sign = if expected_balance > actual_balance {
                        "-"
                    } else {
                        "+"
                    };
                    writeln!(
                        f,
                        "     Expected balance: {:?}\n     Actual   balance: {:?}\n     Difference: {}{:?}\n",
                        expected_balance, actual_balance, difference_sign, net_difference
                    )?;
                }
                if let Some(nonce_diff) = acc_mismatch.nonce_diff {
                    let (expected_nonce, actual_nonce) = nonce_diff;
                    writeln!(
                        f,
                        "     Expected nonce: {:?}\n     Actual   nonce: {:?}\n",
                        expected_nonce, actual_nonce
                    )?;
                }
                if let Some(code_diff) = acc_mismatch.code_diff {
                    let (expected_code_hash, actual_code_hash) = code_diff;
                    writeln!(
                        f,
                        "     Expected code hash: 0x{}\n     Actual   code hash: 0x{}\n",
                        hex::encode(expected_code_hash),
                        hex::encode(actual_code_hash)
                    )?;
                }

                if let Some(storage_diff) = acc_mismatch.storage_diff {
                    let (expected_storage, actual_storage) = storage_diff;
                    writeln!(
                        f,
                        "     Expected storage: {:?}\n     Actual   storage: {:?}",
                        expected_storage, actual_storage
                    )?;
                }
            }
        }

        Ok(())
    }
}
