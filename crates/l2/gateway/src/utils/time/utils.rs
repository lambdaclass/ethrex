use crate::utils::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, Ordering};

#[inline(always)]
pub fn vsync_busy<F, R>(duration: Option<Duration>, f: F) -> R
where
    F: FnOnce() -> R,
{
    match duration {
        Some(duration) if duration != Duration(0) => {
            let start_t = Instant::now();
            let out = f();
            while start_t.elapsed() < duration {}
            out
        }
        _ => f(),
    }
}

#[inline]
pub fn vsync<F, R>(duration: Option<Duration>, f: F) -> R
where
    F: FnOnce() -> R,
{
    match duration {
        Some(duration) if duration != Duration(0) => {
            let start_t = Instant::now();
            let out = f();
            let el = start_t.elapsed();
            if el < duration {
                std::thread::sleep((duration - el).into())
            }
            out
        }
        _ => f(),
    }
}

#[inline]
pub fn renderloop_60_fps(mut f: impl FnMut() -> bool) {
    while vsync(Some(Duration::from_millis(16)), &mut f) {}
}

#[inline]
pub fn vsync_with_cancel<F, R>(
    duration: Option<Duration>,
    cancel: &std::sync::Arc<AtomicBool>,
    f: F,
) -> R
where
    F: FnOnce() -> R,
{
    let sleep_before_cancelcheck = std::time::Duration::from_millis(1);
    match duration {
        Some(duration) if duration != Duration(0) => {
            let start_t = Instant::now();
            let out = f();
            let el = start_t.elapsed();
            while el < duration && !cancel.load(Ordering::Relaxed) {
                std::thread::sleep(sleep_before_cancelcheck);
            }
            out
        }
        _ => f(),
    }
}

// TODO: does match statement impact performance? Thesis: no because of branch predicting
#[inline]
pub fn busy_sleep(duration: Option<Duration>) {
    match duration {
        None => (),
        Some(duration) if duration == Duration::ZERO => (),
        Some(duration) => {
            let curt = Instant::now();
            while curt.elapsed() < duration {}
        }
    }
}

#[inline]
pub fn sleep(duration: Duration) {
    if duration == Duration::ZERO {
        std::thread::sleep(duration.into())
    }
}

#[inline]
pub fn timeit<O>(msg: &str, f: impl FnOnce() -> O) -> O {
    let curt = Instant::now();
    let o = f();
    println!("Timing result: {msg} took {}", curt.elapsed());
    o
}
