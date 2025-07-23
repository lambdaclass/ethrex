use std::{collections::HashSet, path::PathBuf};

use crate::runner_v2::{
    error::RunnerError,
    types::{Test, TestCase, Tests},
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
pub fn parse_file(
    path: PathBuf,
    test_cases: &mut HashSet<TestCase>,
    repeated_test_cases: &mut usize,
) -> Result<Vec<Test>, RunnerError> {
    let test_file = std::fs::File::open(path.clone()).unwrap();
    let mut tests: Tests = serde_json::from_reader(test_file).unwrap();
    for test in tests.0.iter_mut() {
        test.path = String::from(path.to_str().unwrap());
        for test_case in &test.test_cases {
            //println!("Inserting - name {:?} - path {:?} - fork {:?} - post hash {:?}", test.name, test.path, test_case.fork, test_case.post.hash);
            let inserted = test_cases.insert(test_case.clone());
            if !inserted {
                *repeated_test_cases += 1;
                // println!(
                //     "Found repeated test case, test name {:?}, path {:?}, fork: {:?}, post hash: {:?}",
                //     test.name, test.path, test_case.fork, test_case.post.hash
                // );
            }
        }
    }
    Ok(tests.0)
}

/// Parse a directory of tests into a Vec<Test>.
pub fn parse_dir(path: PathBuf, repeated_test_cases: &mut usize) -> Result<Vec<Test>, RunnerError> {
    // println!("Parsing test directory: {:?}", path);
    let mut tests = Vec::new();
    let mut test_cases: HashSet<TestCase> = HashSet::new();
    let dir_entries = std::fs::read_dir(path.clone()).unwrap().flatten();

    // For each entry in the directory check if it is a .json file or a directory as well.
    for entry in dir_entries {
        // Check entry type
        let entry_type = entry.file_type().unwrap();
        if entry_type.is_dir() {
            let dir_tests = parse_dir(entry.path(), repeated_test_cases)?;
            tests.push(dir_tests);
        } else {
            let is_json_file = entry.path().extension().is_some_and(|ext| ext == "json");
            let is_not_skipped =
                !IGNORED_TESTS.contains(&entry.path().file_name().unwrap().to_str().unwrap());
            if is_json_file && is_not_skipped {
                let file_tests = parse_file(entry.path(), &mut test_cases, repeated_test_cases)?;
                tests.push(file_tests);
            }
        }
    }
    // Up to this point the parsing of every .json file has given a Vec<Test> as a result, so we have to concat
    // to obtain a single Vec<Test> from the Vec<Vec<Test>>.
    Ok(tests.concat())
}
