use std::fs::{self, File};
use std::io::BufReader;

use hive_report::{HiveResult, JsonFile};

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

            let result = HiveResult::new(json_data.name, passed_tests, total_tests);
            if !result.should_skip() {
                results.push(result);
            }
        }
    }

    // First by category ascending, then by passed tests descending, then by success percentage descending.
    results.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| b.passed_tests.cmp(&a.passed_tests))
            .then_with(|| {
                b.success_percentage
                    .partial_cmp(&a.success_percentage)
                    .unwrap()
            })
    });

    let results_by_category = results.chunk_by(|a, b| a.category == b.category);

    for results in results_by_category {
        // print category
        println!("*{}*", results[0].category);
        for result in results {
            println!("\t{}", result);
        }
        println!();
    }

    println!();
    let total_passed = results.iter().map(|r| r.passed_tests).sum::<usize>();
    let total_tests = results.iter().map(|r| r.total_tests).sum::<usize>();
    let total_percentage = (total_passed as f64 / total_tests as f64) * 100.0;
    println!("*Total: {total_passed}/{total_tests} ({total_percentage:.02}%)*");

    Ok(())
}
