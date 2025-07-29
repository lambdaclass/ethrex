use std::path::PathBuf;

use crate::runner_v2::{
    error::RunnerError,
    types::{Test, Tests},
};

const IGNORED_TESTS: [&str; 12] = [
    "static_Call50000_sha256.json", // Skip because it takes longer to run than some tests, but not a huge deal.
    "CALLBlake2f_MaxRounds.json",   // Skip because it takes extremely long to run, but passes.
    "ValueOverflow.json",           // Skip because it tries to deserialize number > U256::MAX
    "ValueOverflowParis.json",      // Skip because it tries to deserialize number > U256::MAX
    "loopMul.json",                 // Skip because it takes too long to run
    "dynamicAccountOverwriteEmpty_Paris.json", // Skip because it fails on REVM
    "RevertInCreateInInitCreate2Paris.json", // Skip because it fails on REVM. See https://github.com/lambdaclass/ethrex/issues/1555
    "RevertInCreateInInit_Paris.json", // Skip because it fails on REVM. See https://github.com/lambdaclass/ethrex/issues/1555
    "create2collisionStorageParis.json", // Skip because it fails on REVM
    "InitCollisionParis.json",         // Skip because it fails on REVM
    "InitCollision.json",              // Skip because it fails on REVM
    "contract_create.json", // Skip for now as it requires special transaction type handling
];

/// Parse a `.json` file of tests into a Vec<Test>.
pub fn parse_file(path: PathBuf) -> Result<Vec<Test>, RunnerError> {
    let test_file = std::fs::File::open(path.clone()).unwrap();
    let mut tests: Tests = serde_json::from_reader(test_file).unwrap();
    for test in tests.0.iter_mut() {
        test.path = String::from(path.to_str().unwrap());
    }
    Ok(tests.0)
}

/// Parse a directory of tests into a Vec<Test>.
pub fn parse_dir(path: PathBuf) -> Result<Vec<Test>, RunnerError> {
    println!("Parsing test directory: {:?}", path);
    let mut tests = Vec::new();
    let dir_entries = std::fs::read_dir(path.clone()).unwrap().flatten();

    // For each entry in the directory check if it is a .json file or a directory as well.
    for entry in dir_entries {
        // Check entry type
        let entry_type = entry.file_type().unwrap();
        if entry_type.is_dir() {
            let dir_tests = parse_dir(entry.path())?;
            tests.push(dir_tests);
        } else {
            let is_json_file = entry.path().extension().is_some_and(|ext| ext == "json");
            let is_not_skipped =
                !IGNORED_TESTS.contains(&entry.path().file_name().unwrap().to_str().unwrap());
            if is_json_file && is_not_skipped {
                let file_tests = parse_file(entry.path())?;
                tests.push(file_tests);
            }
        }
    }
    // Up to this point the parsing of every .json file has given a Vec<Test> as a result, so we have to concat
    // to obtain a single Vec<Test> from the Vec<Vec<Test>>.
    Ok(tests.concat())
}
