use ef_tests_blockchain::{
    network::Network,
    test_runner::{parse_test_file, run_ef_test},
};
use ethrex_vm::{backends::EVM, EVM_BACKEND};
use std::path::Path;

#[cfg(feature = "levm")]
pub static VM: EVM = EVM::LEVM;
#[cfg(not(feature = "levm"))]
pub static VM: EVM = EVM::REVM;

fn parse_and_execute(path: &Path) -> datatest_stable::Result<()> {
    EVM_BACKEND.get_or_init(|| VM.clone());

    let tests = parse_test_file(path);

    for (test_key, test) in tests {
        if test.network < Network::Merge || test.network >= Network::Prague {
            // Discard this test
            continue;
        }

        run_ef_test(&test_key, &test);
    }
    Ok(())
}

datatest_stable::harness!(parse_and_execute, "vectors/shanghai/", r".*/.*/.*\.json");
