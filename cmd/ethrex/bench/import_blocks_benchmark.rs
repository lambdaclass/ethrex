use criterion::{Criterion, criterion_group, criterion_main};
use ethrex::{
    DEFAULT_DATADIR,
    cli::{import_blocks, remove_db},
    networks::Network,
    utils::set_datadir,
};
use ethrex_blockchain::BlockchainType;
use ethrex_vm::EvmEngine;

#[inline]
fn block_import() {
    let data_dir = DEFAULT_DATADIR;
    set_datadir(data_dir);
    remove_db(data_dir, true);

    let evm_engine = EvmEngine::default();

    let blockchain_type = BlockchainType::default(); // TODO: Should we support L2?

    let network = Network::from("../../fixtures/genesis/perf-ci.json");
    let genesis = network
        .get_genesis()
        .expect("Failed to generate genesis from file");
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(import_blocks(
        "../../fixtures/blockchain/l2-1k-erc20.rlp",
        data_dir,
        genesis,
        evm_engine,
        blockchain_type,
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
