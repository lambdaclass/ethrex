use std::{fmt, fs::OpenOptions, io::Write, path::PathBuf};

use ethrex_common::types::Fork;

use crate::runner_v2::{
    error::RunnerError, result_check::PostCheckResult, types::{Test, TestCase}
};

pub fn create_report(test_result: (&Test, Vec<(Fork, PostCheckResult)>)) -> Result<(), RunnerError> {
    let passing_report_path = PathBuf::from("./runner_v2/success_report.txt");
    let failing_report_path = PathBuf::from("./runner_v2/failure_report.txt");
    let test = test_result.0;
    let failed_test_cases = test_result.1;
    if !failed_test_cases.is_empty() {
        write_failed_tests_to_report(test, failed_test_cases);
    }
    Ok(())
}

pub fn write_failed_tests_to_report(test: &Test, failing_test_cases: Vec<(Fork, PostCheckResult)>) {
    let failing_report_path = PathBuf::from("./runner_v2/failure_report.txt");
    let mut report = OpenOptions::new()
        .append(true)
        .create(true)
        .open(failing_report_path)
        .map_err(|err| RunnerError::FailedToCreateReportFile(err.to_string()))
        .unwrap();
    let content = format!(
        "-----------------------------------------------------\nTest checks failed for test: {:?}. \nTest path: {:?}. ",
        test.name, test.path
    );
    let _ = report
        .write_all(content.as_bytes())
        .map_err(|err| RunnerError::FailedToWriteReport(err.to_string()));

    for (fork, check_result) in failing_test_cases {
        let content = format!(
            "\nFork: {:?}\n{}",
            fork, check_result
        );
        let _ = report
            .write_all(content.as_bytes())
            .map_err(|err| RunnerError::FailedToWriteReport(err.to_string()));
    }
    let content = format!(
        "-----------------------------------------------------\n\n"
    );
    let _ = report
        .write_all(content.as_bytes());
}

impl fmt::Display for PostCheckResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(root_mismatch) = self.root_dif {
            writeln!(f, "  ERR - ROOT MISMATCH:\n    Expected root: {:?}\n    Actual   root: {:?}", root_mismatch.0, root_mismatch.1)?;
        }

        if let Some(exception_diff) = self.exception_diff.clone() {
            writeln!(f, "  ERR - EXCEPTION MISMATCH:\n    Expected exception: {:?}\n    Actual   exception: {:?}", exception_diff.0, exception_diff.1)?;
        }

        if let Some(logs_mismatch) = self.logs_diff {
            writeln!(f, "  ERR - LOGS MISMATCH:\n    Expected logs hash: {:?}\n    Actual   logs hash: {:?}", logs_mismatch.0, logs_mismatch.1)?;
        }

        if let Some(account_mismatches) = self.accounts_diff.clone() {
            for acc_mismatch in account_mismatches {
                writeln!(f, "  ERR - ACCOUNT STATE MISMATCH:\n    Address: {:?}\n     Expected balance: {:?}\n     Actual   balance: {:?}\n     Expected nonce: {:?}\n     Actual   nonce: {:?}\n     Expected code: 0x{}\n     Actual   code: 0x{}\n     Expected storage: {:?}\n     Actual   storage: {:?}",
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
