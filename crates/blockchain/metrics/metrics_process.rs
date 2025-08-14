use prometheus::{Encoder, Gauge, IntGauge, Registry, TextEncoder};
use std::sync::{LazyLock, Mutex};
use sysinfo::{System, Pid};

use crate::MetricsError;

pub static METRICS_PROCESS: LazyLock<MetricsProcess> =
    LazyLock::new(MetricsProcess::default);

#[derive(Debug)]
pub struct MetricsProcess {
    sys: Mutex<System>,
    cpu_usage: Gauge,
    memory_usage: IntGauge,
    virtual_memory_usage: IntGauge,
    pid: Pid,
}

impl Default for MetricsProcess {
    fn default() -> Self {
        let mut sys = System::new_all();
        let pid = sysinfo::get_current_pid().expect("pid");
        sys.refresh_process(pid);

        MetricsProcess {
            sys: Mutex::new(sys),
            pid,
            cpu_usage: Gauge::new(
                "process_cpu_usage_percentage",
                "CPU usage of the process (%)",
            )
            .unwrap(),
            memory_usage: IntGauge::new(
                "process_memory_bytes",
                "Resident memory (bytes)",
            )
            .unwrap(),
            virtual_memory_usage: IntGauge::new(
                "process_virtual_memory_bytes",
                "Virtual memory (bytes)",
            )
            .unwrap(),
        }
    }
}

impl MetricsProcess {
    fn update_metrics(&self) {
        let mut sys = self.sys.lock().unwrap();
        // <-- no SystemExt import needed
        sys.refresh_process(self.pid);

        if let Some(process) = sys.process(self.pid) {
            // <-- no ProcessExt import needed
            self.cpu_usage.set(process.cpu_usage() as f64);
            self.memory_usage.set(process.memory() as i64);
            self.virtual_memory_usage
                .set(process.virtual_memory() as i64);
        }
    }

    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        self.update_metrics();

        let registry = Registry::new();
        registry
            .register(Box::new(self.cpu_usage.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        registry
            .register(Box::new(self.memory_usage.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        registry
            .register(Box::new(self.virtual_memory_usage.clone()))
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        let mut buffer = Vec::new();
        let encoder = TextEncoder::new();
        encoder
            .encode(&registry.gather(), &mut buffer)
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        Ok(String::from_utf8(buffer)?)
    }
}
