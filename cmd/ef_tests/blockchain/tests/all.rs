use ef_tests_blockchain::test_runner::parse_and_execute;
use ethrex_vm::EvmEngine;
use std::path::Path;

const TEST_FOLDER: &str = "vectors/";

fn test_runner(path: &Path) -> datatest_stable::Result<()> {
    let engine = if cfg!(not(any(feature = "sp1", feature = "levm"))) {
        EvmEngine::REVM
    } else {
        EvmEngine::LEVM
    };

    #[cfg(feature = "levm")]
    let backend = Some(ethrex_prover_lib::backends::Backend::Exec);
    #[cfg(feature = "sp1")]
    let backend = Some(ethrex_prover_lib::backends::Backend::SP1);
    #[cfg(not(any(feature = "sp1", feature = "levm")))]
    let backend = None;

    parse_and_execute(path, engine, None, backend)
}

datatest_stable::harness!(test_runner, TEST_FOLDER, r".*");
