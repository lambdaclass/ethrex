use crate::runner_v2::types::{Test, Tests};

pub fn parse_file() -> Vec<Test> {
    // Hardcode test file path for testing purposes for now
    let test_path = "./runner_v2/test_files/gas.json";
    let test_file = std::fs::File::open(test_path).unwrap();
    let tests: Tests = serde_json::from_reader(test_file).unwrap();
    println!("Tests: {}", tests);
    tests.0
}
