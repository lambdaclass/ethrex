use crate::runner_v2::{
    error::RunnerError,
    types::{Test, Tests},
};

use clap::Parser;

/// Command line flags for runner execution.
#[derive(Parser, Debug)]
pub struct RunnerOptions {
    /// For running tests in a specific path (could be either a directory or a .json)
    #[arg(
        short,
        long,
        value_name = "PATH",
        value_delimiter = ',',
        default_value = "./vectors"
    )]
    pub path: String,
    /// For running tests in specific .json files. If this is not empty, "path" flag will be ignored.
    #[arg(short, long, value_name = "JSON_FILES", value_delimiter = ',')]
    pub json_files: Vec<String>,
    /// For skipping certain .json files
    #[arg(long, value_name = "SKIP_FILES", value_delimiter = ',')]
    pub skip_files: Vec<String>,
}

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
pub fn parse_file(path: &String) -> Result<Vec<Test>, RunnerError> {
    println!("Parsing file: {:?}", path);
    let test_file = std::fs::File::open(path.clone()).unwrap();
    let mut tests: Tests = serde_json::from_reader(test_file).unwrap();
    for test in tests.0.iter_mut() {
        test.path = path.clone();
    }
    Ok(tests.0)
}

/// Parse a directory of tests into a Vec<Test>.
pub fn parse_dir(path: &String, skipped: &Vec<String>) -> Result<Vec<Test>, RunnerError> {
    println!("Parsing test directory: {:?}", path);
    let mut tests = Vec::new();
    let dir_entries = std::fs::read_dir(path.clone()).unwrap().flatten();

    // For each entry in the directory check if it is a .json file or a directory as well.
    for entry in dir_entries {
        // Check entry type
        let entry_type = entry.file_type().unwrap();
        if entry_type.is_dir() {
            let dir_tests = parse_dir(&String::from(entry.path().to_str().unwrap()), skipped)?;
            tests.push(dir_tests);
        } else {
            // Verify it is a `.json` file, ignore files with different extensions.
            let is_json_file = entry.path().extension().is_some_and(|ext| ext == "json");
            // Verify it is not supposed to be ignored.
            let is_not_skipped = !skipped.contains(&String::from(
                entry.path().file_name().unwrap().to_str().unwrap(),
            ));
            // Parse if it meets requirements.
            if is_json_file && is_not_skipped {
                let file_tests = parse_file(&String::from(entry.path().to_str().unwrap()))?;
                tests.push(file_tests);
            }
        }
    }
    // Up to this point the parsing of every .json file has given a Vec<Test> as a result, so we have to concat
    // to obtain a single Vec<Test> from the Vec<Vec<Test>>.
    Ok(tests.concat())
}

/// Initiates the parser with the corresponding option flags.
pub fn parse_tests(options: &mut RunnerOptions) -> Result<Vec<Test>, RunnerError> {
    let mut tests = Vec::new();
    let mut skipped: Vec<String> = IGNORED_TESTS.iter().map(|test| test.to_string()).collect();
    // Append always ignored tests with user's desired ignored tests.
    skipped.append(&mut options.skip_files);

    // If the user selected specific `.json` files to be executed, parse only those files.
    if !options.json_files.is_empty() {
        for file in &options.json_files {
            if skipped.contains(file) {
                continue;
            }
            let file_tests = parse_file(file)?;
            tests.push(file_tests);
        }
    }
    // If no files were specified, use the path set in the `path` field as the starting
    // point. When user sets nothing it will be the "./vectors" directory by default. 
    else if options.path.ends_with(".json") {
        let file_tests = parse_file(&options.path)?;
        tests.push(file_tests);
    } else {
        let dir_tests = parse_dir(&options.path, &skipped)?;
        tests.push(dir_tests);
    }
    Ok(tests.concat())
}
