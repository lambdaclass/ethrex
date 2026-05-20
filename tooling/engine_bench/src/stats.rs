//! Statistical aggregation helpers.

/// Median of a non-empty sorted slice. Panics if empty.
pub fn median(sorted: &[u128]) -> u128 {
    assert!(!sorted.is_empty(), "median of empty slice");
    let n = sorted.len();
    if n.is_multiple_of(2) {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2
    } else {
        sorted[n / 2]
    }
}

/// p99 of a non-empty sorted slice. For small inputs, returns the maximum.
pub fn p99(sorted: &[u128]) -> u128 {
    assert!(!sorted.is_empty(), "p99 of empty slice");
    let n = sorted.len();
    let idx = ((n as f64 * 0.99).ceil() as usize)
        .saturating_sub(1)
        .min(n - 1);
    sorted[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_odd() {
        let v = vec![1u128, 2, 3, 4, 5];
        assert_eq!(median(&v), 3);
    }

    #[test]
    fn median_even() {
        let v = vec![1u128, 2, 3, 4];
        assert_eq!(median(&v), 2);
    }

    #[test]
    fn p99_small_returns_max() {
        let v = vec![1u128, 2, 3, 4, 5];
        assert_eq!(p99(&v), 5);
    }

    #[test]
    fn p99_large() {
        let v: Vec<u128> = (1..=100).collect();
        assert_eq!(p99(&v), 99);
    }
}
