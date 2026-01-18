//! Memory limit validation tests for snap sync
//!
//! These tests verify that memory threshold calculations work correctly
//! under various simulated memory conditions without requiring actual
//! OS memory constraints.

#[cfg(test)]
mod memory_limit_tests {
    // Constants matching sync.rs (duplicated here for test isolation)
    const DEFAULT_FLUSH_THRESHOLD: usize = 500_000;
    const MIN_FLUSH_THRESHOLD: usize = 250_000;
    const MAX_FLUSH_THRESHOLD: usize = 3_000_000;
    const BYTES_PER_STORAGE_SLOT: usize = 400;
    const MEMORY_USAGE_PERCENT: usize = 5;
    const BYTES_PER_STORAGE_FILE: usize = 400 * 1024 * 1024;
    const BYTES_PER_ACCOUNT_FILE: usize = 200 * 1024 * 1024;
    const MIN_FILE_BATCH_SIZE: usize = 2;
    const MAX_FILE_BATCH_SIZE: usize = 32;
    const DEFAULT_FILE_BATCH_SIZE: usize = 4;

    /// Calculate flush threshold with provided memory value (for testing)
    fn calculate_flush_threshold_with_memory(available_bytes: Option<usize>) -> usize {
        if let Some(available_bytes) = available_bytes {
            let target_bytes = available_bytes * MEMORY_USAGE_PERCENT / 100;
            let threshold = target_bytes / BYTES_PER_STORAGE_SLOT;
            threshold.clamp(MIN_FLUSH_THRESHOLD, MAX_FLUSH_THRESHOLD)
        } else {
            DEFAULT_FLUSH_THRESHOLD
        }
    }

    /// Calculate file batch size with provided memory value (for testing)
    fn calculate_file_batch_size_with_memory(
        available_bytes: Option<usize>,
        bytes_per_file: usize,
    ) -> usize {
        if let Some(available_bytes) = available_bytes {
            let target_bytes = available_bytes * MEMORY_USAGE_PERCENT / 100;
            let batch_size = target_bytes / bytes_per_file;
            batch_size.clamp(MIN_FILE_BATCH_SIZE, MAX_FILE_BATCH_SIZE)
        } else {
            DEFAULT_FILE_BATCH_SIZE
        }
    }

    // ===== FLUSH THRESHOLD TESTS =====

    #[test]
    fn test_flush_threshold_low_memory_hits_minimum() {
        // 500MB available - should hit MIN_FLUSH_THRESHOLD
        // 500MB * 5% = 25MB / 400 bytes = 62,500 slots < MIN (250,000)
        let available = 500 * 1024 * 1024; // 500MB
        let threshold = calculate_flush_threshold_with_memory(Some(available));
        assert_eq!(
            threshold, MIN_FLUSH_THRESHOLD,
            "Low memory ({} bytes) should hit MIN_FLUSH_THRESHOLD",
            available
        );
    }

    #[test]
    fn test_flush_threshold_high_memory_hits_maximum() {
        // 100GB available - should hit MAX_FLUSH_THRESHOLD
        // 100GB * 5% = 5GB / 400 bytes = 12.5M slots > MAX (3M)
        let available = 100 * 1024 * 1024 * 1024; // 100GB
        let threshold = calculate_flush_threshold_with_memory(Some(available));
        assert_eq!(
            threshold, MAX_FLUSH_THRESHOLD,
            "High memory ({} bytes) should hit MAX_FLUSH_THRESHOLD",
            available
        );
    }

    #[test]
    fn test_flush_threshold_normal_memory_in_range() {
        // 16GB available - should be between MIN and MAX
        // 16GB * 5% = 819MB / 400 bytes = ~2M slots (between 250k and 3M)
        let available = 16 * 1024 * 1024 * 1024; // 16GB
        let threshold = calculate_flush_threshold_with_memory(Some(available));
        assert!(
            threshold >= MIN_FLUSH_THRESHOLD && threshold <= MAX_FLUSH_THRESHOLD,
            "Normal memory ({} bytes) should produce threshold {} in range [{}, {}]",
            available,
            threshold,
            MIN_FLUSH_THRESHOLD,
            MAX_FLUSH_THRESHOLD
        );

        // Verify expected value: 16GB * 5% / 400 = 2,097,152
        let expected = (16 * 1024 * 1024 * 1024 * MEMORY_USAGE_PERCENT / 100) / BYTES_PER_STORAGE_SLOT;
        assert_eq!(threshold, expected);
    }

