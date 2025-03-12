#![allow(unused)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ethrex::{
    import,
    initializers::{init_blockchain, init_store},
    removedb,
    utils::set_datadir,
    DEFAULT_DATADIR,
};
use ethrex_p2p::network;
use std::{env::set_current_dir, str::FromStr, thread, time::Duration};
use tracing_subscriber::{filter::Directive, EnvFilter, FmtSubscriber};

#[inline]
fn block_import() {
    let log_level = "info";
    let log_filter = EnvFilter::builder()
        .with_default_directive(
            Directive::from_str(log_level).expect("Not supported log level provided"),
        )
        .from_env_lossy();
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(log_filter)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let data_dir = DEFAULT_DATADIR;
    set_datadir(data_dir);
    removedb::remove_db(&data_dir);

    let evm_engine = "revm".to_owned().try_into().unwrap();

    let network = "../../test_data/genesis-l2.json";

    import::import_blocks_from_datadir(
        data_dir.to_owned(),
        evm_engine,
        &network,
        "../../test_data/2000-blocks.rlp",
    );
}

pub fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Block import");
    group.sample_size(10);
    group.bench_function("Block import", |b| b.iter(|| black_box(block_import())));
    group.finish();
}

criterion_group!(runner, criterion_benchmark);
criterion_main!(runner);
