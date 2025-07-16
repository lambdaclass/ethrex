use std::{fs::OpenOptions, io::Write, path::PathBuf};

use ethrex_common::types::Fork;

use crate::runner_v2::{
    error::RunnerError,
    types::{Test, TestCase},
};

pub fn create_report(test_result: (&Test, Vec<&TestCase>)) -> Result<(), RunnerError> {
    let passing_report_path = PathBuf::from("./runner_v2/success_report.txt");
    let failing_report_path = PathBuf::from("./runner_v2/failure_report.txt");
    let test = test_result.0;
    let failed_test_cases = test_result.1;
    let (content, report_path) = if failed_test_cases.is_empty() {
        (
            format!(
                "Test checks succeded for test: {:?}, in path: {:?}.\n",
                test.name, test.path
            ),
            passing_report_path,
        )
    } else {
        let failed_forks: Vec<Fork> = failed_test_cases
            .iter()
            .map(|tc| {
                tc.fork
            })
            .collect();
        (
            format!(
                "Test checks failed for test: {:?}, in path: {:?} for forks: {:?}.\n",
                test.name, test.path, failed_forks
            ),
            failing_report_path,
        )
    };
    let mut report = OpenOptions::new()
        .append(true)
        .create(true)
        .open(report_path)
        .map_err(|err| RunnerError::FailedToCreateReportFile(err.to_string()))?;
    report
        .write_all(content.as_bytes())
        .map_err(|err| RunnerError::FailedToWriteReport(err.to_string()))?;

    Ok(())
}
