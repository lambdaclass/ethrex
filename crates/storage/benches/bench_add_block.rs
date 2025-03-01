use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ethereum_types::Bloom;
use ethrex_common::{
    types::{Block, BlockBody, BlockHash, BlockHeader, BlockNumber, Genesis, Transaction, TxKind},
    Address, Bytes, H160, H256, U256,
};
use ethrex_storage::{EngineType, Store};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::{fs::File, io::BufReader, time::Duration};
use tempdir::TempDir;

// Helper function to create a random transaction
fn create_random_transaction(seed: u64) -> Transaction {
    let mut rng = StdRng::seed_from_u64(seed);
    Transaction::LegacyTransaction(ethrex_common::types::LegacyTransaction {
        nonce: rng.gen::<u64>(),
        gas_price: rng.gen::<u64>(),
        gas: rng.gen::<u64>(),
        to: if rng.gen_bool(0.2) {
            TxKind::default() // Create transaction
        } else {
            TxKind::Call(H160::random_using(&mut rng))
        },
        value: U256::from(rng.gen::<u64>()),
        data: Bytes::from(
            (0..rng.gen_range(0..1024))
                .map(|_| rng.gen::<u8>())
                .collect::<Vec<_>>(),
        ),
        v: U256::from(rng.gen::<u64>()),
        r: U256::from(rng.gen::<u64>()),
        s: U256::from(rng.gen::<u64>()),
    })
}

// Helper function to create a block with random transactions
fn create_block_with_random_transactions(
    parent_hash: H256,
    block_number: BlockNumber,
    num_transactions: usize,
    seed: u64,
) -> Block {
    let mut rng = StdRng::seed_from_u64(seed);
    let transactions = (0..num_transactions)
        .map(|i| create_random_transaction(seed + i as u64))
        .collect::<Vec<_>>();

    let header = BlockHeader {
        parent_hash,
        ommers_hash: H256::random_using(&mut rng),
        coinbase: Address::random_using(&mut rng),
        state_root: H256::random_using(&mut rng),
        transactions_root: H256::random_using(&mut rng),
        receipts_root: H256::random_using(&mut rng),
        logs_bloom: Bloom::default(),
        difficulty: U256::from(rng.gen::<u64>()),
        number: block_number,
        gas_limit: rng.gen::<u64>(),
        gas_used: {
            let limit = rng.gen::<u64>();
            rng.gen_range(0..limit)
        },
        timestamp: rng.gen::<u64>(),
        extra_data: Bytes::from(vec![]),
        prev_randao: H256::random_using(&mut rng),
        nonce: rng.gen::<u64>(),
        base_fee_per_gas: Some(rng.gen::<u64>()),
        withdrawals_root: Some(H256::random_using(&mut rng)),
        blob_gas_used: Some(rng.gen::<u64>()),
        excess_blob_gas: Some(rng.gen::<u64>()),
        parent_beacon_block_root: Some(H256::random_using(&mut rng)),
        requests_hash: Some(H256::random_using(&mut rng)),
    };

    let body = BlockBody {
        transactions,
        ommers: vec![],
        withdrawals: Some(vec![]),
    };

    Block::new(header, body)
}

// Helper function to create a test store with genesis
fn create_test_store(engine_type: EngineType) -> (Store, TempDir) {
    let temp_dir = TempDir::new("ethrex_benchmark").unwrap();
    let store_path = temp_dir.path().to_str().unwrap();

    // Get genesis
    let file = File::open("../../test_data/genesis-execution-api.json")
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let genesis: Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");

    // Create store with genesis
    let store = Store::new(store_path, engine_type.clone()).unwrap();
    store.add_initial_state(genesis).unwrap();

    (store, temp_dir)
}

fn bench_add_blocks(c: &mut Criterion) {
    let mut group = c.benchmark_group("add_blocks");
    group.measurement_time(Duration::from_secs(30));

    // Use the same base seed for consistent results across runs
    let base_seed = 42;

    for num_blocks in [10, 50, 100].iter() {
        group.throughput(Throughput::Elements(*num_blocks as u64));

        // Benchmark InMemory store
        {
            let (store, _temp_dir) = create_test_store(EngineType::InMemory);
            let genesis_header = store.get_block_header(0).unwrap().unwrap();
            let genesis_hash = genesis_header.compute_block_hash();

            group.bench_with_input(
                BenchmarkId::new("InMemory", num_blocks),
                num_blocks,
                |b, &num_blocks| {
                    b.iter(|| {
                        // Create a sequence of blocks
                        let mut parent_hash = genesis_hash;
                        let mut parent_number = 0;

                        for i in 0..num_blocks {
                            let block = create_block_with_random_transactions(
                                parent_hash,
                                parent_number + 1,
                                10, // 10 transactions per block
                                base_seed + i as u64,
                            );
                            parent_hash = block.hash();
                            parent_number = block.header.number;
                            black_box(store.add_block(block).unwrap());
                        }
                    });
                },
            );
        }

        // Benchmark MDBX store if available
        #[cfg(feature = "libmdbx")]
        {
            let (store, _temp_dir) = create_test_store(EngineType::Libmdbx);
            let genesis_header = store.get_block_header(0).unwrap().unwrap();
            let genesis_hash = genesis_header.compute_block_hash();

            group.bench_with_input(
                BenchmarkId::new("Libmdbx", num_blocks),
                num_blocks,
                |b, &num_blocks| {
                    b.iter(|| {
                        // Create a sequence of blocks
                        let mut parent_hash = genesis_hash;
                        let mut parent_number = 0;

                        for i in 0..num_blocks {
                            let block = create_block_with_random_transactions(
                                parent_hash,
                                parent_number + 1,
                                10, // 10 transactions per block
                                base_seed + i as u64,
                            );
                            parent_hash = block.hash();
                            parent_number = block.header.number;
                            black_box(store.add_block(block).unwrap());
                        }
                    });
                },
            );
        }

    }
    group.finish();
}

criterion_group!(benches, bench_add_blocks,);
criterion_main!(benches);
