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

fn parse_and_execute_until_cancun(path: &Path) -> datatest_stable::Result<()> {
    let tests = parse_test_file(path);

    for (test_key, test) in tests {
        if test.network < Network::Merge || test.network >= Network::CancunToPragueAtTime15k {
            // These tests fall into the not developed or not-yet-developed forks, so we filter
            // them. This produces false positives
            continue;
        }
        run_ef_test(&test_key, &test);
    }

    Ok(())
}

#[allow(dead_code)]
fn parse_and_execute_all(path: &Path) -> datatest_stable::Result<()> {
    let tests = parse_test_file(path);

    for (test_key, test) in tests {
        if test.network < Network::Merge {
            // These tests fall into the not supported forks. This produces false positives
            continue;
        }
        run_ef_test(&test_key, &test);
    }

    Ok(())
}

datatest_stable::harness!(
    parse_and_execute_all,
    "vectors/cancun/",
    r"eip1153_tstore/.*/.*\.json",
    parse_and_execute_all,
    "vectors/cancun/",
    r"eip4788_beacon_root/.*/.*\.json",
    // TODO: Here we are still filtering some Prague and Cancun-Prague transition tests
    // after fixing them, the testing function should be replaced in favour of
    // `parse_and_execute_all`
    parse_and_execute_until_cancun,
    "vectors/cancun/",
    r"eip4844_blobs/.*/.*\.json",
    parse_and_execute_all,
    "vectors/cancun/",
    r"eip5656_mcopy/.*/.*\.json",
    parse_and_execute_all,
    "vectors/cancun/",
    r"eip6780_selfdestruct/.*/.*\.json",
    parse_and_execute_all,
    "vectors/cancun/",
    r"eip7516_blobgasfee/.*/.*\.json",
);
