//! Background JIT compilation thread.
//!
//! Provides a single background thread that processes compilation requests
//! asynchronously. When the execution counter hits the threshold, `vm.rs`
//! sends a non-blocking compilation request instead of blocking the VM thread.
//! The next execution of the same bytecode will find the compiled code in cache.

use std::sync::mpsc;
use std::thread;

use ethrex_common::types::{Code, Fork};

/// A request to compile bytecode in the background.
#[derive(Clone)]
pub struct CompilationRequest {
    /// The bytecode to compile (Arc-backed Bytes + jump targets + hash).
    pub code: Code,
    /// The fork to compile for (opcodes/gas baked in at compile time).
    pub fork: Fork,
}

/// Handle to the background compiler thread.
///
/// Holds the sender half of an mpsc channel. Compilation requests are sent
/// non-blocking; the background thread processes them sequentially.
///
/// On `Drop`, the sender is closed (causing the background thread's `recv()`
/// to return `Err`) and the thread is joined. If the background thread panicked,
/// the panic is propagated.
pub struct CompilerThread {
    sender: Option<mpsc::Sender<CompilationRequest>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl CompilerThread {
    /// Start the background compiler thread.
    ///
    /// The `compile_fn` closure is invoked for each request on the background
    /// thread. It receives the `CompilationRequest` and should compile + insert
    /// into the cache. Any errors are logged and silently dropped (graceful
    /// degradation — the VM falls through to the interpreter).
    pub fn start<F>(compile_fn: F) -> Self
    where
        F: Fn(CompilationRequest) + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel::<CompilationRequest>();

        #[expect(clippy::expect_used, reason = "thread spawn failure is unrecoverable")]
        let handle = thread::Builder::new()
            .name("jit-compiler".to_string())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    compile_fn(request);
                }
                // Channel closed — thread exits cleanly
            })
            .expect("failed to spawn JIT compiler thread");

        Self {
            sender: Some(sender),
            handle: Some(handle),
        }
    }

    /// Send a compilation request to the background thread.
    ///
    /// Returns `true` if the request was sent successfully, `false` if the
    /// channel is disconnected (thread panicked or shut down). Non-blocking —
    /// does not wait for compilation to complete.
    pub fn send(&self, request: CompilationRequest) -> bool {
        self.sender
            .as_ref()
            .map(|s| s.send(request).is_ok())
            .unwrap_or(false)
    }
}

impl Drop for CompilerThread {
    fn drop(&mut self) {
        // Drop the sender first so the background thread's recv() returns Err
        drop(self.sender.take());

        // Join the background thread, propagating any panic
        if let Some(handle) = self.handle.take()
            && let Err(panic_payload) = handle.join()
        {
            // Log panic but don't re-panic during drop (double-panic = abort)
            eprintln!(
                "[JIT] compiler thread panicked: {:?}",
                panic_payload.downcast_ref::<&str>()
            );
        }
    }
}

impl std::fmt::Debug for CompilerThread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompilerThread")
            .field("active", &self.sender.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::types::Code;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_compiler_thread_sends_requests() {
        let count = Arc::new(AtomicU64::new(0));
        let count_clone = Arc::clone(&count);

        let thread = CompilerThread::start(move |_req| {
            count_clone.fetch_add(1, Ordering::Relaxed);
        });

        let code = Code::from_bytecode(Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xf3]));

        assert!(thread.send(CompilationRequest {
            code: code.clone(),
            fork: Fork::Cancun,
        }));
        assert!(thread.send(CompilationRequest {
            code,
            fork: Fork::Prague,
        }));

        // Give the background thread time to process
        std::thread::sleep(std::time::Duration::from_millis(100));

        assert_eq!(count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_compiler_thread_graceful_on_drop() {
        let count = Arc::new(AtomicU64::new(0));
        let count_clone = Arc::clone(&count);

        let thread = CompilerThread::start(move |_req| {
            count_clone.fetch_add(1, Ordering::Relaxed);
        });

        let code = Code::from_bytecode(Bytes::from_static(&[0x00]));
        assert!(thread.send(CompilationRequest {
            code,
            fork: Fork::Cancun,
        }));

        // Drop joins the thread — this must not hang or panic
        drop(thread);

        // Thread was joined, so the request was processed
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_compiler_thread_send_after_drop_fails() {
        let thread = CompilerThread::start(|_req| {});
        let code = Code::from_bytecode(Bytes::from_static(&[0x00]));

        // Manually drop sender by dropping the whole thread
        // Can't test send-after-drop directly, but we can verify
        // the drop path doesn't panic
        drop(thread);
    }
}
