#![allow(unused)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ethrex::{
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

    ethrex::cli::Subcommand::RemoveDB {
        datadir: data_dir.to_owned(),
    }
    .run(&ethrex::cli::Options::default())
    .unwrap();

    let evm_engine = "revm".to_owned().try_into().unwrap();

    let network = "../../test_data/genesis-l2-ci.json";

    ethrex::cli::Subcommand::Import {
        path: "../../test_data/l2-1k-erc20.rlp".to_owned(),
        removedb: false,
    }
    .run(&ethrex::cli::Options {
        datadir: data_dir.to_owned(),
        network: network.to_owned(),
        evm: evm_engine,
        ..Default::default()
    })
    .unwrap();
}

pub fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Block import");
    group.sample_size(10);
    group.bench_function("Block import ERC20 transfers", |b| b.iter(block_import));
    group.finish();
}

criterion_group!(runner, criterion_benchmark);
criterion_main!(runner);
