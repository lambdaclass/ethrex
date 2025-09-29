use crossbeam::channel::{Sender, select_biased, unbounded};
use std::marker::Send;
use std::thread::{Builder, Scope};

pub struct ThreadPool<'scope> {
    priority_sender: Sender<Box<dyn 'scope + Send + FnOnce()>>, // Implictly our threads in the thread pool have the receiver
    nice_sender: Sender<Box<dyn 'scope + Send + FnOnce()>>, // Implictly our threads in the thread pool have the receiver
}

impl<'scope> ThreadPool<'scope> {
    pub fn new(thread_count: usize, scope: &'scope Scope<'scope, '_>) -> Self {
        let (priority_sender, priority_receiver) = unbounded::<Box<dyn 'scope + Send + FnOnce()>>();
        let (nice_sender, nice_receiver) = unbounded::<Box<dyn 'scope + Send + FnOnce()>>();

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
                });
        }
        ThreadPool {
            priority_sender,
            nice_sender,
        }
    }

    pub fn execute(&self, task: Box<dyn 'scope + Send + FnOnce()>) {
        self.nice_sender.send(task).unwrap();
    }

    pub fn execute_priority(&self, task: Box<dyn 'scope + Send + FnOnce()>) {
        self.priority_sender.send(task).unwrap();
    }
}
