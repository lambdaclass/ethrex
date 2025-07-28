use std::{fmt, fs::OpenOptions, io::Write, path::PathBuf};

use crate::runner_v2::{error::RunnerError, result_check::PostCheckResult, types::Test};

/// Adds the result of running a Test to the report.
pub fn add_to_report(test_result: (&Test, Vec<PostCheckResult>)) -> Result<(), RunnerError> {
    let test = test_result.0;
    let failed_test_cases = test_result.1;
    if failed_test_cases.is_empty() {
        write_passing_tests_to_report(test);
    } else {
        write_failed_tests_to_report(test, failed_test_cases);
    }
    Ok(())
}

/// Writes a specific test passed.
pub fn write_passing_tests_to_report(test: &Test) {
    let successful_report_path = PathBuf::from("./runner_v2/success_report.txt");
    let mut report = OpenOptions::new()
        .append(true)
        .create(true)
        .open(successful_report_path)
        .unwrap();
    let content = format!(
        "Test {:?} in path {:?} was SUCCESSFUL for all forks.\n",
        test.name, test.path
    );
    report.write_all(content.as_bytes()).unwrap()
}

/// Writes for a failing tests the details of its differences with the expected post state.
pub fn write_failed_tests_to_report(test: &Test, failing_test_cases: Vec<PostCheckResult>) {
    let failing_report_path = PathBuf::from("./runner_v2/failure_report.txt");
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
        writeln!(f, "Fork: {:?} - vector {:?}\n", self.fork, self.vector)?;
        if let Some(root_mismatch) = self.root_diff {
            writeln!(
                f,
                "  ERR - ROOT MISMATCH:\n    Expected root: {:?}\n    Actual   root: {:?}",
                root_mismatch.0, root_mismatch.1
            )?;
        }

        if let Some(exception_diff) = self.exception_diff.clone() {
            writeln!(
                f,
                "  ERR - EXCEPTION MISMATCH:\n    Expected exception: {:?}\n    Actual   exception: {:?}",
                exception_diff.0, exception_diff.1
            )?;
        }

        if let Some(logs_mismatch) = self.logs_diff {
            writeln!(
                f,
                "  ERR - LOGS MISMATCH:\n    Expected logs hash: {:?}\n    Actual   logs hash: {:?}",
                logs_mismatch.0, logs_mismatch.1
            )?;
        }

        if let Some(account_mismatches) = self.accounts_diff.clone() {
            for acc_mismatch in account_mismatches {
                writeln!(
                    f,
                    "  ERR - ACCOUNT STATE MISMATCH:\n    Address: {:?}\n     Expected balance: {:?}\n     Actual   balance: {:?}\n     Expected nonce: {:?}\n     Actual   nonce: {:?}\n     Expected code: 0x{}\n     Actual   code: 0x{}\n     Expected storage: {:?}\n     Actual   storage: {:?}",
                    acc_mismatch.address,
                    acc_mismatch.expected_balance,
                    acc_mismatch.actual_balance,
                    acc_mismatch.expected_nonce,
                    acc_mismatch.actual_nonce,
                    hex::encode(acc_mismatch.expected_code),
                    hex::encode(acc_mismatch.actual_code),
                    acc_mismatch.expected_storage,
                    acc_mismatch.actual_storage
                )?;
            }
        }

        Ok(())
    }
}
