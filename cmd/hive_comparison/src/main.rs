use std::fs::{self, File};
use std::io::BufReader;

use hive_report::{HiveResult, JsonFile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Clear logs, build image with LEVM and run
    // 2. Store results_by_category in results_levm variable
    // 1. Clear logs, build image with REVM and run
    // 4. Store results_by_category in results_revm variable
    // 5. Compare results_levm with results_revm. (They should have the same tests ran)
    //    For now we can just compare the amount of test passed on each category and see if it is the same.

    // Warning: The code down below is copy-pasted just for testing purposes, progress has not been made yet :)

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

    dbg!(&results);
    let results_by_category = results.chunk_by(|a, b| a.category == b.category);

    dbg!(&results_by_category);

    // for results in results_by_category {
    //     // print category
    //     println!("*{}*", results[0].category);
    //     for result in results {
    //         println!("\t{}", result);
    //     }
    //     println!();
    // }

    Ok(())
}

// fn generate_results_by_category() {

// }
