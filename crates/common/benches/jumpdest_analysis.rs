//! JUMPDEST analysis of max-size CREATE initcode, mirroring execution-specs
//! `test_jumpdest_analysis` (ethereum/execution-specs#3631).
//!
//! That test CREATEs a max-size initcode in a loop, so each CREATE pays one full
//! analysis ([`Code::compute_jumpdests`]) and almost no execution. The periodic
//! tiles let the branch predictor learn the period. The random arms draw
//! non-repeating bytes over alphabets that straddle the thresholds the analysis
//! loop branches on (0x5B JUMPDEST, 0x60 PUSH1), so the gap between the two
//! groups is the branch-misprediction cost.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ethrex_common::types::Code;
use rand::{SeedableRng, distr::Distribution, distr::weighted::WeightedIndex, rngs::StdRng};
use std::hint::black_box;

/// Amsterdam's max initcode size (`AMSTERDAM_INIT_CODE_MAX_SIZE` in ethrex-levm,
/// which this crate can't depend on).
const INITCODE_SIZE: usize = 2 * 0x10000;

/// The test tiles each pattern into a window of this size, pads the window with
/// JUMPDEST, then tiles the window across the initcode.
const TILE_WINDOW: usize = 1024;

const STOP: u8 = 0x00;
const JUMPDEST: u8 = 0x5B;
const PUSH1: u8 = 0x60;
const PUSH2: u8 = 0x61;
const DUPN: u8 = 0xE6;
const SWAPN: u8 = 0xE7;
const EXCHANGE: u8 = 0xE8;

/// Periodic patterns, in the test's parametrize order.
const PERIODIC: &[&[u8]] = &[
    &[STOP],
    &[JUMPDEST],
    &[PUSH1, JUMPDEST],
    &[PUSH2, JUMPDEST, JUMPDEST],
    &[PUSH1, JUMPDEST, JUMPDEST],
    &[PUSH2, JUMPDEST, JUMPDEST, JUMPDEST],
    &[SWAPN, JUMPDEST],
    &[DUPN, JUMPDEST],
    &[EXCHANGE, JUMPDEST],
];

/// Random arms as `(id, alphabet, weights)`, named as in the test.
const RANDOM: &[(&str, &[u8], &[u32])] = &[
    (
        "random_stop_jumpdest_push1",
        &[STOP, JUMPDEST, PUSH1],
        &[1, 1, 1],
    ),
    ("random_stop_jumpdest", &[STOP, JUMPDEST], &[1, 1]),
    ("random_jumpdest_push1", &[JUMPDEST, PUSH1], &[1, 1]),
    // PUSH1 is half of all bytes, so the PUSH1 threshold is a 50/50 branch.
    (
        "random_stop_jumpdest_2push1",
        &[STOP, JUMPDEST, PUSH1],
        &[1, 1, 2],
    ),
];

fn periodic_initcode(pattern: &[u8]) -> Vec<u8> {
    let mut window = pattern.repeat(TILE_WINDOW / pattern.len());
    window.resize(TILE_WINDOW, JUMPDEST);
    window.repeat(INITCODE_SIZE / TILE_WINDOW)
}

/// The test seeds Python's `random.Random(0)`; this uses a different generator,
/// so the bytes differ but the distribution over the alphabet is the same.
fn random_initcode(alphabet: &[u8], weights: &[u32]) -> Vec<u8> {
    #[expect(clippy::unwrap_used, reason = "the weights above are valid constants")]
    let dist = WeightedIndex::new(weights).unwrap();
    let mut rng = StdRng::seed_from_u64(0);
    dist.sample_iter(&mut rng)
        .take(INITCODE_SIZE)
        .map(|i| alphabet[i])
        .collect()
}

fn bench_jumpdest_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("jumpdest_analysis");
    group.throughput(Throughput::Bytes(INITCODE_SIZE as u64));

    let periodic = PERIODIC
        .iter()
        .map(|pattern| (hex::encode(pattern), periodic_initcode(pattern)));
    let random = RANDOM
        .iter()
        .map(|(id, alphabet, weights)| (id.to_string(), random_initcode(alphabet, weights)));

    for (id, initcode) in periodic.chain(random) {
        group.bench_with_input(BenchmarkId::from_parameter(id), &initcode, |b, code| {
            b.iter(|| Code::compute_jumpdests(black_box(code)))
        });
    }
    group.finish();
}

criterion_group!(benches, bench_jumpdest_analysis);
criterion_main!(benches);
