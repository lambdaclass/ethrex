use criterion::{Criterion, criterion_group, criterion_main};
use ethrex::{
    DEFAULT_DATADIR,
    cli::{import_blocks, remove_db},
    networks::Network,
    utils::set_datadir,
};
use ethrex_vm::EvmEngine;
use std::path::Path;

#[inline]
fn block_import() {
    let temp_datadir_path = Path::new(DEFAULT_DATADIR);
    let data_dir_actual = set_datadir(Some(temp_datadir_path), None);
    remove_db(&data_dir_actual, true);

    let evm_engine = EvmEngine::default();

    let network = Network::from("../../test_data/genesis-perf-ci.json");
    let genesis = network
        .get_genesis()
        .expect("Failed to generate genesis from file");
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(import_blocks(
        Path::new("../../test_data/l2-1k-erc20.rlp"), // Assuming import_blocks also takes Path
        &data_dir_actual,
        genesis,
        evm_engine,
    ))
    .expect("Failed to import blocks on the Tokio runtime");
}

pub fn import_blocks_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Block import");
    group.sample_size(10);
    group.bench_function("Block import ERC20 transfers", |b| b.iter(block_import));
    group.finish();
}

criterion_group!(runner, import_blocks_benchmark);
criterion_main!(runner);
