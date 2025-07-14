use std::{
    collections::VecDeque,
    sync::{Arc, LazyLock},
    time::{Duration, SystemTime},
};

use prometheus::{Counter, Gauge, Registry};
use tokio::sync::Mutex;

pub static METRICS: LazyLock<DiscoveryMetrics> = LazyLock::new(DiscoveryMetrics::default);

#[derive(Debug, Clone)]
pub struct DiscoveryMetrics {
    pub events: Arc<Mutex<VecDeque<SystemTime>>>,
    pub window_size: Duration,
    pub new_contacts_total: Counter,
    pub contacts_rate: Gauge,
    pub registry: Registry,
    start_time: SystemTime,
}

impl DiscoveryMetrics {
    pub fn new(window_size_in_secs: u64) -> Self {
        DiscoveryMetrics {
            window_size: Duration::from_secs(window_size_in_secs),
            ..Default::default()
        }
    }

    pub async fn record_new_contact(&self) {
        let mut events = self.events.lock().await;
        events.push_back(SystemTime::now());
        self.new_contacts_total.inc();
        self.update_contacts_rate(&mut events).await;
    }

    pub async fn update_contacts_rate(&self, events: &mut VecDeque<SystemTime>) {
        self.clean_old_events(events).await;

        let count = events.len() as f64;

        let windows_size_in_secs = self.window_size.as_secs_f64();
        let elapsed_from_start_time_in_secs =
            self.start_time.elapsed().unwrap_or_default().as_secs_f64();

        // Until self.window_size seconds have passed since the start time, we use the elapsed time
        // from the start time to calculate the rate.
        let window_secs = if elapsed_from_start_time_in_secs < windows_size_in_secs {
            elapsed_from_start_time_in_secs
        } else {
            windows_size_in_secs
        };

        let rate = if window_secs > 0.0 {
            count / window_secs
        } else {
            0.0
        };

        self.contacts_rate.set(rate);
    }

    pub async fn clean_old_events(&self, events: &mut VecDeque<SystemTime>) {
        let now = SystemTime::now();
        while let Some(&event_time) = events.front() {
            if now.duration_since(event_time).unwrap_or_default() > self.window_size {
                events.pop_front();
            } else {
                break;
            }
        }
    }
}

impl Default for DiscoveryMetrics {
    fn default() -> Self {
        let registry = Registry::new();

        let new_contacts_total = Counter::new(
            "discv4_new_contacts_total",
            "Total number of new nodes discovered",
        )
        .expect("Failed to create new_contacts_total counter");

        let contacts_rate = Gauge::new(
            "discv4_contacts_rate",
            "Rate of new nodes discovered per second",
        )
        .expect("Failed to create contacts_rate gauge");

        registry
            .register(Box::new(new_contacts_total.clone()))
            .expect("Failed to register new_contacts_total counter");

        registry
            .register(Box::new(contacts_rate.clone()))
            .expect("Failed to register contacts_rate gauge");

        DiscoveryMetrics {
            events: Arc::new(Mutex::new(VecDeque::new())),
            window_size: Duration::from_secs(60),
            new_contacts_total,
            contacts_rate,
            registry,
            start_time: SystemTime::now(),
        }
    }
}
