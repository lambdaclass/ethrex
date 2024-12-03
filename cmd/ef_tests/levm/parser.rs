use crate::{
    report::format_duration_as_mm_ss,
    runner::{spinner_success_or_print, spinner_update_text_or_print, EFTestRunnerOptions},
    types::{EFTest, EFTests},
};
use colored::Colorize;
use spinoff::{spinners::Dots, Color, Spinner};
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

pub fn parse_ef_tests(opts: &EFTestRunnerOptions) -> Result<Vec<EFTest>, EFTestParseError> {
    let parsing_time = std::time::Instant::now();
    let cargo_manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ef_general_state_tests_path = cargo_manifest_dir.join("vectors/GeneralStateTests");
    let mut spinner = Spinner::new(Dots, "Parsing EF Tests".bold().to_string(), Color::Cyan);
    if opts.disable_spinner {
        spinner.stop();
    }
    let mut tests = Vec::new();
    for test_dir in std::fs::read_dir(ef_general_state_tests_path.clone())
        .map_err(|err| {
            EFTestParseError::FailedToReadDirectory(format!(
                "{:?}: {err}",
                ef_general_state_tests_path.file_name()
            ))
        })?
        .flatten()
    {
        let directory_tests = parse_ef_test_dir(test_dir, opts, &mut spinner)?;
        tests.extend(directory_tests);
    }
    spinner_success_or_print(
        &mut spinner,
        format!(
            "Parsed EF Tests in {}",
            format_duration_as_mm_ss(parsing_time.elapsed())
        ),
        opts.disable_spinner,
    );
    Ok(tests)
}

pub fn parse_ef_test_dir(
    test_dir: DirEntry,
    opts: &EFTestRunnerOptions,
    directory_parsing_spinner: &mut Spinner,
) -> Result<Vec<EFTest>, EFTestParseError> {
    spinner_update_text_or_print(
        directory_parsing_spinner,
        format!("Parsing directory {:?}", test_dir.file_name()),
        opts.disable_spinner,
    );

    let mut directory_tests = Vec::new();
    for test in std::fs::read_dir(test_dir.path())
        .map_err(|err| {
            EFTestParseError::FailedToReadDirectory(format!("{:?}: {err}", test_dir.file_name()))
        })?
        .flatten()
    {
        if test
            .file_type()
            .map_err(|err| {
                EFTestParseError::FailedToGetFileType(format!("{:?}: {err}", test.file_name()))
            })?
            .is_dir()
        {
            let sub_directory_tests = parse_ef_test_dir(test, opts, directory_parsing_spinner)?;
            directory_tests.extend(sub_directory_tests);
            continue;
        }
        // Skip non-JSON files.
        if test.path().extension().is_some_and(|ext| ext != "json")
            | test.path().extension().is_none()
        {
            continue;
        }
        // Skip the ValueOverflowParis.json file because of errors, and loopMul.json because it takes too long to run.
        if test
            .path()
            .file_name()
            .is_some_and(|name| name == "ValueOverflowParis.json" || name == "loopMul.json")
        {
            continue;
        }

        // Skip tests that are not in the list of tests to run.
        if !opts.tests.is_empty()
            && !opts
                .tests
                .contains(&test_dir.file_name().to_str().unwrap().to_owned())
        {
            spinner_update_text_or_print(
                directory_parsing_spinner,
                format!(
                    "Skipping test {:?} as it is not in the list of tests to run",
                    test.path().file_name()
                ),
                opts.disable_spinner,
            );
            return Ok(Vec::new());
        }

        // Skips all tests in a particular directory.
        if opts
            .skip
            .contains(&test_dir.file_name().to_str().unwrap().to_owned())
        {
            spinner_update_text_or_print(
                directory_parsing_spinner,
                format!(
                    "Skipping test {:?} as it is in the folder of tests to skip",
                    test.path().file_name()
                ),
                opts.disable_spinner,
            );
            continue;
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
            spinner_update_text_or_print(
                directory_parsing_spinner,
                format!(
                    "Skipping test {:?} as it is in the list of tests to skip",
                    test.path().file_name()
                ),
                opts.disable_spinner,
            );
            continue;
        }

        let test_file = std::fs::File::open(test.path()).map_err(|err| {
            EFTestParseError::FailedToReadFile(format!("{:?}: {err}", test.path()))
        })?;
        let mut tests: EFTests = serde_json::from_reader(test_file).map_err(|err| {
            EFTestParseError::FailedToParseTestFile(format!("{:?} parse error: {err}", test.path()))
        })?;
        for test in tests.0.iter_mut() {
            test.dir = test_dir.file_name().into_string().unwrap();
        }
        directory_tests.extend(tests.0);
    }
    Ok(directory_tests)
}
