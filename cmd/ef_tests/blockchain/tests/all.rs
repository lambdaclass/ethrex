use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_vm::EvmEngine;
use std::path::Path;

const TEST_FOLDER: &str = "vectors/";

//TODO: Fix this test and remove this.
const SKIPPED_TESTS: &[&str] = &["tests/constantinople/eip1014_create2/test_recreate"];

fn parse_and_execute_runner(path: &Path) -> datatest_stable::Result<()> {
    let engine = if cfg!(feature = "levm") {
        EvmEngine::LEVM
    } else {
        EvmEngine::REVM
    };

    parse_and_execute(path, engine, Some(SKIPPED_TESTS))
}

datatest_stable::harness!(parse_and_execute_runner, TEST_FOLDER, r".*",);
