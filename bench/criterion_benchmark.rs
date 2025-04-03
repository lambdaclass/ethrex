#![allow(unused)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ethrex::{
    cli::{import_blocks, remove_db},
    initializers::{init_blockchain, init_store},
    utils::set_datadir,
    DEFAULT_DATADIR,
};
use ethrex_p2p::network;
use std::{env::set_current_dir, str::FromStr, thread, time::Duration};
use tracing_subscriber::{filter::Directive, EnvFilter, FmtSubscriber};

#[inline]
fn block_import() {
    let data_dir = DEFAULT_DATADIR;
    set_datadir(data_dir);

    remove_db(data_dir, true);

    let evm_engine = "revm".to_owned().try_into().unwrap();

    let network = "../../test_data/genesis-l2-ci.json";

    import_blocks(
        "../../test_data/l2-1k-erc20.rlp",
        data_dir,
        network,
        evm_engine,
    );
}

pub fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Block import");
    group.sample_size(10);
    group.bench_function("Block import ERC20 transfers", |b| b.iter(block_import));
    group.finish();
}

criterion_group!(runner, criterion_benchmark);
criterion_main!(runner);
