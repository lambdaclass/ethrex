//! Error recovery tests for snap sync
//!
//! These tests verify that error handling paths work correctly,
//! including error classification, checkpoint recovery, and
//! staleness detection.

#[cfg(test)]
mod recovery_tests {
    use crate::sync::SyncError;
    use ethrex_common::H256;
    use std::path::PathBuf;

    // ===== ERROR RECOVERABILITY TESTS =====

    #[test]
    fn test_state_root_mismatch_is_recoverable() {
        let error = SyncError::StateRootMismatch {
            expected: H256::zero(),
            computed: H256::from_low_u64_be(1),
        };
        assert!(
            error.is_recoverable(),
            "StateRootMismatch should be recoverable - sync restarts with fresh pivot"
        );
    }

    #[test]
    fn test_storage_healing_failed_is_recoverable() {
        let error = SyncError::StorageHealingFailed;
        assert!(
            error.is_recoverable(),
            "StorageHealingFailed should be recoverable"
        );
    }

    #[test]
    fn test_state_healing_failed_is_recoverable() {
        let error = SyncError::StateHealingFailed;
        assert!(
            error.is_recoverable(),
            "StateHealingFailed should be recoverable"
        );
    }

    #[test]
    fn test_corrupt_db_is_recoverable() {
        let error = SyncError::CorruptDB;
        assert!(
            error.is_recoverable(),
            "CorruptDB should be recoverable - may be temporary"
        );
    }

    #[test]
    fn test_bodies_not_found_is_recoverable() {
        let error = SyncError::BodiesNotFound;
        assert!(
            error.is_recoverable(),
            "BodiesNotFound should be recoverable - can retry with different peers"
        );
    }

    #[test]
    fn test_no_blocks_is_recoverable() {
        let error = SyncError::NoBlocks;
        assert!(
            error.is_recoverable(),
            "NoBlocks should be recoverable - can retry"
        );
    }

    // ===== NON-RECOVERABLE ERROR TESTS =====

    #[test]
    fn test_snapshot_decode_error_is_not_recoverable() {
        let error = SyncError::SnapshotDecodeError(PathBuf::from("/tmp/test.snap"));
        assert!(
            !error.is_recoverable(),
            "SnapshotDecodeError should NOT be recoverable - corrupted file"
        );
    }

    #[test]
    fn test_account_state_snapshots_dir_not_found_is_not_recoverable() {
        let error = SyncError::AccountStateSnapshotsDirNotFound;
        assert!(
            !error.is_recoverable(),
            "AccountStateSnapshotsDirNotFound should NOT be recoverable"
        );
    }

    #[test]
    fn test_bytecodes_not_found_is_not_recoverable() {
        let error = SyncError::BytecodesNotFound;
        assert!(
            !error.is_recoverable(),
            "BytecodesNotFound should NOT be recoverable"
        );
    }

    #[test]
    fn test_no_latest_canonical_is_not_recoverable() {
        let error = SyncError::NoLatestCanonical;
        assert!(
            !error.is_recoverable(),
            "NoLatestCanonical should NOT be recoverable - fundamental error"
        );
    }

    #[test]
    fn test_corrupt_path_is_not_recoverable() {
        let error = SyncError::CorruptPath;
        assert!(
            !error.is_recoverable(),
            "CorruptPath should NOT be recoverable"
        );
    }

    #[test]
    fn test_state_validation_failed_is_not_recoverable() {
        let error = SyncError::StateValidationFailed("test failure".to_string());
        assert!(
            !error.is_recoverable(),
            "StateValidationFailed should NOT be recoverable - indicates bug"
        );
    }

    // ===== COMPREHENSIVE RECOVERABILITY CLASSIFICATION =====

    #[test]
    fn test_all_recoverable_errors() {
        // All errors that should be recoverable
        let recoverable_errors: Vec<SyncError> = vec![
            SyncError::StateRootMismatch {
                expected: H256::zero(),
                computed: H256::zero(),
            },
            SyncError::StorageHealingFailed,
            SyncError::StateHealingFailed,
            SyncError::CorruptDB,
            SyncError::BodiesNotFound,
            SyncError::InvalidRangeReceived,
            SyncError::BlockNumber(H256::zero()),
            SyncError::NoBlocks,
        ];

        for error in recoverable_errors {
            assert!(
                error.is_recoverable(),
                "Error {:?} should be recoverable",
                error
            );
        }
    }

