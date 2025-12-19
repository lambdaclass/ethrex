use std::{
    collections::HashMap,
    ops::Add,
    sync::{LazyLock, Mutex},
    time::Duration,
};

use crate::opcodes::Opcode;

pub struct Timings {
    timings: HashMap<Opcode, OpcodeTiming>,
}

impl Timings {
    pub fn new() -> Self {
        Self {
            timings: HashMap::new(),
        }
    }

    pub fn add_timing(&mut self, opcode: u8, time: Duration) {
        let timing = self.timings.entry(Opcode::from(opcode)).or_default();
        timing.total = timing.total.add(time);
        timing.count = timing.count.saturating_add(1);
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OpcodeTiming {
    pub total: Duration,
    pub count: u64,
}

#[derive(Default, Debug)]
pub struct OpcodeTimingAccumulator {
    totals: HashMap<Opcode, Duration>,
    counts: HashMap<Opcode, u64>,
    blocks: usize,
    txs: usize,
}

impl OpcodeTimingAccumulator {
    pub fn update(&mut self, opcode: u8, time: Duration) {
        let opcode = Opcode::from(opcode);
        *self.totals.entry(opcode).or_default() += time;
        *self.counts.entry(opcode).or_default() += 1;
    }

    pub fn info(&self) -> (Vec<(Opcode, Duration, u64)>, usize, usize) {
        let mut average: Vec<(Opcode, Duration, u64)> = self
            .totals
            .iter()
            .filter_map(|(opcode, total)| {
                let count = *self.counts.get(opcode).unwrap_or(&0);
                (count > 0).then(|| {
                    (
                        *opcode,
                        Duration::from_secs_f64(total.as_secs_f64() / count as f64),
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
        format!(
            "[PERF] opcode timings avg per block (blocks={}, txs={}, sorted desc):\n{}",
            blocks_seen, txs_seen, pretty_avg
        )
    }

    pub fn inc_tx_count(&mut self, count: usize) {
        self.txs += count;
    }

    pub fn inc_block_count(&mut self) {
        self.blocks += 1;
    }
}

pub static OPCODE_TIMINGS: LazyLock<Mutex<OpcodeTimingAccumulator>> =
    LazyLock::new(|| Mutex::new(OpcodeTimingAccumulator::default()));

fn format_opcode_timings(sorted: &[(Opcode, Duration, u64)]) -> String {
    let mut out = String::new();
    for (opcode, dur, count) in sorted {
        out.push_str(&format!(
            "{:<16} {:>18?} ({:>10} calls)\n",
            format!("{opcode:?}"),
            dur,
            count
        ));
    }
    out
}
