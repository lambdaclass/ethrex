//! Persistent worker pool for high-queue-depth cold reads.
//!
//! Cold trie-node / flat-KV reads on a bloated account spend almost all their
//! time blocked on the NVMe device (RocksDB `async_io` is off, so a single
//! `multi_get` runs at queue depth ~1). The lever that hides this latency is
//! issuing many reads concurrently: modern NVMe needs ~64-128 requests in
//! flight to reach peak throughput. So the pool size is an **I/O queue depth**,
//! not a CPU budget -- it is intentionally far larger than the core count.
//!
//! Why a dedicated pool instead of `std::thread::scope` per block: spawning a
//! batch of OS threads on every block costs ~`pthread_create` per shard
//! (~1-2ms at 64 shards) and that cost is pure waste on small/warm blocks. A
//! persistent pool pays the spawn cost once at startup; per-block dispatch is
//! just a channel send.
//!
//! Why not rayon: rayon's global pool is sized to the core count and shared
//! with CPU work, and its scoped APIs deadlock when shared across more
//! concurrent callers than `threads / jobs_per_call` (see the reverted
//! `ShardWorkerPool`). This pool sidesteps both: it is sized for I/O, and its
//! jobs are independent and fire-and-forget -- a worker runs one closure and
//! returns to its queue, never blocking on another worker, so it is safe to
//! share across any number of concurrent callers (no coordinator, no cross-job
//! protocol, hence no deadlock topology).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::thread;

/// A unit of work: a boxed closure run on a worker thread. Jobs are expected to
/// be independent and self-contained (they capture their own `Arc`-shared read
/// view, keys, and result channel) so workers never block on each other.
type Job = Box<dyn FnOnce() + Send + 'static>;

/// A persistent pool of blocking worker threads sized for NVMe queue depth.
pub struct PrefetchPool {
    /// One queue per worker; `submit` round-robins across them (mirrors reth's
    /// `BalPrewarmPool`), avoiding a shared `Mutex<Receiver>` on the hot path.
    workers: Vec<Sender<Job>>,
    /// Round-robin cursor for distributing jobs across workers.
    next: AtomicUsize,
}

impl PrefetchPool {
    fn new(threads: usize) -> Self {
        let mut workers = Vec::with_capacity(threads);
        for i in 0..threads {
            let (tx, rx) = channel::<Job>();
            workers.push(tx);
            thread::Builder::new()
                .name(format!("prefetch-{i}"))
                // Workers only run small read closures; the default 2MB stack is
                // wasteful at 128 threads, so cap it low. Virtual, mostly
                // uncommitted, but keeps the footprint tidy.
                .stack_size(256 * 1024)
                .spawn(move || {
                    // Blocks when idle; ends when the pool (and thus the Sender)
                    // is dropped -- which for the process-wide pool is never.
                    while let Ok(job) = rx.recv() {
                        job();
                    }
                })
                .expect("failed to spawn prefetch worker");
        }
        Self {
            workers,
            next: AtomicUsize::new(0),
        }
    }

    /// Hand a job to some worker. Fire-and-forget: the job reports its own
    /// result via whatever channel it captured. If the worker thread is gone
    /// the send is silently dropped (the caller's read just stays cold).
    pub fn submit(&self, job: Job) {
        let i = self.next.fetch_add(1, Ordering::Relaxed) % self.workers.len();
        let _ = self.workers[i].send(job);
    }
}

/// Number of worker threads in the process-wide pool. Sized to hold NVMe queue
/// depth (peak throughput needs ~64-128 in-flight reads), not core count.
const PREFETCH_POOL_THREADS: usize = 128;

/// The process-wide prefetch pool. Shared by every `Store` instance so creating
/// many stores (e.g. in tests) does not multiply OS threads. Lazily spawned on
/// first use.
pub fn prefetch_pool() -> &'static PrefetchPool {
    static POOL: OnceLock<PrefetchPool> = OnceLock::new();
    POOL.get_or_init(|| PrefetchPool::new(PREFETCH_POOL_THREADS))
}
