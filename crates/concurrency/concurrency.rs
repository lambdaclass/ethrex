use crossbeam::channel::{Sender, select_biased, unbounded};
use std::marker::Send;
use std::thread::{Builder, Scope, ScopedJoinHandle};
use tracing::error;

pub struct ThreadPool<'scope> {
    priority_sender: Sender<Box<dyn 'scope + Send + FnOnce()>>, // Implictly our threads in the thread pool have the receiver
    nice_sender: Sender<Box<dyn 'scope + Send + FnOnce()>>, // Implictly our threads in the thread pool have the receiver
    threads: Vec<ScopedJoinHandle<'scope, ()>>,
}

impl<'scope> ThreadPool<'scope> {
    pub fn new(thread_count: usize, scope: &'scope Scope<'scope, '_>) -> Self {
        let (priority_sender, priority_receiver) = unbounded::<Box<dyn 'scope + Send + FnOnce()>>();
        let (nice_sender, nice_receiver) = unbounded::<Box<dyn 'scope + Send + FnOnce()>>();
        let mut threads = Vec::new();

        for i in 0..thread_count {
            let priority_receiver = priority_receiver.clone();
            let nice_receiver = nice_receiver.clone();
            let _ = Builder::new()
                .name(format!("ThreadPool {i}"))
                .spawn_scoped(scope, move || {
                    // Thread work goes here
                    while let Ok(task) = select_biased! {
                        recv(priority_receiver) -> msg => msg,
                        recv(nice_receiver) -> msg => msg,
                    } {
                        task();
                    }
                    // If one of the senders closes because the threadpool is dropped, the other one
                    // channel may still exist and have data
                    while let Ok(task) = priority_receiver.recv() {
                        task();
                    }
                    while let Ok(task) = nice_receiver.recv() {
                        task();
                    }
                })
                .inspect_err(|err| error!(error=%err, "Couldn't spawn thread"))
                .map(|handle| threads.push(handle));
        }
        if threads.is_empty() {
            panic!("We couldn't spawn any threads!");
        }
        ThreadPool {
            priority_sender,
            nice_sender,
            threads,
        }
    }

    pub fn execute(&self, task: Box<dyn 'scope + Send + FnOnce()>) {
        self.nice_sender.send(task).unwrap();
    }

    pub fn execute_priority(&self, task: Box<dyn 'scope + Send + FnOnce()>) {
        self.priority_sender.send(task).unwrap();
    }
}

impl<'scope> Drop for ThreadPool<'scope> {
    fn drop(&mut self) {
        (self.nice_sender, _) = unbounded::<Box<dyn 'scope + Send + FnOnce()>>();
        (self.priority_sender, _) = unbounded::<Box<dyn 'scope + Send + FnOnce()>>();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
