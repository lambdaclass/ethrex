use serde::Deserialize;
use std::fs::{self, File};
use std::io::BufReader;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestCase {
    summary_result: SummaryResult,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SummaryResult {
    pass: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonFile {
    name: String,
    test_cases: std::collections::HashMap<String, TestCase>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut results = Vec::new();

    for entry in fs::read_dir("hive/workspace/logs")? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file()
            && path.extension().and_then(|s| s.to_str()) == Some("json")
            && path.file_name().and_then(|s| s.to_str()) != Some("hive.json")
        {
            let file_name = path
                .file_name()
                .and_then(|s| s.to_str())
                .expect("Path should be a valid string");
            let file = File::open(&path)?;
            let reader = BufReader::new(file);

            let json_data: JsonFile = match serde_json::from_reader(reader) {
                Ok(data) => data,
                Err(_) => {
                    eprintln!("Error processing file: {}", file_name);
                    continue;
                }
            };

            let total_tests = json_data.test_cases.len();
            let passed_tests = json_data
                .test_cases
                .values()
                .filter(|test_case| test_case.summary_result.pass)
                .count();

            results.push((json_data.name, passed_tests, total_tests));
        }
    }

    // Sort by file name.
    results.sort_by(|a, b| a.0.cmp(&b.0));

    for (file_name, passed, total) in &results {
        let success_percentage = (*passed as f64 / *total as f64) * 100.0;
        println!("{file_name}: {passed}/{total} ({success_percentage:.02}%)");
    }
    println!();
    let total_passed = results.iter().map(|(_, p, _)| p).sum::<usize>();
    let total_tests = results.iter().map(|(_, _, t)| t).sum::<usize>();
    let total_percentage = (total_passed as f64 / total_tests as f64) * 100.0;
    println!("Total: {total_passed}/{total_tests} ({total_percentage:.02}%)");

    Ok(())
}
