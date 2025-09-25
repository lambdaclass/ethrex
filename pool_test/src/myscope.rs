use std::thread;

pub struct ThreadPool<'scope, 'env> {
    s: &'scope thread::Scope<'scope, 'env>,
}

impl<'scope, 'env> ThreadPool<'scope, 'env> {
    pub fn new(thread_count: usize) -> Self {
        thread::scope(|s| {
                for _ in 0..thread_count {
                    s.spawn(|| {
                        // Thread work goes here
                    });
                }
            });

        ThreadPool {
            s: ,
        }
    }
}

fn execute<'a>(s: &'a thread::Scope<'a, '_>, f: impl FnOnce() + std::marker::Send + 'a) {
    s.spawn(|| f);
}

fn run_scoped_threads() {
    let mut a = vec![1, 2, 3];
    let mut x = 0;

    let f = || {
            println!("hello from the first scoped thread");
            // We can borrow `a` here.
            dbg!(&a);
        };
    thread::scope(|s| {
        s.spawn(f);
        s.spawn(|| {
            println!("hello from the second scoped thread");
            // We can even mutably borrow `x` here,
            // because no other threads are using it.
            x += a[0] + a[2];
        });
        println!("hello from the main thread");
    });

    // After the scope, we can modify and access our variables again:
    a.push(4);
    assert_eq!(x, a.len());
}

//std::thread::scope
