use std::{
    fmt,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::OnceLock,
};

use chrono::Local;
use prettytable::{Cell, Row, Table};

use crate::modules::{error::RunnerError, result_check::PostCheckResult, types::Test};

/// Static storage for report paths that are initialized once per program run
static REPORT_PATHS: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();

/// Ensures the reports directory exists, creating it if necessary
pub fn ensure_reports_dir() -> Result<(), RunnerError> {
    let reports_dir = PathBuf::from("./reports");
    if !reports_dir.exists() {
        fs::create_dir_all(&reports_dir).map_err(|e| {
            RunnerError::Custom(format!("Failed to create reports directory: {}", e))
        })?;
    }
    Ok(())
}

/// Generates timestamped report paths (called only once per run)
fn get_report_paths() -> &'static (PathBuf, PathBuf) {
    REPORT_PATHS.get_or_init(|| {
        let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
        let success_path = PathBuf::from(format!("./reports/success_report_{}.txt", timestamp));
        let failure_path = PathBuf::from(format!("./reports/failure_report_{}.txt", timestamp));
        (success_path, failure_path)
    })
}

pub fn add_test_to_report(test_result: (&Test, Vec<PostCheckResult>)) -> Result<(), RunnerError> {
    ensure_reports_dir()?;
    let (test, failed_test_cases) = test_result;
    if failed_test_cases.is_empty() {
        write_passing_test_to_report(test);
    } else {
        write_failing_test_to_report(test, failed_test_cases);
    }
    Ok(())
}

pub fn write_passing_test_to_report(test: &Test) {
    let (successful_report_path, _) = get_report_paths();
    let mut report = OpenOptions::new()
        .append(true)
        .create(true)
        .open(successful_report_path)
        .unwrap();
    let content = format!("Test {:?} - Path {:?}\n", test.name, test.path);
    report.write_all(content.as_bytes()).unwrap()
}

pub fn write_failing_test_to_report(test: &Test, failing_test_cases: Vec<PostCheckResult>) {
    let (_, failing_report_path) = get_report_paths();
    let mut report = OpenOptions::new()
        .append(true)
        .create(true)
        .open(failing_report_path)
        .unwrap();

    // Create header table
    let mut header_table = Table::new();
    header_table.add_row(Row::new(vec![
        Cell::new("Test Information").style_spec("Fb"),
    ]));
    header_table.add_row(Row::new(vec![Cell::new("Name"), Cell::new(&test.name)]));
    header_table.add_row(Row::new(vec![
        Cell::new("Path"),
        Cell::new(&test.path.display().to_string()),
    ]));
    header_table.add_row(Row::new(vec![
        Cell::new("Description"),
        Cell::new(
            &test._info.description.clone().unwrap_or(
                test._info
                    .comment
                    .clone()
                    .unwrap_or("No description or comment".to_string()),
            ),
        ),
    ]));
    header_table.add_row(Row::new(vec![
        Cell::new("Reference"),
        Cell::new(
            &test
                ._info
                .reference_spec
                .clone()
                .unwrap_or("No reference spec".to_string()),
        ),
    ]));

    let header_content = format!("{}\n", header_table);
    report.write_all(header_content.as_bytes()).unwrap();

    for check_result in failing_test_cases {
        let content = format!("\n{}", check_result);
        report.write_all(content.as_bytes()).unwrap();
    }
    let dividing_line =
        "\n═══════════════════════════════════════════════════════════════════════\n\n".to_string();
    let _ = report.write_all(dividing_line.as_bytes());
}

