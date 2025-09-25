use std::{sync::Arc, thread::scope};

use crate::myscope::ThreadPool;

pub mod myscope;

fn main() {
    println!("Start");
    scope(|s| {
        let pool = ThreadPool::new(4, s);
        let pool_arc = Arc::new(pool);
        let pool_arc_2 = pool_arc.clone();
        pool_arc.execute(Box::new(move || {
            let x = 1;
            println!("Inside, Inside, world!");
            pool_arc_2.execute(Box::new(move || {
                let x = 3;
                println!("3, world!");
            }));
        }));
        pool_arc.execute(Box::new(move || {
            let x = 2;
            println!("Inside, world!");
        }));
    });
    println!("Hello, world!");
}
