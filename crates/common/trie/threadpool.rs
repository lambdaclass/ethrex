use crossbeam::channel::{Sender, select_biased, unbounded};
use std::marker::Send;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::{Builder, Scope};

pub struct ThreadPool<'scope> {
    priority_sender: Sender<Box<dyn 'scope + Send + FnOnce()>>, // Implictly our threads in the thread pool have the receiver
    nice_sender: Sender<Box<dyn 'scope + Send + FnOnce()>>, // Implictly our threads in the thread pool have the receiver
    /// Tracks the number of currently idle workers
    available_workers: Arc<AtomicUsize>,
    /// Total number of workers in the pool
    worker_count: usize,
}

impl<'scope> ThreadPool<'scope> {
    pub fn new(thread_count: usize, scope: &'scope Scope<'scope, '_>) -> Self {
        let (priority_sender, priority_receiver) = unbounded::<Box<dyn 'scope + Send + FnOnce()>>();
        let (nice_sender, nice_receiver) = unbounded::<Box<dyn 'scope + Send + FnOnce()>>();
        let available_workers = Arc::new(AtomicUsize::new(thread_count));

        for i in 0..thread_count {
            let priority_receiver = priority_receiver.clone();
            let nice_receiver = nice_receiver.clone();
            let available = available_workers.clone();
            let _ = Builder::new()
                .name(format!("ThreadPool {i}"))
                .spawn_scoped(scope, move || {
                    // Thread work goes here
                    while let Ok(task) = select_biased! {
                        recv(priority_receiver) -> msg => msg,
                        recv(nice_receiver) -> msg => msg,
                    } {
                        // Mark worker as busy
                        available.fetch_sub(1, Ordering::AcqRel);
                        task();
                        // Mark worker as available
                        available.fetch_add(1, Ordering::AcqRel);
                    }
                    // If one of the senders closes because the threadpool is dropped, the other one
                    // channel may still exist and have data
                    while let Ok(task) = priority_receiver.recv() {
                        available.fetch_sub(1, Ordering::AcqRel);
                        task();
                        available.fetch_add(1, Ordering::AcqRel);
                    }
                    while let Ok(task) = nice_receiver.recv() {
                        available.fetch_sub(1, Ordering::AcqRel);
                        task();
                        available.fetch_add(1, Ordering::AcqRel);
                    }
                });
        }
        ThreadPool {
            priority_sender,
            nice_sender,
            available_workers,
            worker_count: thread_count,
        }
    }

    pub fn execute(&self, task: Box<dyn 'scope + Send + FnOnce()>) {
        self.nice_sender.send(task).unwrap();
    }

    pub fn execute_priority(&self, task: Box<dyn 'scope + Send + FnOnce()>) {
        self.priority_sender.send(task).unwrap();
    }

    /// Returns the number of currently idle workers.
    ///
    /// This is a snapshot that may become stale immediately after reading,
    /// but is useful for making load balancing decisions.
    #[inline]
    pub fn available_workers(&self) -> usize {
        self.available_workers.load(Ordering::Acquire)
    }

    /// Returns the total number of workers in the pool.
    #[inline]
    pub fn worker_count(&self) -> usize {
        self.worker_count
    }

    /// Calculates an optimal chunk size for dividing work across available workers.
    ///
    /// # Arguments
    /// * `total` - Total number of items to process
    /// * `min` - Minimum chunk size (to avoid too-small chunks)
    ///
    /// # Returns
    /// A chunk size that balances work across available workers while respecting
    /// the minimum chunk size.
    #[inline]
    pub fn optimal_chunk_size(&self, total: usize, min: usize) -> usize {
        let available = self.available_workers().max(1);
        (total / available).max(min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::thread;

    #[test]
    fn test_worker_count() {
        thread::scope(|s| {
            let pool = ThreadPool::new(4, s);
            assert_eq!(pool.worker_count(), 4);
        });
    }

    #[test]
    fn test_optimal_chunk_size() {
        thread::scope(|s| {
            let pool = ThreadPool::new(4, s);
            // With 4 workers (all available), 100 items should give ~25 per worker
            // but we ensure at least the minimum
            assert_eq!(pool.optimal_chunk_size(100, 10), 25);
            assert_eq!(pool.optimal_chunk_size(100, 30), 30); // min takes precedence
            assert_eq!(pool.optimal_chunk_size(8, 5), 5); // min takes precedence for small totals
        });
    }

    #[test]
    fn test_available_workers_tracking() {
        thread::scope(|s| {
            let pool = ThreadPool::new(2, s);

            // Initially all workers should be available
            // Give workers time to start
            thread::sleep(std::time::Duration::from_millis(10));
            assert_eq!(pool.available_workers(), 2);

            // Submit a task that takes some time
            let counter = Arc::new(AtomicUsize::new(0));
            let counter_clone = counter.clone();
            pool.execute(Box::new(move || {
                thread::sleep(std::time::Duration::from_millis(100));
                counter_clone.fetch_add(1, Ordering::SeqCst);
            }));

            // Give time for worker to pick up task
            thread::sleep(std::time::Duration::from_millis(20));

            // One worker should be busy
            assert!(pool.available_workers() < 2);

            // Wait for task to complete
            thread::sleep(std::time::Duration::from_millis(150));
            assert_eq!(counter.load(Ordering::SeqCst), 1);
        });
    }
}
