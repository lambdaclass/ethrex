use std::path::Path;

use ef_tests_ethrex::{
    network::Network,
    test_runner::{parse_test_file, run_ef_test},
};

#[allow(dead_code)]
fn parse_and_execute(path: &Path) -> datatest_stable::Result<()> {
    let tests = parse_test_file(path);

    for (test_key, test) in tests {
        if test.network < Network::Merge {
            // Discard this test
            continue;
        }

        run_ef_test(&test_key, &test);
    }
    Ok(())
}

// TODO: Delete main function and uncomment the following line to allow prague tests to be parsed

// datatest_stable::harness!(parse_and_execute, "vectors/prague/", r".*/.*/.*\.json");

fn main() {
    //Do nothing
}
