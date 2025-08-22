use std::path::PathBuf;

use crate::modules::{
    error::RunnerError,
    types::{Test, Tests},
};

use clap::Parser;

/// Command line flags for runner execution.
#[derive(Parser, Debug)]
pub struct RunnerOptions {
    /// For running tests in a specific file (could be either a directory or a .json)
    //TODO: Change default path to ./vectors when the other EFTests are replaced by this runner
    #[arg(short, long, value_name = "PATH", default_value = "../state/vectors")]
    pub path: PathBuf,
    /// For running tests in specific .json files. If this is not empty, "path" flag will be ignored.
    #[arg(short, long, value_name = "JSON_FILES", value_delimiter = ',')]
    pub json_files: Vec<PathBuf>,
    #[arg(long, value_name = "sp1")] //TODO: Implement integration with SP1.
    pub sp1: bool,
}

//TODO: Use this constant, improve it.
const IGNORED_TESTS: [&str; 12] = [
    "static_Call50000_sha256.json", // Skip because it takes longer to run than some tests, but not a huge deal.
    "CALLBlake2f_MaxRounds.json",   // Skip because it takes extremely long to run, but passes.
    "ValueOverflow.json",           // Skip because it tries to deserialize number > U256::MAX
    "ValueOverflowParis.json",      // Skip because it tries to deserialize number > U256::MAX
    "loopMul.json",                 // Skip because it takes too long to run
    "dynamicAccountOverwriteEmpty_Paris.json",
    "RevertInCreateInInitCreate2Paris.json",
    "RevertInCreateInInit_Paris.json",
    "create2collisionStorageParis.json",
    "InitCollisionParis.json",
    "InitCollision.json",
    "contract_create.json", // Skip for now as it requires special transaction type handling
];

/// Parse a `.json` file of tests into a Vec<Test>.
pub fn parse_file(path: &PathBuf) -> Result<Vec<Test>, RunnerError> {
    println!("Parsing file: {:?}", path);
    let test_file = std::fs::File::open(path.clone()).unwrap();
    let mut tests: Tests = serde_json::from_reader(test_file).unwrap();
    for test in tests.0.iter_mut() {
        test.path = path.clone();
    }
    Ok(tests.0)
}

/// Parse a directory of tests into a Vec<Test>.
pub fn parse_dir(
    path: &PathBuf,
    skipped_files: &[PathBuf],
    only_files: &Vec<PathBuf>,
) -> Result<Vec<Test>, RunnerError> {
    let mut tests = Vec::new();
    let dir_entries = std::fs::read_dir(path.clone()).unwrap().flatten();

    // For each entry in the directory check if it is a .json file or a directory as well.
    for entry in dir_entries {
        // Check entry type
        let entry_type = entry.file_type().unwrap();
        if entry_type.is_dir() {
            let dir_tests = parse_dir(&entry.path(), skipped_files, only_files)?;
            tests.push(dir_tests);
        } else {
            let file_name = PathBuf::from(entry.file_name().as_os_str());
            // If only certain files were supposed to be parsed make sure this file is among them.
            if !only_files.is_empty() && !only_files.contains(&file_name) {
                continue;
            }

            // Normally this would be true but it's safe to make this check.
            let is_json_file = entry.path().extension().is_some_and(|ext| ext == "json");
            let is_not_skipped = !skipped_files.contains(&file_name);
            if is_json_file && is_not_skipped {
                let file_tests = parse_file(&entry.path())?;
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
    if options.path.to_string_lossy().ends_with(".json") {
        parse_file(&options.path)
    } else {
        let skipped = IGNORED_TESTS.map(PathBuf::from);
        parse_dir(&options.path, &skipped, &options.json_files)
    }
}
