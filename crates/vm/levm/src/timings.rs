use std::{
    collections::HashMap,
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::opcodes::Opcode;
use crate::precompiles::PRECOMPILES;
use ethrex_common::Address;

#[derive(Default, Debug)]
pub struct OpcodeTimings {
    totals: HashMap<Opcode, Duration>,
    counts: HashMap<Opcode, u64>,
    blocks: usize,
    txs: usize,
}

impl OpcodeTimings {
    pub fn update(&mut self, opcode: u8, time: Duration) {
        let opcode = Opcode::from(opcode);
        *self.totals.entry(opcode).or_default() += time;
        *self.counts.entry(opcode).or_default() += 1;
    }

    pub fn info(&self) -> (Vec<(Opcode, Duration, Duration, u64)>, usize, usize) {
        let mut average: Vec<(Opcode, Duration, Duration, u64)> = self
            .totals
            .iter()
            .filter_map(|(opcode, total)| {
                let count = *self.counts.get(opcode).unwrap_or(&0);
                (count > 0).then(|| {
                    (
                        *opcode,
                        Duration::from_secs_f64(total.as_secs_f64() / count as f64),
                        *total,
                        count,
                    )
                })
            })
            .collect();
        average.sort_by(|a, b| b.1.cmp(&a.1));
        (average, self.blocks, self.txs)
    }

    pub fn info_pretty(&self) -> String {
        let (avg_timings_sorted, blocks_seen, txs_seen) = self.info();
        let pretty_avg = format_opcode_timings(&avg_timings_sorted);
        let total_accumulated = self
            .totals
            .values()
            .fold(Duration::from_secs(0), |acc, dur| acc + *dur);
        format!(
            "[PERF] opcode timings avg per block (blocks={}, txs={}, total={:?}, sorted desc):\n{}",
            blocks_seen, txs_seen, total_accumulated, pretty_avg
        )
    }

    pub fn inc_tx_count(&mut self, count: usize) {
        self.txs += count;
    }

    pub fn inc_block_count(&mut self) {
        self.blocks += 1;
    }
}

pub static OPCODE_TIMINGS: LazyLock<Mutex<OpcodeTimings>> =
    LazyLock::new(|| Mutex::new(OpcodeTimings::default()));

#[derive(Default, Debug)]
pub struct PrecompilesTimings {
    totals: HashMap<Address, Duration>,
    counts: HashMap<Address, u64>,
}

impl PrecompilesTimings {
    pub fn update(&mut self, address: Address, time: Duration) {
        *self.totals.entry(address).or_default() += time;
        *self.counts.entry(address).or_default() += 1;
    }

    pub fn info(&self) -> Vec<(Address, Duration, Duration, u64)> {
        let mut average: Vec<(Address, Duration, Duration, u64)> = self
            .totals
            .iter()
            .filter_map(|(address, total)| {
                let count = *self.counts.get(address).unwrap_or(&0);
                (count > 0).then(|| {
                    (
                        *address,
                        Duration::from_secs_f64(total.as_secs_f64() / count as f64),
                        *total,
                        count,
                    )
                })
            })
            .collect();
        average.sort_by(|a, b| b.1.cmp(&a.1));
        average
    }

    pub fn info_pretty(&self) -> String {
        let pretty_avg = format_precompile_timings(&self.info());
        let total_accumulated = self
            .totals
            .values()
            .fold(Duration::from_secs(0), |acc, dur| acc + *dur);
        format!(
            "[PERF] precompile timings (total={:?}, sorted desc):\n{}",
            total_accumulated, pretty_avg
        )
    }
}

pub static PRECOMPILES_TIMINGS: LazyLock<Mutex<PrecompilesTimings>> =
    LazyLock::new(|| Mutex::new(PrecompilesTimings::default()));

fn format_opcode_timings(sorted: &[(Opcode, Duration, Duration, u64)]) -> String {
    let mut out = String::new();
    for (opcode, avg_dur, total_dur, count) in sorted {
        out.push_str(&format!(
            "{:<16} {:>18?} {:>18?} ({:>10} calls)\n",
            format!("{opcode:?}"),
            avg_dur,
            total_dur,
            count
        ));
    }
    out
}

fn format_precompile_timings(sorted: &[(Address, Duration, Duration, u64)]) -> String {
    let mut out = String::new();
    for (address, avg_dur, total_dur, count) in sorted {
        let name = PRECOMPILES
            .iter()
            .find(|precompile| &precompile.address == address)
            .map(|precompile| precompile.name)
            .unwrap_or("unknown");
        out.push_str(&format!(
            "{:<16} {:>18?} {:>18?} ({:>10} calls)\n",
            name, avg_dur, total_dur, count
        ));
    }
    out
}

/// SLOAD cache attribution counters for measuring hit/miss rates across cache layers.
///
/// Uses atomics for thread safety since CachingDatabase is shared between
/// warmer and executor threads.
pub struct SloadCounters {
    /// Value found in GeneralizedDatabase.current_accounts_state.storage (L1 per-tx cache)
    pub sload_l1_hit: AtomicU64,
    /// Value found in CachingDatabase storage cache (L2 cross-tx cache)
    pub sload_l2_hit: AtomicU64,
    /// Value not in CachingDatabase, fell through to Store/trie
    pub sload_l2_miss: AtomicU64,
    /// Read miss on CachingDatabase became a hit on write recheck (warmer filled it between locks)
    pub sload_duplicate_miss_race: AtomicU64,
}

impl SloadCounters {
    const fn new() -> Self {
        Self {
            sload_l1_hit: AtomicU64::new(0),
            sload_l2_hit: AtomicU64::new(0),
            sload_l2_miss: AtomicU64::new(0),
            sload_duplicate_miss_race: AtomicU64::new(0),
        }
    }

    pub fn reset(&self) {
        self.sload_l1_hit.store(0, Ordering::Relaxed);
        self.sload_l2_hit.store(0, Ordering::Relaxed);
        self.sload_l2_miss.store(0, Ordering::Relaxed);
        self.sload_duplicate_miss_race.store(0, Ordering::Relaxed);
    }

    pub fn info_pretty(&self) -> String {
        let l1 = self.sload_l1_hit.load(Ordering::Relaxed);
        let l2 = self.sload_l2_hit.load(Ordering::Relaxed);
        let miss = self.sload_l2_miss.load(Ordering::Relaxed);
        let race = self.sload_duplicate_miss_race.load(Ordering::Relaxed);
        let total = l1.saturating_add(l2).saturating_add(miss);
        format!(
            "[PERF] sload cache (total={total}): l1_hit={l1} l2_hit={l2} l2_miss={miss} dup_race={race}"
        )
    }
}

pub static SLOAD_COUNTERS: SloadCounters = SloadCounters::new();
