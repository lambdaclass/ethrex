use std::path::PathBuf;

use crate::runner_v2::{
    error::RunnerError,
    types::{Test, Tests},
};

/// Parse a `.json` file of tests into a Vec<Test>.
pub fn parse_file(path: PathBuf) -> Result<Vec<Test>, RunnerError> {
    println!("Parsing test file: {:?}", path); 
    let test_file = std::fs::File::open(path.clone())
        .map_err(|err| RunnerError::FailedToOpenFile(err.to_string()))?;
    let mut tests: Tests = serde_json::from_reader(test_file)
        .map_err(|err| RunnerError::FailedToParseTestFile(path.clone(), err.to_string()))?;
    for test in tests.0.iter_mut() {
        test.path = String::from(path.to_str().ok_or(RunnerError::FailedToConvertPath)?);
    }
    Ok(tests.0)
}

/// Parse a directory of tests into a Vec<Test>.
pub fn parse_dir(path: PathBuf) -> Result<Vec<Test>, RunnerError> {
    let mut tests = Vec::new();
    let dir_entries = std::fs::read_dir(path.clone())
        .map_err(|err| RunnerError::FailedToReadDirectory(path, err.to_string()))?
        .flatten();

    // For each entry in the directory check if it is a .json file or a directory as well.
    for entry in dir_entries {
        // Check entry type
        let entry_type = entry
            .file_type()
            .map_err(|err| RunnerError::FailedToGetFileType(err.to_string()))?;
        if entry_type.is_dir() {
            let dir_tests = parse_dir(entry.path())?;
            tests.push(dir_tests);
        } else if entry.path().extension().is_some_and(|ext| ext == "json"){
            let file_tests = parse_file(entry.path())?;
            tests.push(file_tests);
        }
    }
    // Up to this point the parsing of every .json file has given a Vec<Test> as a result, so we have to concat
    // to obtain a single Vec<Test> from the Vec<Vec<Test>>.
    Ok(tests.concat())
}
