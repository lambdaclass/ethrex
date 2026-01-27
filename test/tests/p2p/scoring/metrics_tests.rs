use std::time::Duration;

use ethrex_p2p::scoring::{EWMA, LatencyTracker, RequestTypeMetrics, ThroughputTracker};

#[test]
fn test_ewma_basic() {
    let mut ewma = EWMA::new(0.5);

    assert!(ewma.value().is_none());

    ewma.update(100.0);
    assert_eq!(ewma.value(), Some(100.0));

    ewma.update(200.0);
    // 0.5 * 200 + 0.5 * 100 = 150
    assert_eq!(ewma.value(), Some(150.0));
}

#[test]
fn test_ewma_convergence() {
    let mut ewma = EWMA::new(0.2);

    // Feed constant values, should converge
    for _ in 0..100 {
        ewma.update(100.0);
    }

    // Should be very close to 100
    assert!((ewma.value().unwrap() - 100.0).abs() < 0.01);
}

#[test]
fn test_latency_tracker_percentiles() {
    let mut tracker = LatencyTracker::new(100);

    // Add samples 1..=100 ms
    for i in 1..=100 {
        tracker.record(Duration::from_millis(i));
    }

    // p50 should be around 50ms
    let p50 = tracker.p50().unwrap().as_millis();
    assert!((45..=55).contains(&p50), "p50 was {}", p50);

    // p95 should be around 95ms
    let p95 = tracker.p95().unwrap().as_millis();
    assert!((90..=100).contains(&p95), "p95 was {}", p95);

    // p99 should be around 99ms
    let p99 = tracker.p99().unwrap().as_millis();
    assert!((95..=100).contains(&p99), "p99 was {}", p99);
}

#[test]
fn test_latency_tracker_circular_buffer() {
    let mut tracker = LatencyTracker::new(10);

    // Add more samples than buffer size
    for i in 1..=20 {
        tracker.record(Duration::from_millis(i));
    }

    // Should only have last 10 samples (11-20)
    assert_eq!(tracker.sample_count(), 10);

    // p50 should be around 15ms (median of 11-20)
    let p50 = tracker.p50().unwrap().as_millis();
    assert!((14..=16).contains(&p50), "p50 was {}", p50);
}

#[test]
fn test_throughput_tracker() {
    let mut tracker = ThroughputTracker::new();

    // 1MB in 1 second = 1 MB/s
    tracker.record(1024 * 1024, Duration::from_secs(1));

    let mbps = tracker.mb_per_sec().unwrap();
    assert!((mbps - 1.0).abs() < 0.01, "mbps was {}", mbps);
}

#[test]
fn test_request_type_metrics() {
    let mut metrics = RequestTypeMetrics::new();

    // Record some successes
    metrics.record_success(Duration::from_millis(100), Some(1024));
    metrics.record_success(Duration::from_millis(150), Some(2048));

    // Record a failure
    metrics.record_failure();

    assert_eq!(metrics.successes, 2);
    assert_eq!(metrics.failures, 1);
    assert_eq!(metrics.total_requests(), 3);

    // Success rate should be 2/3
    let rate = metrics.success_rate();
    assert!((rate - 0.666).abs() < 0.01, "rate was {}", rate);
}

#[test]
fn test_request_type_metrics_empty() {
    let metrics = RequestTypeMetrics::new();

    // Empty metrics should return neutral success rate
    assert_eq!(metrics.success_rate(), 0.5);
}