impl fmt::Display for PostCheckResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Fork and indexes table
        let mut info_table = Table::new();
        info_table.add_row(Row::new(vec![
            Cell::new("Fork"),
            Cell::new("Data Idx"),
            Cell::new("Gas Idx"),
            Cell::new("Val Idx"),
            Cell::new("Status"),
        ]));
        info_table.add_row(Row::new(vec![
            Cell::new(&format!("{:?}", self.fork)),
            Cell::new(&self.vector.0.to_string()),
            Cell::new(&self.vector.1.to_string()),
            Cell::new(&self.vector.2.to_string()),
            Cell::new("FAILED").style_spec("Fr"),
        ]));
        writeln!(f, "{}", info_table)?;

        // Root mismatch
        if let Some(root_mismatch) = self.root_diff {
            let (expected_root, actual_root) = root_mismatch;
            writeln!(f, "\nERROR: Root Mismatch")?;
            let mut root_table = Table::new();
            root_table.add_row(Row::new(vec![Cell::new("Type"), Cell::new("Value")]));
            root_table.add_row(Row::new(vec![
                Cell::new("Expected"),
                Cell::new(&format!("{:?}", expected_root)),
            ]));
            root_table.add_row(Row::new(vec![
                Cell::new("Actual"),
                Cell::new(&format!("{:?}", actual_root)),
            ]));
            writeln!(f, "{}", root_table)?;
        }

        // Exception mismatch
        if let Some(exception_diff) = self.exception_diff.clone() {
            let (expected_exception, actual_exception) = exception_diff;
            writeln!(f, "\nERROR: Exception Mismatch")?;
            let mut exception_table = Table::new();
            exception_table.add_row(Row::new(vec![Cell::new("Type"), Cell::new("Value")]));
            exception_table.add_row(Row::new(vec![
                Cell::new("Expected"),
                Cell::new(&format!("{:?}", expected_exception)),
            ]));
            exception_table.add_row(Row::new(vec![
                Cell::new("Actual"),
                Cell::new(&format!("{:?}", actual_exception)),
            ]));
            writeln!(f, "{}", exception_table)?;
        }

        // Logs mismatch
        if let Some(logs_mismatch) = self.logs_diff {
            let (expected_log_hash, actual_log_hash) = logs_mismatch;
            writeln!(f, "\nERROR: Logs Mismatch")?;
            let mut logs_table = Table::new();
            logs_table.add_row(Row::new(vec![Cell::new("Type"), Cell::new("Value")]));
            logs_table.add_row(Row::new(vec![
                Cell::new("Expected"),
                Cell::new(&format!("{:?}", expected_log_hash)),
            ]));
            logs_table.add_row(Row::new(vec![
                Cell::new("Actual"),
                Cell::new(&format!("{:?}", actual_log_hash)),
            ]));
            writeln!(f, "{}", logs_table)?;
        }

        // Account mismatches
        if let Some(account_mismatches) = self.accounts_diff.clone() {
            for acc_mismatch in account_mismatches {
                writeln!(f, "\nERROR: Account State Mismatch")?;
                writeln!(f, "Address: {:?}", acc_mismatch.address)?;

                if let Some(balance_diff) = acc_mismatch.balance_diff {
                    let (expected_balance, actual_balance) = balance_diff;
                    let net_difference = expected_balance.abs_diff(actual_balance);
                    let difference_sign = if expected_balance > actual_balance {
                        "-"
                    } else {
                        "+"
                    };

                    let mut balance_table = Table::new();
                    balance_table.add_row(Row::new(vec![Cell::new("Type"), Cell::new("Value")]));
                    balance_table.add_row(Row::new(vec![
                        Cell::new("Expected Balance"),
                        Cell::new(&format!("{:?}", expected_balance)),
                    ]));
                    balance_table.add_row(Row::new(vec![
                        Cell::new("Actual Balance"),
                        Cell::new(&format!("{:?}", actual_balance)),
                    ]));
                    balance_table.add_row(Row::new(vec![
                        Cell::new("Difference"),
                        Cell::new(&format!("{}{:?}", difference_sign, net_difference)),
                    ]));
                    writeln!(f, "{}", balance_table)?;
                }

                if let Some(nonce_diff) = acc_mismatch.nonce_diff {
                    let (expected_nonce, actual_nonce) = nonce_diff;
                    let mut nonce_table = Table::new();
                    nonce_table.add_row(Row::new(vec![Cell::new("Type"), Cell::new("Value")]));
                    nonce_table.add_row(Row::new(vec![
                        Cell::new("Expected Nonce"),
                        Cell::new(&format!("{:?}", expected_nonce)),
                    ]));
                    nonce_table.add_row(Row::new(vec![
                        Cell::new("Actual Nonce"),
                        Cell::new(&format!("{:?}", actual_nonce)),
                    ]));
                    writeln!(f, "{}", nonce_table)?;
                }

                if let Some(code_diff) = acc_mismatch.code_diff {
                    let (expected_code_hash, actual_code_hash) = code_diff;
                    let mut code_table = Table::new();
                    code_table.add_row(Row::new(vec![Cell::new("Type"), Cell::new("Value")]));
                    code_table.add_row(Row::new(vec![
                        Cell::new("Expected Code Hash"),
                        Cell::new(&format!("0x{}", hex::encode(expected_code_hash))),
                    ]));
                    code_table.add_row(Row::new(vec![
                        Cell::new("Actual Code Hash"),
                        Cell::new(&format!("0x{}", hex::encode(actual_code_hash))),
                    ]));
                    writeln!(f, "{}", code_table)?;
                }

                if let Some(storage_diff) = acc_mismatch.storage_diff {
                    let (expected_storage, actual_storage) = storage_diff;
                    let mut storage_table = Table::new();
                    storage_table.add_row(Row::new(vec![Cell::new("Type"), Cell::new("Value")]));
                    storage_table.add_row(Row::new(vec![
                        Cell::new("Expected Storage"),
                        Cell::new(&format!("{:?}", expected_storage)),
                    ]));
                    storage_table.add_row(Row::new(vec![
                        Cell::new("Actual Storage"),
                        Cell::new(&format!("{:?}", actual_storage)),
                    ]));
                    writeln!(f, "{}", storage_table)?;
                }
            }
        }

        Ok(())
    }
}
