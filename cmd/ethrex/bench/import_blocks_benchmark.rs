use criterion::{Criterion, criterion_group, criterion_main};
use ethrex::{
    cli::{import_blocks, remove_db},
    utils::{default_datadir, init_datadir},
};
use ethrex_blockchain::BlockchainType;
use ethrex_config::networks::Network;

#[inline]
fn block_import() {
    let datadir = default_datadir();
    init_datadir(&datadir);
    remove_db(&datadir, true);

    let blockchain_type = BlockchainType::default(); // TODO: Should we support L2?

    let network = Network::from("../../fixtures/genesis/perf-ci.json");
    let genesis = network
        .get_genesis()
        .expect("Failed to generate genesis from file");
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(import_blocks(
        "../../fixtures/blockchain/l2-1k-erc20.rlp",
        &datadir,
        genesis,
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
