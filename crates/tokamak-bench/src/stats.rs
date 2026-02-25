//! Statistical analysis for benchmark measurements.
//!
//! Computes mean, standard deviation, and 95% confidence intervals
//! from per-run duration samples.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Statistical summary of benchmark run durations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchStats {
    /// Arithmetic mean in nanoseconds.
    pub mean_ns: f64,
    /// Sample standard deviation in nanoseconds.
    pub stddev_ns: f64,
    /// Lower bound of 95% confidence interval (ns).
    pub ci_lower_ns: f64,
    /// Upper bound of 95% confidence interval (ns).
    pub ci_upper_ns: f64,
    /// Minimum duration observed (ns).
    pub min_ns: u128,
    /// Maximum duration observed (ns).
    pub max_ns: u128,
    /// Number of samples (after warmup exclusion).
    pub samples: usize,
}

/// Z-score for 95% confidence interval (two-tailed).
const Z_95: f64 = 1.96;

/// Compute statistics from a slice of durations.
///
/// Returns `None` if fewer than 2 samples (cannot compute stddev).
pub fn compute_stats(durations: &[Duration]) -> Option<BenchStats> {
    let n = durations.len();
    if n < 2 {
        return None;
    }

    let ns_values: Vec<f64> = durations.iter().map(|d| d.as_nanos() as f64).collect();
    let n_f = n as f64;

    let mean = ns_values.iter().sum::<f64>() / n_f;

    // Sample variance (Bessel's correction: n-1)
    let variance = ns_values.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n_f - 1.0);
    let stddev = variance.sqrt();

    // 95% CI margin = z * (stddev / sqrt(n))
    let ci_margin = Z_95 * stddev / n_f.sqrt();

    let min_ns = ns_values.iter().map(|x| *x as u128).min().unwrap_or(0);
    let max_ns = ns_values.iter().map(|x| *x as u128).max().unwrap_or(0);

    Some(BenchStats {
        mean_ns: mean,
        stddev_ns: stddev,
        ci_lower_ns: mean - ci_margin,
        ci_upper_ns: mean + ci_margin,
        min_ns,
        max_ns,
        samples: n,
    })
}

/// Split durations into warmup (discarded) and measured samples.
///
/// Returns only the measured portion (after warmup_count samples).
/// If warmup_count >= total, returns the last sample only.
pub fn split_warmup(durations: &[Duration], warmup_count: usize) -> &[Duration] {
    if warmup_count >= durations.len() {
        // Edge case: keep at least the last sample
        let start = durations.len().saturating_sub(1);
        &durations[start..]
    } else {
        &durations[warmup_count..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn test_compute_stats_basic() {
        let durations = vec![ms(100), ms(100), ms(100), ms(100)];
        let stats = compute_stats(&durations).expect("should compute stats");

        assert_eq!(stats.samples, 4);
        // Mean = 100ms = 100_000_000 ns
        let expected_mean = 100_000_000.0;
        assert!(
            (stats.mean_ns - expected_mean).abs() < 1.0,
            "mean should be ~100ms, got {}",
            stats.mean_ns
        );
        // Stddev = 0 for identical values
        assert!(
            stats.stddev_ns < 1.0,
            "stddev should be ~0, got {}",
            stats.stddev_ns
        );
        // CI should be tight
        assert!(
            (stats.ci_lower_ns - stats.ci_upper_ns).abs() < 1.0,
            "CI should be zero-width for constant data"
        );
    }

    #[test]
    fn test_compute_stats_variance() {
        // 10ms, 20ms, 30ms, 40ms, 50ms
        let durations = vec![ms(10), ms(20), ms(30), ms(40), ms(50)];
        let stats = compute_stats(&durations).expect("should compute stats");

        // Mean = 30ms
        let expected_mean = 30_000_000.0;
        assert!(
            (stats.mean_ns - expected_mean).abs() < 1.0,
            "mean should be 30ms"
        );

        // Sample stddev = sqrt(((10-30)^2 + (20-30)^2 + ... + (50-30)^2) / 4)
        //               = sqrt((400+100+0+100+400)*1e12 / 4) = sqrt(250e12) ≈ 15_811_388 ns
        let expected_stddev = 15_811_388.3;
        assert!(
            (stats.stddev_ns - expected_stddev).abs() < 1.0,
            "stddev should be ~15.8ms, got {}",
            stats.stddev_ns
        );

        // Min/max
        assert_eq!(stats.min_ns, 10_000_000);
        assert_eq!(stats.max_ns, 50_000_000);

        // CI should be wider than zero
        assert!(stats.ci_lower_ns < stats.mean_ns);
        assert!(stats.ci_upper_ns > stats.mean_ns);
    }

    #[test]
    fn test_compute_stats_too_few_samples() {
        let single = vec![ms(100)];
        assert!(
            compute_stats(&single).is_none(),
            "should return None for < 2 samples"
        );

        let empty: Vec<Duration> = vec![];
        assert!(
            compute_stats(&empty).is_none(),
            "should return None for empty"
        );
    }

    #[test]
    fn test_compute_stats_two_samples() {
        let durations = vec![ms(100), ms(200)];
        let stats = compute_stats(&durations).expect("should work with 2 samples");

        assert_eq!(stats.samples, 2);
        // Mean = 150ms
        let expected_mean = 150_000_000.0;
        assert!((stats.mean_ns - expected_mean).abs() < 1.0);
    }

    #[test]
    fn test_split_warmup_normal() {
        let durations = vec![ms(1), ms(2), ms(3), ms(4), ms(5)];
        let measured = split_warmup(&durations, 2);
        assert_eq!(measured.len(), 3);
        assert_eq!(measured[0], ms(3));
    }

    #[test]
    fn test_split_warmup_zero() {
        let durations = vec![ms(1), ms(2), ms(3)];
        let measured = split_warmup(&durations, 0);
        assert_eq!(measured.len(), 3);
    }

    #[test]
    fn test_split_warmup_exceeds() {
        let durations = vec![ms(1), ms(2)];
        let measured = split_warmup(&durations, 10);
        assert_eq!(measured.len(), 1, "should keep at least the last sample");
    }

    #[test]
    fn test_stats_serialization() {
        let stats = BenchStats {
            mean_ns: 100_000_000.0,
            stddev_ns: 5_000_000.0,
            ci_lower_ns: 96_040_000.0,
            ci_upper_ns: 103_960_000.0,
            min_ns: 95_000_000,
            max_ns: 108_000_000,
            samples: 10,
        };
        let json = serde_json::to_string(&stats).expect("serialize");
        let parsed: BenchStats = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.samples, 10);
        assert!((parsed.mean_ns - 100_000_000.0).abs() < 0.1);
    }
}
