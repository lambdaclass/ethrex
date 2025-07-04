use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_vm::EvmEngine;
use std::path::Path;

const TEST_FOLDER: &str = "vectors/";

fn parse_and_execute_runner(path: &Path) -> datatest_stable::Result<()> {
    let engine = if cfg!(feature = "revm") {
        EvmEngine::REVM
    } else {
        EvmEngine::LEVM
    };

    parse_and_execute(path, engine, None)
}

datatest_stable::harness!(parse_and_execute_runner, TEST_FOLDER, r".*",);
