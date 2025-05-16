use criterion::{criterion_group, criterion_main, Criterion};
use ethrex::{
    cli::{import_blocks, remove_db},
    utils::set_datadir,
    DEFAULT_DATADIR,
};
use ethrex_vm::EvmEngine;

#[inline]
fn block_import() {
    let data_dir = DEFAULT_DATADIR;

    let network = "../../test_data/genesis-perf-ci.json";

    set_datadir(data_dir, &Some(network.to_string()));
    remove_db(data_dir, true, &Some(network.to_string()));

    let evm_engine = EvmEngine::default();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(import_blocks(
        "../../test_data/l2-1k-erc20.rlp",
        data_dir,
        network,
        evm_engine,
    ));
}

pub fn import_blocks_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Block import");
    group.sample_size(10);
    group.bench_function("Block import ERC20 transfers", |b| b.iter(block_import));
    group.finish();
}

criterion_group!(runner, import_blocks_benchmark);
criterion_main!(runner);
