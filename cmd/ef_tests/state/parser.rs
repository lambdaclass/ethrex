use crate::{
    report::format_duration_as_mm_ss,
    runner::EFTestRunnerOptions,
    types::{EFTest, EFTests},
};
use colored::Colorize;
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};
use std::fs::DirEntry;

#[derive(Debug, thiserror::Error)]
pub enum EFTestParseError {
    #[error("Failed to read directory: {0}")]
    FailedToReadDirectory(String),
    #[error("Failed to read file: {0}")]
    FailedToReadFile(String),
    #[error("Failed to get file type: {0}")]
    FailedToGetFileType(String),
    #[error("Failed to parse test file: {0}")]
    FailedToParseTestFile(String),
}

const IGNORED_TESTS: [&str; 11] = [
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
];

// One .json can have multiple tests, sometimes we want to skip one of those.
pub const SPECIFIC_IGNORED_TESTS: [&str; 1] = [
    "test_set_code_to_non_empty_storage[fork_Prague-state_test-zero_nonce]", // Skip because EIP-7702 has changed. See https://github.com/ethereum/EIPs/pull/9710
];

pub fn parse_ef_tests(opts: &EFTestRunnerOptions) -> Result<Vec<EFTest>, EFTestParseError> {
    let parsing_time = std::time::Instant::now();
    let cargo_manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ef_general_state_tests_path = cargo_manifest_dir.join("vectors");
    println!("{}", "Parsing EF Tests".bold().cyan());

    let dir: Vec<_> = std::fs::read_dir(ef_general_state_tests_path.clone())
        .map_err(|err| {
            EFTestParseError::FailedToReadDirectory(format!(
                "{:?}: {err}",
                ef_general_state_tests_path.file_name()
            ))
        })?
        .flatten()
        .collect();

    let mut directory_tests_results = Vec::new();
    dir.into_par_iter()
        .map(|test: DirEntry| -> Result<Vec<EFTest>, EFTestParseError> {
            parse_ef_test_dir(test, opts)
        })
        .collect_into_vec(&mut directory_tests_results);

    let tests = directory_tests_results
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    println!(
        "Parsed EF Tests in {}",
        format_duration_as_mm_ss(parsing_time.elapsed())
    );

    Ok(tests.into_iter().flatten().collect())
}

pub fn parse_ef_test_dir(
    test_dir: DirEntry,
    opts: &EFTestRunnerOptions,
) -> Result<Vec<EFTest>, EFTestParseError> {
    println!("Parsing directory {:?}", test_dir.file_name());

    let mut directory_tests_results = Vec::new();

    let dir: Vec<_> = std::fs::read_dir(test_dir.path())
        .map_err(|err| {
            EFTestParseError::FailedToReadDirectory(format!("{:?}: {err}", test_dir.file_name()))
        })?
        .flatten()
        .collect();

    dir.into_par_iter()
        .map(
            |test: DirEntry| -> Result<Option<Vec<EFTest>>, EFTestParseError> {
                if test
                    .file_type()
                    .map_err(|err| {
                        EFTestParseError::FailedToGetFileType(format!(
                            "{:?}: {err}",
                            test.file_name()
                        ))
                    })?
                    .is_dir()
                {
                    let sub_directory_tests = parse_ef_test_dir(test, opts)?;
                    return Ok(Some(sub_directory_tests));
                }
                // Skip non-JSON files.
                if test.path().extension().is_some_and(|ext| ext != "json")
                    | test.path().extension().is_none()
                {
                    return Ok(None);
                }
                // Skip ignored tests
                if test
                    .path()
                    .file_name()
                    .is_some_and(|name| IGNORED_TESTS.contains(&name.to_str().unwrap_or("")))
                {
                    return Ok(None);
                }

                // Skip tests that are not in the list of tests to run.
                if !opts.tests.is_empty()
                    && !opts
                        .tests
                        .contains(&test_dir.file_name().to_str().unwrap().to_owned())
                    && !opts.tests.contains(
                        &test
                            .path()
                            .file_name()
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .to_owned(),
                    )
                {
                    return Ok(None);
                }

                // Skips all tests in a particular directory.
                if opts
                    .skip
                    .contains(&test_dir.file_name().to_str().unwrap().to_owned())
                {
                    println!(
                        "Skipping test {:?} as it is in the folder of tests to skip",
                        test.path().file_name().unwrap()
                    );
                    return Ok(None);
                }

                // Skip tests by name (with .json extension)
                if opts.skip.contains(
                    &test
                        .path()
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                ) {
                    println!(
                        "Skipping test file {:?} as it is in the list of tests to skip",
                        test.path().file_name().unwrap()
                    );
                    return Ok(None);
                }

                let test_file = std::fs::File::open(test.path()).map_err(|err| {
                    EFTestParseError::FailedToReadFile(format!("{:?}: {err}", test.path()))
                })?;
                let mut tests: EFTests = serde_json::from_reader(test_file).map_err(|err| {
                    EFTestParseError::FailedToParseTestFile(format!(
                        "{:?} parse error: {err}",
                        test.path()
                    ))
                })?;
                for test in tests.0.iter_mut() {
                    test.dir = test_dir.file_name().into_string().unwrap();
                }

                // We only want to include tests that have post states from the specified forks in EFTestsRunnerOptions.
                if let Some(forks) = &opts.forks {
                    for test in tests.0.iter_mut() {
                        let test_forks_numbers: Vec<u8> =
                            forks.iter().map(|fork| *fork as u8).collect();

                        test.post.forks = test
                            .post
                            .forks
                            .iter()
                            .filter(|a| test_forks_numbers.contains(&(*a.0 as u8)))
                            .map(|(k, v)| (*k, v.clone()))
                            .collect();
                    }

                    tests.0.retain(|test| !test.post.forks.is_empty());
                }

                Ok(Some(tests.0))
            },
        )
        .collect_into_vec(&mut directory_tests_results);

    let directory_tests = directory_tests_results
        .into_iter()
        .filter_map(|x| x.transpose())
        .collect::<Result<Vec<_>, _>>()?;

    Ok(directory_tests.into_iter().flatten().collect())
}
