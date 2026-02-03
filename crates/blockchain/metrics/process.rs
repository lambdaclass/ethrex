use prometheus::{Encoder, IntGauge, Registry, TextEncoder};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{LazyLock, OnceLock},
};

use crate::MetricsError;

pub static METRICS_PROCESS: LazyLock<MetricsProcess> = LazyLock::new(MetricsProcess::default);
static DATADIR_PATH: OnceLock<PathBuf> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct MetricsProcess;

impl Default for MetricsProcess {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsProcess {
    pub fn new() -> Self {
        MetricsProcess
    }

    /// The Process collector gathers standard process metrics (CPU time, RSS, VSZ, FDs, threads, start_time).
    /// But it only works on Linux. This is an initial implementation.
    pub fn gather_metrics(&self) -> Result<String, MetricsError> {
        let r = Registry::new();

        // Register Prometheus' built-in Linux process metrics
        #[cfg(target_os = "linux")]
        {
            use prometheus::process_collector::ProcessCollector;
            r.register(Box::new(ProcessCollector::for_self()))
                .map_err(|e| {
                    MetricsError::PrometheusErr(format!(
                        "Failed to register process collector: {}",
                        e
                    ))
                })?;
        }

        if let Some(path) = DATADIR_PATH.get()
            && let Ok(size) = directory_size(path)
        {
            let gauge = IntGauge::new(
                "datadir_size_bytes",
                "Total size in bytes consumed by the configured datadir.",
            )
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
            let clamped = size.min(i64::MAX as u64);
            gauge.set(clamped as i64);
            r.register(Box::new(gauge))
                .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;
        }

        let encoder = TextEncoder::new();
        let metric_families = r.gather();

        let mut buffer = Vec::new();
        encoder
            .encode(&metric_families, &mut buffer)
            .map_err(|e| MetricsError::PrometheusErr(e.to_string()))?;

        let res = String::from_utf8(buffer)?;
        Ok(res)
    }
}

pub fn set_datadir_path(path: PathBuf) {
    let _ = DATADIR_PATH.set(path);
}

fn directory_size(root: &Path) -> io::Result<u64> {
    let mut total = 0;
    let mut stack = vec![root.to_path_buf()];

    while let Some(path) = stack.pop() {
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };

        for entry in entries {
            let entry = entry?;
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            };

            if metadata.is_dir() {
                stack.push(entry.path());
            } else {
                total += metadata.len();
            }
        }
    }

    Ok(total)
}
