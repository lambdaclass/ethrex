use ef_tests_blockchain::{
    network::Network,
    test_runner::{parse_test_file, run_ef_test},
};
use std::path::Path;

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
