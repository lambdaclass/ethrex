use std::path::Path;

use criterion::{criterion_group, criterion_main, Criterion};

use ef_tests_blockchain::{
    test_runner::{build_store_for_test, execute_blocks, parse_test_file},
    types::BlockWithRLP,
};
use ethrex_blockchain::Blockchain;
use ethrex_storage::Store;

static LARGE_TEST: (&str, &str) = ("vectors/prague/eip2935_historical_block_hashes_from_state/block_hashes/block_hashes_history.json", "tests/prague/eip2935_historical_block_hashes_from_state/test_block_hashes.py::test_block_hashes_history[fork_Prague-blockchain_test-full_history_plus_one_check_blockhash_first]");

static LARGE_TEST_2: (&str, &str) = ("vectors/prague/eip2935_historical_block_hashes_from_state/block_hashes/block_hashes_history_at_transition.json", "tests/prague/eip2935_historical_block_hashes_from_state/test_block_hashes.py::test_block_hashes_history_at_transition[fork_CancunToPragueAtTime15k-blockchain_test-blocks_before_fork_1-blocks_after_fork_257]");

pub fn setup_benchmark() -> (Blockchain, Vec<BlockWithRLP>, Store) {
    let tests = parse_test_file(Path::new(LARGE_TEST_2.0));
    let test = tests.get(LARGE_TEST_2.1).unwrap();

    let store = build_store_for_test(&test);

    let mut blocks = test.blocks.clone();

    // print blocks length and total tx length
    println!("blocks length: {}", blocks.len());
    let total_txs: usize = blocks
        .iter()
        .map(|b| b.block().unwrap().transactions.len())
        .sum();
    println!("total txs: {}", total_txs);
    // print avg tx per block
    println!("avg tx per block: {}", total_txs / blocks.len());

    let last_blocks = blocks.split_off(blocks.len() - 100);
    let first_blocks = blocks;

    let blockchain = Blockchain::default_with_store(store.clone());
    execute_blocks(&blockchain, &first_blocks, store.clone());

    (blockchain, last_blocks, store)
}

pub fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("benches");
    group.sample_size(10);

    group.bench_function("import blocks", |b| {
        b.iter_with_setup(
            || setup_benchmark(),
            |(blockchain, blocks, store)| {
                execute_blocks(&blockchain, &blocks, store.clone());
            },
        );
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