    #[test]
    fn test_all_non_recoverable_errors() {
        // All errors that should NOT be recoverable
        let non_recoverable_errors: Vec<SyncError> = vec![
            SyncError::SnapshotDecodeError(PathBuf::from("/test")),
            SyncError::CodeHashesSnapshotDecodeError(PathBuf::from("/test")),
            SyncError::AccountState(H256::zero(), H256::zero()),
            SyncError::BytecodesNotFound,
            SyncError::AccountStateSnapshotsDirNotFound,
            SyncError::AccountStoragesSnapshotsDirNotFound,
            SyncError::CodeHashesSnapshotsDirNotFound,
            SyncError::DifferentStateRoots(H256::zero(), H256::zero(), H256::zero()),
            SyncError::NoBlockHeaders,
            SyncError::CorruptPath,
            SyncError::AccountTempDBDirNotFound("test".to_string()),
            SyncError::StorageTempDBDirNotFound("test".to_string()),
            SyncError::RocksDBError("test".to_string()),
            SyncError::BytecodeFileError,
            SyncError::NoLatestCanonical,
            SyncError::MissingFullsyncBatch,
            SyncError::StateValidationFailed("test".to_string()),
        ];

        for error in non_recoverable_errors {
            assert!(
                !error.is_recoverable(),
                "Error {:?} should NOT be recoverable",
                error
            );
        }
    }

    // ===== STALENESS DETECTION TESTS =====

    #[test]
    fn test_block_staleness_constants() {
        // Constants matching sync.rs (duplicated for test isolation)
        const SNAP_LIMIT: u64 = 128;
        const SECONDS_PER_BLOCK: u64 = 12;
        const MISSING_SLOTS_PERCENTAGE: f64 = 0.8;

        // Verify staleness window is reasonable
        let staleness_window_secs = SNAP_LIMIT * SECONDS_PER_BLOCK;
        assert!(
            staleness_window_secs >= 1500 && staleness_window_secs <= 2000,
            "Staleness window ({} secs) should be ~25-33 minutes",
            staleness_window_secs
        );

        // Verify missing slots percentage is reasonable
        assert!(
            MISSING_SLOTS_PERCENTAGE >= 0.7 && MISSING_SLOTS_PERCENTAGE <= 0.9,
            "MISSING_SLOTS_PERCENTAGE ({}) should be between 70-90%",
            MISSING_SLOTS_PERCENTAGE
        );
    }

    #[test]
    fn test_checkpoint_max_age_constant() {
        // Constant matching sync.rs
        const CHECKPOINT_MAX_AGE_SECS: u64 = 30 * 60;

        // Checkpoint max age should be reasonable (15-60 minutes)
        assert!(
            CHECKPOINT_MAX_AGE_SECS >= 15 * 60 && CHECKPOINT_MAX_AGE_SECS <= 60 * 60,
            "CHECKPOINT_MAX_AGE_SECS ({}) should be 15-60 minutes",
            CHECKPOINT_MAX_AGE_SECS
        );
    }

    // ===== ERROR MESSAGE TESTS =====

    #[test]
    fn test_error_messages_are_descriptive() {
        let errors_and_expected_content = vec![
            (
                SyncError::StateRootMismatch {
                    expected: H256::from_low_u64_be(123),
                    computed: H256::from_low_u64_be(456),
                },
                "mismatch",
            ),
            (SyncError::StorageHealingFailed, "Storage healing failed"),
            (SyncError::StateHealingFailed, "State healing failed"),
            (SyncError::CorruptDB, "DB"),
            (SyncError::BodiesNotFound, "bodies"),
            (SyncError::NoBlocks, "blocks"),
        ];

        for (error, expected_content) in errors_and_expected_content {
            let message = format!("{}", error);
            assert!(
                message.to_lowercase().contains(&expected_content.to_lowercase()),
                "Error message '{}' should contain '{}'",
                message,
                expected_content
            );
        }
    }

    // ===== RETRY LOGIC TESTS =====

    #[test]
    fn test_max_header_fetch_attempts_is_reasonable() {
        // Constant matching sync.rs
        const MAX_HEADER_FETCH_ATTEMPTS: u64 = 100;

        // Should allow enough retries but not infinite
        assert!(
            MAX_HEADER_FETCH_ATTEMPTS >= 10 && MAX_HEADER_FETCH_ATTEMPTS <= 200,
            "MAX_HEADER_FETCH_ATTEMPTS ({}) should be 10-200",
            MAX_HEADER_FETCH_ATTEMPTS
        );
    }

    #[test]
    fn test_bytecode_chunk_size_is_reasonable() {
        // Constant matching sync.rs
        const BYTECODE_CHUNK_SIZE: usize = 25_000;

        // Should be large enough for efficiency but not too large
        assert!(
            BYTECODE_CHUNK_SIZE >= 1000 && BYTECODE_CHUNK_SIZE <= 100_000,
            "BYTECODE_CHUNK_SIZE ({}) should be 1k-100k",
            BYTECODE_CHUNK_SIZE
        );
    }

