use crossbeam::channel::{Receiver, Sender, unbounded};
use std::marker::Send;
use std::sync::{Arc, Mutex};
use std::thread::{Builder, Scope};

pub struct ThreadPool<'scope> {
    task_queue_sender: Sender<Box<dyn 'scope + Send + FnOnce()>>, // Implictly our threads in the thread pool have the receiver
}

impl<'scope> ThreadPool<'scope> {
    pub fn new(thread_count: usize, scope: &'scope Scope<'scope, '_>) -> Self {
        let (task_queue_sender, receiver) = unbounded::<Box<dyn 'scope + Send + FnOnce()>>();

        for i in 0..thread_count {
            let task_queue_rx_clone = receiver.clone();
            let _ = Builder::new()
                .name(format!("ThreadPool {i}"))
                .spawn_scoped(scope, move || {
                    // Thread work goes here
                    while let Ok(task) = task_queue_rx_clone.recv() {
                        task();
                    }
                });
        }

        ThreadPool { task_queue_sender }
    }

    pub fn execute(&self, task: Box<dyn 'scope + Send + FnOnce()>) {
        self.task_queue_sender.send(task).unwrap();
    }
}