    #[test]
    fn test_flush_threshold_unavailable_returns_default() {
        let threshold = calculate_flush_threshold_with_memory(None);
        assert_eq!(
            threshold, DEFAULT_FLUSH_THRESHOLD,
            "Unavailable memory should return DEFAULT_FLUSH_THRESHOLD"
        );
    }

    #[test]
    fn test_flush_threshold_zero_memory() {
        // Edge case: 0 bytes available
        let threshold = calculate_flush_threshold_with_memory(Some(0));
        assert_eq!(
            threshold, MIN_FLUSH_THRESHOLD,
            "Zero memory should hit MIN_FLUSH_THRESHOLD"
        );
    }

    #[test]
    fn test_flush_threshold_exact_min_boundary() {
        // Calculate exact memory needed for MIN_FLUSH_THRESHOLD
        // threshold = (available * 5%) / 400
        // MIN = (available * 5%) / 400
        // available = MIN * 400 * 100 / 5 = MIN * 8000
        let exact_min_memory = MIN_FLUSH_THRESHOLD * BYTES_PER_STORAGE_SLOT * 100 / MEMORY_USAGE_PERCENT;
        let threshold = calculate_flush_threshold_with_memory(Some(exact_min_memory));
        assert_eq!(
            threshold, MIN_FLUSH_THRESHOLD,
            "Exact boundary memory should produce MIN_FLUSH_THRESHOLD"
        );

        // Just below should still be MIN (clamped)
        let below_min = exact_min_memory - 1;
        let threshold_below = calculate_flush_threshold_with_memory(Some(below_min));
        assert_eq!(threshold_below, MIN_FLUSH_THRESHOLD);
    }

    #[test]
    fn test_flush_threshold_exact_max_boundary() {
        // Calculate exact memory needed for MAX_FLUSH_THRESHOLD
        let exact_max_memory = MAX_FLUSH_THRESHOLD * BYTES_PER_STORAGE_SLOT * 100 / MEMORY_USAGE_PERCENT;
        let threshold = calculate_flush_threshold_with_memory(Some(exact_max_memory));
        assert_eq!(
            threshold, MAX_FLUSH_THRESHOLD,
            "Exact boundary memory should produce MAX_FLUSH_THRESHOLD"
        );

        // Just above should still be MAX (clamped)
        let above_max = exact_max_memory + 1_000_000;
        let threshold_above = calculate_flush_threshold_with_memory(Some(above_max));
        assert_eq!(threshold_above, MAX_FLUSH_THRESHOLD);
    }

    // ===== FILE BATCH SIZE TESTS =====

    #[test]
    fn test_file_batch_size_low_memory_hits_minimum() {
        // 200MB available for storage files (~400MB each)
        // 200MB * 5% = 10MB / 400MB = 0.025 < MIN (2)
        let available = 200 * 1024 * 1024; // 200MB
        let batch_size = calculate_file_batch_size_with_memory(Some(available), BYTES_PER_STORAGE_FILE);
        assert_eq!(
            batch_size, MIN_FILE_BATCH_SIZE,
            "Low memory should hit MIN_FILE_BATCH_SIZE for storage files"
        );
    }

    #[test]
    fn test_file_batch_size_high_memory_hits_maximum() {
        // 500GB available
        // 500GB * 5% = 25GB / 400MB = 62.5 > MAX (32)
        let available = 500 * 1024 * 1024 * 1024; // 500GB
        let batch_size = calculate_file_batch_size_with_memory(Some(available), BYTES_PER_STORAGE_FILE);
        assert_eq!(
            batch_size, MAX_FILE_BATCH_SIZE,
            "High memory should hit MAX_FILE_BATCH_SIZE"
        );
    }

