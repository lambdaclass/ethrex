use std::time::Instant;

use ethrex_common::H256;

#[derive(Debug, Clone)]
struct ExecutionCycle {
    started_at: Instant,
    finished_at: Instant,
    started_at_block_num: u64,
    started_at_block_hash: H256,
    finished_at_block_num: u64,
    finished_at_block_hash: H256,
    executed_blocks_count: u32,
}

impl Default for ExecutionCycle {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            finished_at: Instant::now(),
            started_at_block_num: 0,
            started_at_block_hash: H256::default(),
            finished_at_block_num: 0,
            finished_at_block_hash: H256::default(),
            executed_blocks_count: 0,
        }
    }
}

#[derive(Debug, Default)]
struct Monitor {
    current_cycle: ExecutionCycle,
    prev_cycle: ExecutionCycle,
    blocks_to_restart_cycle: u32,
}

impl Monitor {
    pub fn new(start_block_num: u64, start_block_hash: H256, blocks_to_restart_cycle: u32) -> Self {
        Self {
            blocks_to_restart_cycle,
            prev_cycle: ExecutionCycle::default(),
            current_cycle: ExecutionCycle {
                started_at_block_num: start_block_num,
                started_at_block_hash: start_block_hash,
                ..Default::default()
            },
        }
    }

    pub fn log_cycle(&mut self, executed_blocks: u32, block_num: u64, block_hash: H256) {
        self.current_cycle.executed_blocks_count += executed_blocks;

        if self.current_cycle.executed_blocks_count >= self.blocks_to_restart_cycle {
            self.current_cycle.finished_at = Instant::now();
            self.current_cycle.finished_at_block_num = block_num;
            self.current_cycle.finished_at_block_hash = block_hash;
            self.show_stats();

            // restart cycle
            self.prev_cycle = self.current_cycle.clone();
            self.current_cycle = ExecutionCycle {
                started_at_block_num: block_num,
                started_at_block_hash: block_hash,
                ..ExecutionCycle::default()
            };
        }
    }

    fn show_stats(&self) {
        let elapsed = self
            .current_cycle
            .finished_at
            .duration_since(self.current_cycle.started_at)
            .as_secs();
        let avg = elapsed as f64 / self.current_cycle.executed_blocks_count as f64;

        let prev_elapsed = self
            .prev_cycle
            .finished_at
            .duration_since(self.prev_cycle.started_at)
            .as_secs();

        let elapsed_diff = elapsed as i128 - prev_elapsed as i128;

        tracing::info!(
            "[SYNCING PERF] Last {} blocks performance:\n\
            \tTotal time: {} seconds\n\
            \tAverage block time: {:.3} seconds\n\
            \tStarted at block: {} (hash: {:?})\n\
            \tFinished at block: {} (hash: {:?})\n\
            \tExecution count: {}\n\
            \t======= Overall, this cycle took {} seconds with respect to the previous one =======",
            self.current_cycle.executed_blocks_count,
            elapsed,
            avg,
            self.current_cycle.started_at_block_num,
            self.current_cycle.started_at_block_hash,
            self.current_cycle.finished_at_block_num,
            self.current_cycle.finished_at_block_hash,
            self.current_cycle.executed_blocks_count,
            elapsed_diff
        );
    }
}

#[derive(Default, Debug)]
pub struct SyncMetrics {
    monitors: Vec<Monitor>,
}

impl SyncMetrics {
    pub fn new(start_block_num: u64, start_block_hash: H256) -> Self {
        // start 6 monitors to show stats every:
        // - 100 blocks
        // - 1.000 blocks
        // - 10.000 blocks
        // - 100.000 blocks
        // - 1.000.000 blocks
        Self {
            monitors: vec![
                Monitor::new(start_block_num, start_block_hash, 100),
                Monitor::new(start_block_num, start_block_hash, 1000),
                Monitor::new(start_block_num, start_block_hash, 10000),
                Monitor::new(start_block_num, start_block_hash, 100000),
                Monitor::new(start_block_num, start_block_hash, 1000000),
            ],
        }
    }

    pub fn log_cycle(
        &mut self,
        number_of_blocks_processed: u32,
        last_block_number: u64,
        last_block_hash: H256,
    ) {
        for monitor in &mut self.monitors {
            monitor.log_cycle(
                number_of_blocks_processed,
                last_block_number,
                last_block_hash,
            );
        }
    }
}
