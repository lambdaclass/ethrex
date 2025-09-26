use std::{
    sync::Arc,
    thread::{scope, sleep},
    time::Duration,
};

use crate::myscope::ThreadPool;

pub mod myscope;

fn main() {
    println!("Start");
    scope(|s| {
        let pool = ThreadPool::new(1, s);
        let pool_arc = Arc::new(pool);
        let pool_arc_2 = pool_arc.clone();
        pool_arc.execute(Box::new(move || {
            sleep(Duration::from_secs(1));
            println!("Inside, Inside, world!");
            pool_arc_2.execute(Box::new(move || {
                println!("3, world!");
            }));
        }));
        pool_arc.execute_priority(Box::new(move || {
            println!("Inside, world!");
        }));
    });
    println!("Hello, world!");
}