    #[test]
    fn test_file_batch_size_normal_memory_in_range() {
        // 32GB available for storage files
        // 32GB * 5% = 1.6GB / 400MB = 4 files
        let available = 32 * 1024 * 1024 * 1024; // 32GB
        let batch_size = calculate_file_batch_size_with_memory(Some(available), BYTES_PER_STORAGE_FILE);
        assert!(
            batch_size >= MIN_FILE_BATCH_SIZE && batch_size <= MAX_FILE_BATCH_SIZE,
            "Normal memory should produce batch size {} in range [{}, {}]",
            batch_size,
            MIN_FILE_BATCH_SIZE,
            MAX_FILE_BATCH_SIZE
        );
    }

    #[test]
    fn test_file_batch_size_unavailable_returns_default() {
        let batch_size = calculate_file_batch_size_with_memory(None, BYTES_PER_STORAGE_FILE);
        assert_eq!(
            batch_size, DEFAULT_FILE_BATCH_SIZE,
            "Unavailable memory should return DEFAULT_FILE_BATCH_SIZE"
        );
    }

    #[test]
    fn test_file_batch_size_account_vs_storage_files() {
        // Account files are smaller (200MB vs 400MB), so same memory = more files
        let available = 32 * 1024 * 1024 * 1024; // 32GB

        let storage_batch = calculate_file_batch_size_with_memory(Some(available), BYTES_PER_STORAGE_FILE);
        let account_batch = calculate_file_batch_size_with_memory(Some(available), BYTES_PER_ACCOUNT_FILE);

        assert!(
            account_batch >= storage_batch,
            "Account batch size ({}) should be >= storage batch size ({}) for same memory",
            account_batch,
            storage_batch
        );
    }

    #[test]
    fn test_file_batch_size_zero_memory() {
        let batch_size = calculate_file_batch_size_with_memory(Some(0), BYTES_PER_STORAGE_FILE);
        assert_eq!(
            batch_size, MIN_FILE_BATCH_SIZE,
            "Zero memory should hit MIN_FILE_BATCH_SIZE"
        );
    }

    // ===== CONSISTENCY TESTS =====

    #[test]
    fn test_memory_percentage_is_conservative() {
        // Verify MEMORY_USAGE_PERCENT is reasonably conservative (<=10%)
        assert!(
            MEMORY_USAGE_PERCENT <= 10,
            "MEMORY_USAGE_PERCENT ({}) should be conservative (<=10%)",
            MEMORY_USAGE_PERCENT
        );
    }

    #[test]
    fn test_constants_are_reasonable() {
        // Sanity checks on constant values
        assert!(MIN_FLUSH_THRESHOLD < DEFAULT_FLUSH_THRESHOLD);
        assert!(DEFAULT_FLUSH_THRESHOLD < MAX_FLUSH_THRESHOLD);
        assert!(MIN_FILE_BATCH_SIZE < DEFAULT_FILE_BATCH_SIZE);
        assert!(DEFAULT_FILE_BATCH_SIZE < MAX_FILE_BATCH_SIZE);
        assert!(BYTES_PER_ACCOUNT_FILE < BYTES_PER_STORAGE_FILE);
    }

    #[test]
    fn test_threshold_memory_estimate_accuracy() {
        // Verify that the memory estimate at MAX_FLUSH_THRESHOLD is reasonable
        // MAX_FLUSH_THRESHOLD * BYTES_PER_STORAGE_SLOT should be ~1.2GB
        let max_memory_estimate = MAX_FLUSH_THRESHOLD * BYTES_PER_STORAGE_SLOT;
        let max_memory_mb = max_memory_estimate / 1024 / 1024;

        assert!(
            max_memory_mb >= 1000 && max_memory_mb <= 2000,
            "MAX threshold memory estimate ({} MB) should be ~1.2GB",
            max_memory_mb
        );
    }
}
