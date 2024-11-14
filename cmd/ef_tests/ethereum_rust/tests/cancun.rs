use std::path::Path;

use ef_tests_ethereum_rust::test_runner::{parse_test_file, run_ef_test};

fn parse_and_execute(path: &Path) -> datatest_stable::Result<()> {
    let tests = parse_test_file(path);

    for (test_key, test) in tests {
        run_ef_test(&test_key, &test);
    }
    Ok(())
}

datatest_stable::harness!(
    parse_and_execute,
    "vectors/cancun/",
    r"eip1153_tstore/.*/.*\.json",
    parse_and_execute,
    "vectors/cancun/",
    r"eip4788_beacon_root/.*/.*\.json",
    parse_and_execute,
    "vectors/cancun/",
    r"eip5656_mcopy/.*/.*\.json",
    parse_and_execute,
    "vectors/cancun/",
    r"eip7516_blobgasfee/.*/.*\.json",
    parse_and_execute,
    "vectors/cancun/",
    r"eip6780_selfdestruct/.*/.*\.json",
    parse_and_execute,
    "vectors/cancun/",
    r"eip4844_blobs/.*/.*\.json",
);
