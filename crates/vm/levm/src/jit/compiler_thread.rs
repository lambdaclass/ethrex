//! Background JIT compilation thread pool.
//!
//! Provides a pool of background threads that process compilation requests
//! concurrently. When the execution counter hits the threshold, `vm.rs`
//! sends a non-blocking compilation request instead of blocking the VM thread.
//! `crossbeam-channel` enables multi-consumer distribution — requests are
//! fairly distributed across workers via work-stealing.

use std::sync::Arc;
use std::thread;

use crossbeam_channel::{Receiver, Sender};
use ethrex_common::types::{Code, Fork};

use super::arena::{ArenaId, FuncSlot};

/// A request to compile bytecode in the background.
#[derive(Clone)]
pub struct CompilationRequest {
    /// The bytecode to compile (Arc-backed Bytes + jump targets + hash).
    pub code: Code,
    /// The fork to compile for (opcodes/gas baked in at compile time).
    pub fork: Fork,
}

/// Request types for the background compiler thread pool.
#[derive(Clone)]
pub enum CompilerRequest {
    /// Compile bytecode into native code and insert into cache.
    Compile(CompilationRequest),
    /// Free a previously compiled function's arena slot.
    Free { slot: FuncSlot },
    /// Free an entire arena (all its LLVM resources).
    FreeArena { arena_id: ArenaId },
}

/// Handle to the background compiler thread pool.
///
/// Holds the sender half of a crossbeam channel. Compilation requests are sent
/// non-blocking; worker threads pull and process them concurrently.
///
/// On `Drop`, the sender is closed (causing all workers' `recv()` to return
/// `Err`) and all threads are joined. If any worker panicked, the panic is
/// logged (not propagated, to avoid double-panic in drop).
pub struct CompilerThreadPool {
    sender: Option<Sender<CompilerRequest>>,
    handles: Vec<thread::JoinHandle<()>>,
}

impl CompilerThreadPool {
    /// Start the background compiler thread pool with `num_workers` threads.
    ///
    /// The `handler_fn` closure is invoked for each request on a worker thread.
    /// It receives a `CompilerRequest` and should handle `Compile`, `Free`,
    /// and `FreeArena` variants. Any errors are logged and silently dropped
    /// (graceful degradation — the VM falls through to the interpreter).
    ///
    /// Each worker gets its own clone of the `Receiver` (crossbeam channels
    /// are multi-consumer) and processes requests independently. The handler
    /// is wrapped in `Arc` for sharing across workers.
    pub fn start<F>(num_workers: usize, handler_fn: F) -> Self
    where
        F: Fn(CompilerRequest) + Send + Sync + 'static,
    {
        debug_assert!(
            num_workers > 0,
            "CompilerThreadPool requires at least 1 worker"
        );
        let num_workers = num_workers.max(1);
        let (sender, receiver) = crossbeam_channel::unbounded::<CompilerRequest>();
        let handler = Arc::new(handler_fn);
        let mut handles = Vec::with_capacity(num_workers);

        for i in 0..num_workers {
            let rx = receiver.clone();
            let handler = Arc::clone(&handler);
            #[expect(clippy::expect_used, reason = "thread spawn failure is unrecoverable")]
            let handle = thread::Builder::new()
                .name(format!("jit-compiler-{i}"))
                .spawn(move || {
                    worker_loop(&rx, handler.as_ref());
                })
                .expect("failed to spawn JIT compiler worker");
            handles.push(handle);
        }

        Self {
            sender: Some(sender),
            handles,
        }
    }

    /// Number of worker threads in the pool.
    pub fn num_workers(&self) -> usize {
        self.handles.len()
    }

    /// Send a compilation request to the pool.
    ///
    /// Returns `true` if the request was sent successfully, `false` if the
    /// channel is disconnected (all workers shut down). Non-blocking —
    /// does not wait for compilation to complete.
    pub fn send(&self, request: CompilationRequest) -> bool {
        self.sender
            .as_ref()
            .map(|s| s.send(CompilerRequest::Compile(request)).is_ok())
            .unwrap_or(false)
    }

    /// Send a free request for an evicted function's arena slot.
    ///
    /// Returns `true` if the request was sent, `false` if disconnected.
    pub fn send_free(&self, slot: FuncSlot) -> bool {
        self.sender
            .as_ref()
            .map(|s| s.send(CompilerRequest::Free { slot }).is_ok())
            .unwrap_or(false)
    }

    /// Send a request to free an entire arena's LLVM resources.
    ///
    /// Returns `true` if the request was sent, `false` if disconnected.
    pub fn send_free_arena(&self, arena_id: ArenaId) -> bool {
        self.sender
            .as_ref()
            .map(|s| s.send(CompilerRequest::FreeArena { arena_id }).is_ok())
            .unwrap_or(false)
    }
}

/// Worker loop: pull requests from the channel and dispatch to the handler.
fn worker_loop<F>(rx: &Receiver<CompilerRequest>, handler: &F)
where
    F: Fn(CompilerRequest),
{
    while let Ok(request) = rx.recv() {
        handler(request);
    }
    // Channel closed — worker exits cleanly
}