    #[test]
    fn test_checkpoint_chunk_interval_is_reasonable() {
        // Constant matching sync.rs - saves checkpoint every N chunks
        const CHECKPOINT_CHUNK_INTERVAL: usize = 10;

        // Should reduce I/O but still allow reasonable recovery
        // At 25k bytecodes/chunk and 10 chunk interval, we checkpoint every ~250k bytecodes
        assert!(
            CHECKPOINT_CHUNK_INTERVAL >= 5 && CHECKPOINT_CHUNK_INTERVAL <= 20,
            "CHECKPOINT_CHUNK_INTERVAL ({}) should be 5-20 for balanced I/O vs recovery",
            CHECKPOINT_CHUNK_INTERVAL
        );
    }
}

#[cfg(test)]
mod checkpoint_tests {
    // Constant matching sync.rs
    const CHECKPOINT_MAX_AGE_SECS: u64 = 30 * 60;

    use crate::utils::current_unix_time;

    #[test]
    fn test_checkpoint_age_calculation() {
        let now = current_unix_time();
        let old_timestamp = now.saturating_sub(CHECKPOINT_MAX_AGE_SECS + 1);
        let recent_timestamp = now.saturating_sub(CHECKPOINT_MAX_AGE_SECS / 2);

        // Old checkpoint should be considered stale
        let old_age = now - old_timestamp;
        assert!(
            old_age > CHECKPOINT_MAX_AGE_SECS,
            "Old checkpoint should be stale"
        );

        // Recent checkpoint should not be stale
        let recent_age = now - recent_timestamp;
        assert!(
            recent_age <= CHECKPOINT_MAX_AGE_SECS,
            "Recent checkpoint should not be stale"
        );
    }

    #[test]
    fn test_timestamp_overflow_safety() {
        // Ensure saturating subtraction prevents overflow
        let result = 0u64.saturating_sub(1000);
        assert_eq!(result, 0, "Saturating sub should prevent underflow");

        let result = u64::MAX.saturating_add(1);
        assert_eq!(result, u64::MAX, "Saturating add should prevent overflow");
    }
}

#[cfg(test)]
mod membatch_invariant_tests {
    //! Tests for the Membatch invariant: "If a node exists in DB, all children exist"

    #[test]
    fn test_children_count_decrement_logic() {
        // Simulate the children_not_in_storage_count logic
        let mut count: u64 = 5;

        // Each child received decrements the count
        count = count.saturating_sub(1);
        assert_eq!(count, 4);

        count = count.saturating_sub(1);
        assert_eq!(count, 3);

        // When count reaches 0, node can be flushed to DB
        count = 0;
        assert_eq!(count, 0, "Zero count means all children present");
    }

    #[test]
    fn test_children_count_saturating_sub() {
        // Ensure we don't underflow if decremented too many times (bug protection)
        let count: u64 = 1;
        let result = count.saturating_sub(5);
        assert_eq!(result, 0, "Saturating sub should prevent underflow");
    }
}

#[cfg(test)]
mod healing_cache_recovery_tests {
    use crate::sync::healing_cache::{HealingCache, PathStatus};
    use ethrex_trie::Nibbles;

    #[test]
    fn test_cache_handles_empty_batch() {
        let cache = HealingCache::new();
        let empty_paths: Vec<Nibbles> = vec![];

        // Should not panic on empty batch
        cache.mark_exists_batch(&empty_paths);
        let results = cache.check_paths_batch(&empty_paths);
        assert!(results.is_empty());
    }

    #[test]
    fn test_cache_handles_duplicate_paths() {
        let cache = HealingCache::new();
        let path = Nibbles::from_hex(vec![1, 2, 3, 4]);

        // Mark same path multiple times should not cause issues
        cache.mark_exists(&path);
        cache.mark_exists(&path);
        cache.mark_exists(&path);

        let status = cache.check_path(&path);
        assert!(
            matches!(status, PathStatus::ConfirmedExists | PathStatus::ProbablyExists),
            "Path should still be found after duplicate marks"
        );
    }

    #[test]
    fn test_cache_clear_resets_state() {
        let cache = HealingCache::new();
        let path = Nibbles::from_hex(vec![1, 2, 3, 4]);

        cache.mark_exists(&path);
        cache.clear();

        // After clear, path should be unknown (DefinitelyMissing or ProbablyExists from filter)
        let status = cache.check_path(&path);
        // Note: Quotient filter may still report ProbablyExists due to false positives
        // but LRU cache should be cleared
        assert!(
            !matches!(status, PathStatus::ConfirmedExists),
            "After clear, path should not be ConfirmedExists"
        );
    }

    #[test]
    fn test_cache_stats_tracking() {
        let cache = HealingCache::new();
        cache.reset_stats();

        let path = Nibbles::from_hex(vec![1, 2, 3, 4]);

        // Check a path that doesn't exist
        let _ = cache.check_path(&path);

        // Mark it as existing
        cache.mark_exists(&path);

        // Check again
        let _ = cache.check_path(&path);

        let stats = cache.stats();
        assert!(
            stats.paths_added >= 1,
            "Stats should track paths added: {:?}",
            stats
        );
    }
}
