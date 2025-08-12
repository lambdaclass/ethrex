use crossbeam_queue::SegQueue;
use std::io::Write;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::{
    borrow::Cow,
    thread::sleep,
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub enum Value {
    Duration(Duration),
    Float(f64),
    Int(usize),
}

// time elapsed since start, label, duration
static LOGS: SegQueue<(u64, Cow<'static, str>, Value)> = SegQueue::new();
static START: OnceLock<Instant> = OnceLock::new();

pub fn add_log(label: impl Into<Cow<'static, str>>, value: Value) {
    LOGS.push((
        START.get().unwrap().elapsed().as_secs(),
        label.into(),
        value,
    ));
}

pub struct DropLog {
    label: Cow<'static, str>,
    start: Instant,
}

impl DropLog {
    pub fn wrap_return<T>(&self, inp: T) -> T {
        inp
    }
}

impl Drop for DropLog {
    fn drop(&mut self) {
        add_log(self.label.clone(), Value::Duration(self.start.elapsed()))
    }
}

// starts time when called, stops when dropped, useful when multiple paths that can go out of scope
#[must_use]
pub fn add_time_till_drop(label: impl Into<Cow<'static, str>>) -> DropLog {
    DropLog {
        label: label.into(),
        start: Instant::now(),
    }
}

static SIGNAL_CLOSE: OnceLock<AtomicBool> = OnceLock::new();

fn process_logs_thread() {
    SIGNAL_CLOSE.get_or_init(|| AtomicBool::new(false));
    let mut file = std::fs::File::create("ethrex-perf-logs.log").expect("error getting file");

    loop {
        let next_log = LOGS.pop();

        match next_log {
            Some((elapsed_since_start, label, value)) => {
                writeln!(
                    file,
                    "{} | {} | {}",
                    elapsed_since_start,
                    label,
                    match value {
                        Value::Duration(duration) => duration.as_millis().to_string(),
                        Value::Float(value) => format!("{value:.3}"),
                        Value::Int(value) => value.to_string(),
                    }
                )
                .ok();
            }
            None => {
                if SIGNAL_CLOSE
                    .get()
                    .unwrap()
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    file.flush().unwrap();
                    return;
                }
                file.flush().ok();
                sleep(Duration::from_millis(500))
            }
        }
    }
}

pub fn init_process_logs_thread() {
    START.get_or_init(Instant::now);
    std::thread::spawn(process_logs_thread);
}

pub fn close_logging() {
    SIGNAL_CLOSE
        .get()
        .unwrap()
        .store(false, std::sync::atomic::Ordering::Release);
}