impl Drop for CompilerThreadPool {
    fn drop(&mut self) {
        // Drop the sender first so all workers' recv() returns Err
        drop(self.sender.take());

        // Join all worker threads, logging any panics
        for handle in self.handles.drain(..) {
            if let Err(panic_payload) = handle.join() {
                eprintln!(
                    "[JIT] compiler worker panicked: {:?}",
                    panic_payload.downcast_ref::<&str>()
                );
            }
        }
    }
}

impl std::fmt::Debug for CompilerThreadPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompilerThreadPool")
            .field("active", &self.sender.is_some())
            .field("num_workers", &self.handles.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::types::Code;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn test_pool_sends_requests() {
        let count = Arc::new(AtomicU64::new(0));
        let count_clone = Arc::clone(&count);

        let pool = CompilerThreadPool::start(2, move |req| {
            if matches!(req, CompilerRequest::Compile(_)) {
                count_clone.fetch_add(1, Ordering::Relaxed);
            }
        });

        let code = Code::from_bytecode(Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xf3]));

        assert!(pool.send(CompilationRequest {
            code: code.clone(),
            fork: Fork::Cancun,
        }));
        assert!(pool.send(CompilationRequest {
            code,
            fork: Fork::Prague,
        }));

        // Give the workers time to process
        std::thread::sleep(std::time::Duration::from_millis(100));

        assert_eq!(count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_pool_distributes_across_workers() {
        use std::collections::HashSet;
        use std::sync::Mutex;

        let thread_ids = Arc::new(Mutex::new(HashSet::new()));
        let thread_ids_clone = Arc::clone(&thread_ids);
        let count = Arc::new(AtomicU64::new(0));
        let count_clone = Arc::clone(&count);

        let pool = CompilerThreadPool::start(2, move |req| {
            if matches!(req, CompilerRequest::Compile(_)) {
                // Record which thread processed this request
                #[expect(clippy::unwrap_used)]
                thread_ids_clone
                    .lock()
                    .unwrap()
                    .insert(std::thread::current().id());
                // Small sleep to ensure both workers get requests
                std::thread::sleep(std::time::Duration::from_millis(20));
                count_clone.fetch_add(1, Ordering::Relaxed);
            }
        });

        // Send 10 requests
        for i in 0..10u64 {
            let code = Code::from_bytecode(Bytes::from(vec![
                0x60,
                i.to_le_bytes()[0],
                0x60,
                0x00,
                0xf3,
            ]));
            assert!(pool.send(CompilationRequest {
                code,
                fork: Fork::Cancun,
            }));
        }

        // Wait for all to process
        std::thread::sleep(std::time::Duration::from_millis(500));

        assert_eq!(count.load(Ordering::Relaxed), 10);
        // With 10 requests and 20ms sleep, both workers should have participated
        #[expect(clippy::unwrap_used)]
        let unique_threads = thread_ids.lock().unwrap().len();
        assert_eq!(unique_threads, 2, "both workers should process requests");
    }

    #[test]
    fn test_pool_single_worker_backward_compat() {
        let count = Arc::new(AtomicU64::new(0));
        let count_clone = Arc::clone(&count);

        let pool = CompilerThreadPool::start(1, move |req| {
            if matches!(req, CompilerRequest::Compile(_)) {
                count_clone.fetch_add(1, Ordering::Relaxed);
            }
        });

        assert_eq!(pool.num_workers(), 1);

        let code = Code::from_bytecode(Bytes::from_static(&[0x60, 0x00, 0xf3]));
        for _ in 0..5 {
            assert!(pool.send(CompilationRequest {
                code: code.clone(),
                fork: Fork::Cancun,
            }));
        }

        // Drop joins all threads — requests are fully processed
        drop(pool);

        assert_eq!(count.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn test_pool_graceful_shutdown() {
        let count = Arc::new(AtomicU64::new(0));
        let count_clone = Arc::clone(&count);

        let pool = CompilerThreadPool::start(3, move |req| {
            if matches!(req, CompilerRequest::Compile(_)) {
                count_clone.fetch_add(1, Ordering::Relaxed);
            }
        });

        let code = Code::from_bytecode(Bytes::from_static(&[0x00]));
        assert!(pool.send(CompilationRequest {
            code,
            fork: Fork::Cancun,
        }));

        // Drop joins all threads — this must not hang or panic
        drop(pool);

        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_pool_free_requests_processed() {
        let free_count = Arc::new(AtomicU64::new(0));
        let free_arena_count = Arc::new(AtomicU64::new(0));
        let fc = Arc::clone(&free_count);
        let fac = Arc::clone(&free_arena_count);

        let pool = CompilerThreadPool::start(2, move |req| match req {
            CompilerRequest::Free { .. } => {
                fc.fetch_add(1, Ordering::Relaxed);
            }
            CompilerRequest::FreeArena { .. } => {
                fac.fetch_add(1, Ordering::Relaxed);
            }
            CompilerRequest::Compile(_) => {}
        });

        assert!(pool.send_free((0, 0)));
        assert!(pool.send_free((0, 1)));
        assert!(pool.send_free_arena(42));

        // Drop joins — all processed
        drop(pool);

        assert_eq!(free_count.load(Ordering::Relaxed), 2);
        assert_eq!(free_arena_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_pool_num_workers() {
        let pool = CompilerThreadPool::start(4, |_req: CompilerRequest| {});
        assert_eq!(pool.num_workers(), 4);
        drop(pool);
    }
}
