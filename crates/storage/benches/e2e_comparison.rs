//! End-to-end performance comparison benchmark
//!
//! Compares the current Store+TrieLayerCache approach vs BlockchainStateManager
//! using realistic block execution workloads.
//!
//! Run with: cargo bench --bench e2e_comparison -p ethrex-storage --features rocksdb

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use std::time::Instant;

use ethrex_common::{Address, H256, U256};
use ethrex_common::types::AccountInfo;

use ethrex_storage::state_manager::BlockchainStateManager;

/// Simulate a block with N account updates and M storage updates per account
fn simulate_block_workload(
    manager: &BlockchainStateManager,
    parent_hash: H256,
    block_number: u64,
    num_accounts: usize,
    storage_slots_per_account: usize,
) -> H256 {
    let block_hash = H256::from_low_u64_be(block_number);

    let mut block = manager.start_block(parent_hash, block_hash, block_number).unwrap();

    // Update accounts
    for i in 0..num_accounts {
        let address = Address::from_low_u64_be(i as u64);
        let info = AccountInfo {
            nonce: block_number,
            balance: U256::from(1000 + i),
            code_hash: H256::zero(),
        };
        block.set_account(&address, &info);

        // Update storage slots
        for j in 0..storage_slots_per_account {
            let slot = H256::from_low_u64_be(j as u64);
            let value = U256::from(block_number * 1000 + j as u64);
            block.set_storage(&address, slot, value);
        }
    }

    manager.commit_block(block).unwrap();
    block_hash
}

/// Benchmark: Multiple blocks with account/storage updates
fn bench_block_execution(c: &mut Criterion) {
    let mut group = c.benchmark_group("Block Execution");
    group.sample_size(20); // Reduce sample size for longer benchmarks

    // Test different workload sizes
    for (accounts, storage_slots) in [(100, 10), (500, 5), (1000, 2)] {
        let id = format!("{}acc_{}slots", accounts, storage_slots);

        group.bench_function(BenchmarkId::new("BlockchainStateManager", &id), |b| {
            b.iter_custom(|iters| {
                let mut total = std::time::Duration::ZERO;

                for _ in 0..iters {
                    let manager = BlockchainStateManager::in_memory(10000).unwrap();

                    let start = Instant::now();
                    // Execute 10 blocks
                    let mut parent_hash = manager.last_finalized_hash();
                    for block_num in 1..=10 {
                        parent_hash = simulate_block_workload(&manager, parent_hash, block_num, accounts, storage_slots);
                    }
                    total += start.elapsed();
                }

                total
            });
        });
    }

    group.finish();
}

/// Benchmark: Block finalization (persisting to cold storage)
fn bench_finalization(c: &mut Criterion) {
    let mut group = c.benchmark_group("Finalization");
    group.sample_size(20);

    for num_blocks in [5, 10, 20] {
        group.bench_function(BenchmarkId::new("finalize_chain", num_blocks), |b| {
            b.iter_custom(|iters| {
                let mut total = std::time::Duration::ZERO;

                for _ in 0..iters {
                    let manager = BlockchainStateManager::in_memory(10000).unwrap();

                    // Create blocks
                    let mut parent_hash = manager.last_finalized_hash();
                    for block_num in 1..=num_blocks {
                        parent_hash = simulate_block_workload(&manager, parent_hash, block_num as u64, 100, 5);
                    }

                    // Measure finalization time
                    let start = Instant::now();
                    manager.finalize(parent_hash).unwrap();
                    total += start.elapsed();
                }

                total
            });
        });
    }

    group.finish();
}

/// Benchmark: State queries after finalization
fn bench_state_queries(c: &mut Criterion) {
    let mut group = c.benchmark_group("State Queries");

    // Setup: Create and finalize some state
    let manager = BlockchainStateManager::in_memory(10000).unwrap();
    let mut parent_hash = manager.last_finalized_hash();
    for block_num in 1..=10 {
        parent_hash = simulate_block_workload(&manager, parent_hash, block_num, 1000, 5);
    }
    manager.finalize(parent_hash).unwrap();

    // Benchmark account lookups
    group.bench_function("get_finalized_account", |b| {
        let address = Address::from_low_u64_be(500); // Middle of the range
        b.iter(|| {
            black_box(manager.get_finalized_account(&address))
        });
    });

    // Benchmark state root computation
    group.bench_function("state_root", |b| {
        b.iter(|| {
            black_box(manager.state_root())
        });
    });

    group.finish();
}

/// Benchmark: Simulate realistic block processing rate
fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("Throughput");
    group.sample_size(10);

    // Simulate Ethereum mainnet-like workload:
    // ~150 transactions per block, each touching 1-2 accounts
    group.bench_function("mainnet_like_block", |b| {
        b.iter_custom(|iters| {
            let mut total = std::time::Duration::ZERO;

            for _ in 0..iters {
                let manager = BlockchainStateManager::in_memory(10000).unwrap();

                let start = Instant::now();

                // 100 blocks
                let mut parent_hash = manager.last_finalized_hash();
                for block_num in 1..=100u64 {
                    let block_hash = H256::from_low_u64_be(block_num);

                    let mut block = manager.start_block(parent_hash, block_hash, block_num).unwrap();

                    // ~150 "transactions" per block
                    for tx in 0..150 {
                        let from = Address::from_low_u64_be(tx as u64 % 10000);
                        let to = Address::from_low_u64_be((tx as u64 + 1) % 10000);

                        // Update sender
                        let from_info = AccountInfo {
                            nonce: block_num + tx as u64,
                            balance: U256::from(1_000_000 - tx),
                            code_hash: H256::zero(),
                        };
                        block.set_account(&from, &from_info);

                        // Update receiver
                        let to_info = AccountInfo {
                            nonce: 0,
                            balance: U256::from(tx),
                            code_hash: H256::zero(),
                        };
                        block.set_account(&to, &to_info);
                    }

                    manager.commit_block(block).unwrap();

                    // Finalize every 32 blocks (like Ethereum finality)
                    if block_num % 32 == 0 {
                        manager.finalize(block_hash).unwrap();
                        parent_hash = manager.last_finalized_hash();
                    } else {
                        parent_hash = block_hash;
                    }
                }

                total += start.elapsed();
            }

            total
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_block_execution,
    bench_finalization,
    bench_state_queries,
    bench_throughput
);
criterion_main!(benches);
