use crate::{
    report::format_duration_as_mm_ss,
    runner::EFTestRunnerOptions,
    types::{EFTest, EFTests},
    utils::{spinner_success_or_print, spinner_update_text_or_print},
};
use colored::Colorize;
use spinoff::{spinners::Dots, Color, Spinner};
use std::{fs::DirEntry, path::PathBuf};

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

const IGNORED_TESTS: [&str; 7] = [
    "ValueOverflowParis.json",                 // Skip because of errors
    "loopMul.json",                            // Skip because it takes too long to run
    "dynamicAccountOverwriteEmpty_Paris.json", // Skip because it fails on REVM
    "RevertInCreateInInitCreate2Paris.json", // Skip because it fails on REVM. See https://github.com/lambdaclass/ethrex/issues/1555
    "RevertInCreateInInit_Paris.json", // Skip because it fails on REVM. See https://github.com/lambdaclass/ethrex/issues/1555
    "create2collisionStorageParis.json", // Skip because it fails on REVM
    "InitCollisionParis.json",         // Skip because it fails on REVM
];

pub fn parse_ef_tests(opts: &EFTestRunnerOptions) -> Result<Vec<EFTest>, EFTestParseError> {
    let parsing_time = std::time::Instant::now();
    let cargo_manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let general_tests_path = cargo_manifest_dir.join("vectors/GeneralStateTests");
    let pectra_tests_path = cargo_manifest_dir.join("vectors/stEIP2537");

    let mut spinner = Spinner::new(Dots, "Parsing EF Tests".bold().to_string(), Color::Cyan);
    if !opts.spinner {
        spinner.stop();
    }

    let mut tests = parse_ef_test_dir(general_tests_path, opts, &mut spinner, true)?;
    let pectra_tests = parse_ef_test_dir(pectra_tests_path, opts, &mut spinner, false)?;
    tests.extend(pectra_tests);

    spinner_success_or_print(
        &mut spinner,
        format!(
            "Parsed EF Tests in {}",
            format_duration_as_mm_ss(parsing_time.elapsed())
        ),
        opts.spinner,
    );

    Ok(tests)
}

pub fn parse_ef_test_dir(
    path: PathBuf,
    opts: &EFTestRunnerOptions,
    spinner: &mut Spinner,
    recursive: bool,
) -> Result<Vec<EFTest>, EFTestParseError> {
    spinner_update_text_or_print(
        spinner,
        format!("Parsing directory {:?}", path.file_name().unwrap()),
        opts.spinner,
    );

    let mut directory_tests = Vec::new();

    for entry in std::fs::read_dir(&path)
        .map_err(|err| EFTestParseError::FailedToReadDirectory(format!("{:?}: {err}", path)))?
        .flatten()
    {
        // If the entry is a directory, parse it recursively
        if entry
            .file_type()
            .map_err(|err| {
                EFTestParseError::FailedToGetFileType(format!("{:?}: {err}", entry.file_name()))
            })?
            .is_dir()
        {
            if recursive {
                let sub_tests = parse_ef_test_dir(entry.path(), opts, spinner, recursive)?;
                directory_tests.extend(sub_tests);
            }
            continue;
        }

        if let Some(mut tests) = process_test_file(&entry, opts, spinner)? {
            for test in tests.0.iter_mut() {
                test.dir = path.file_name().unwrap().to_str().unwrap().to_owned();
            }
            directory_tests.extend(tests.0);
        }
    }

    Ok(directory_tests)
}

fn process_test_file(
    test: &DirEntry,
    opts: &EFTestRunnerOptions,
    spinner: &mut Spinner,
) -> Result<Option<EFTests>, EFTestParseError> {
    // Skip non-JSON files
    if test.path().extension().is_some_and(|ext| ext != "json") || test.path().extension().is_none()
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

    // Skip tests not in the list to run
    if !opts.tests.is_empty()
        && !opts
            .tests
            .contains(&test.file_name().to_str().unwrap().to_owned())
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

    // Skip tests in the skip list
    if opts.skip.contains(
        &test
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
    ) {
        spinner_update_text_or_print(
            spinner,
            format!(
                "Skipping test {:?} as it is in the skip list",
                test.path().file_name().unwrap()
            ),
            opts.spinner,
        );
        return Ok(None);
    }

    // Open and parse the JSON file
    let test_file = std::fs::File::open(test.path())
        .map_err(|err| EFTestParseError::FailedToReadFile(format!("{:?}: {err}", test.path())))?;
    let tests: EFTests = serde_json::from_reader(test_file).map_err(|err| {
        EFTestParseError::FailedToParseTestFile(format!("{:?} parse error: {err}", test.path()))
    })?;

    Ok(Some(tests))
}
