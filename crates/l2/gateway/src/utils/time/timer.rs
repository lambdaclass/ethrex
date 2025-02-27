use std::{
    collections::HashMap,
    fmt::Display,
    sync::{LazyLock, Mutex},
};

use crate::utils::{
    communication::{
        queue::{Producer, Queue, QueueType},
        queues_dir_string,
    },
    time::{Duration, Instant, InternalMessage},
};

#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct TimingMessage {
    pub start_t: Instant,
    pub stop_t: Instant,
}

impl TimingMessage {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            start_t: Instant::now(),
            stop_t: Default::default(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        Duration(self.stop_t.0.saturating_sub(self.start_t.0))
    }

    pub fn is_valid(&self) -> bool {
        self.start_t != Instant::ZERO && self.start_t.same_socket(&self.stop_t)
    }
}

impl From<TimingMessage> for Duration {
    fn from(value: TimingMessage) -> Self {
        value.elapsed()
    }
}

const QUEUE_SIZE: usize = 16384;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Timer {
    pub curmsg: TimingMessage,
    timing_producer: Producer<TimingMessage>,
    latency_producer: Producer<TimingMessage>,
}

impl Timer {
    pub fn new<S: Display>(name: S) -> Self {
        let dirstr = queues_dir_string();
        let _ = std::fs::create_dir_all(&dirstr);
        if cfg!(feature = "shmem") {
            let file = format!("{dirstr}/timing-{name}");

            let timing_queue = Queue::create_or_open_shared(file, QUEUE_SIZE, QueueType::MPMC)
                .expect("couldn't open timing queue");

            let file = format!("{dirstr}/latency-{name}");
            let latency_queue = Queue::create_or_open_shared(file, QUEUE_SIZE, QueueType::MPMC)
                .expect("couldn't open latency queue");
            Timer {
                curmsg: Default::default(),
                timing_producer: Producer::from(timing_queue),
                latency_producer: Producer::from(latency_queue),
            }
        } else {
            let timing_queue =
                Queue::new(QUEUE_SIZE, QueueType::MPMC).expect("couldn't open timing queue");

            let latency_queue =
                Queue::new(QUEUE_SIZE, QueueType::MPMC).expect("couldn't open latency queue");
            Timer {
                curmsg: Default::default(),
                timing_producer: Producer::from(timing_queue),
                latency_producer: Producer::from(latency_queue),
            }
        }
    }
}

unsafe impl Send for Timer {}
unsafe impl Sync for Timer {}

impl Timer {
    #[inline]
    pub fn start(&mut self) {
        self.set_start(Instant::now());
    }

    #[inline]
    pub fn stop(&mut self) {
        self.set_stop(Instant::now());
        self.send_business();
    }

    #[inline]
    pub fn stop_and_latency(&mut self, ingestion_t: Instant) {
        self.stop();
        self.set_stop(self.curmsg.start_t);
        self.set_start(ingestion_t);
        self.send_latency();
    }

    #[inline]
    pub fn stop_and_latency_till_now(&mut self, ingestion_t: Instant) {
        self.stop();
        self.set_start(ingestion_t);
        self.send_latency();
    }

    #[inline]
    pub fn latency_till_now(&mut self, ingestion_t: Instant) {
        let m = TimingMessage {
            start_t: ingestion_t,
            stop_t: Instant::now(),
        };
        if m.is_valid() {
            self.latency_producer.produce(&m);
        }
    }

    #[inline]
    fn set_stop(&mut self, stop: Instant) {
        self.curmsg.stop_t = stop;
    }

    #[inline]
    pub fn set_start(&mut self, start: Instant) {
        self.curmsg.start_t = start;
    }

    #[inline]
    fn send_latency(&mut self) {
        if self.curmsg.is_valid() {
            self.latency_producer.produce(&self.curmsg);
        }
    }

    #[inline]
    fn send_business(&mut self) {
        if self.curmsg.is_valid() {
            self.timing_producer.produce(&self.curmsg);
        }
    }

    #[inline]
    pub fn get_stop_t(&self) -> Instant {
        self.curmsg.stop_t
    }

    #[inline]
    pub fn start_accumulate(&mut self) {
        self.curmsg.start_t = Instant::ZERO;
        self.curmsg.stop_t = Instant::ZERO
    }

    #[inline]
    pub fn accumulate(&mut self, duration: Duration) {
        self.curmsg.stop_t += duration
    }

    #[inline]
    pub fn finish_accumulate(&mut self) {
        self.send_business()
    }

    #[inline]
    pub fn process<T, R>(
        &mut self,
        msg: InternalMessage<T>,
        mut f: impl FnMut(InternalMessage<T>) -> R,
    ) -> R {
        self.start();
        let in_t = (&msg).into();
        let o = f(msg);
        self.stop_and_latency(in_t);
        o
    }

    #[inline]
    pub fn time<R>(&mut self, f: impl FnOnce() -> R) -> R {
        self.start();
        let o = f();
        self.stop();
        o
    }

    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.curmsg.elapsed()
    }
}

// Global map of timers
#[allow(dead_code)]
pub static TIMERS: LazyLock<Mutex<HashMap<&'static str, Timer>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Macro to be used when quickly benchmarking some piece of code, should not remain in prod as it
/// is not particularly performant
#[macro_export]
macro_rules! timeit {
    ($name:expr, $block:block) => {{
        use bop_common::time::timer::TIMERS;
        // Initialize or retrieve the timer
        let mut timer = {
            let mut timers = TIMERS.lock().unwrap();
            timers
                .entry($name)
                .or_insert_with(|| bop_common::time::Timer::new($name))
                .clone()
        };

        // Start timing
        timer.start();

        // Execute the block of code and capture the result
        let result = { $block };

        // Stop timing
        timer.stop();

        // Return the block result
        result
    }};
}
