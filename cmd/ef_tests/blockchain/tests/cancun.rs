use ef_tests_blockchain::{
    network::Network,
    test_runner::{parse_test_file, run_ef_test},
};
use std::path::Path;

// NOTE: There are many tests which are failing due to the usage of Prague fork.
// These tests are distributed in almost all json test files.
// The `parse_and_execute_until_cancun` function will filter those tests after parsing them
// this will mark said tests as passed, so they will become a false positive.
// The idea is to move those tests to be executed with the `parse_and_execute_all` function once
// Prague development starts.
// This modification should be made on the harness down below, matching the regex with the desired
// test or set of tests

#[allow(dead_code)]
fn parse_and_execute(path: &Path) -> datatest_stable::Result<()> {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let tests = parse_test_file(path);

    for (test_key, test) in tests {
        if test.network < Network::Merge {
            // These tests fall into the not supported forks. This produces false positives
            continue;
        }
        rt.block_on(run_ef_test(&test_key, &test));
    }

    Ok(())
}

datatest_stable::harness!(parse_and_execute, "vectors/cancun/", r".*/.*\.json",);
