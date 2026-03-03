//! Safe Rust wrapper around libmdbx.
//!
//! This crate provides a safe abstraction over the raw `ethrex-mdbx-sys` FFI
//! bindings. The `StorageBackend` implementation lives in `ethrex-storage`
//! (at `backend/mdbx.rs`) to avoid a cyclic dependency.

pub mod cursor;
pub mod env;
pub mod error;
pub mod txn;

pub use env::{EnvConfig, Environment};
pub use error::MdbxError;

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    const TEST_TABLES: &[&str] = &["test_table", "other_table"];

    /// Create a temporary directory and open an MDBX environment with test tables.
    fn open_test_env() -> (Environment, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ethrex_mdbx_test_{}", std::process::id()));
        // Clean up any leftover from a previous run
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let env = Environment::open(&dir, EnvConfig::default(), TEST_TABLES).unwrap();
        (env, dir)
    }

    fn cleanup(dir: &PathBuf) {
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn open_and_close() {
        let (env, dir) = open_test_env();
        drop(env);
        cleanup(&dir);
    }

    #[test]
    fn put_get_delete() {
        let (env, dir) = open_test_env();

        // Put a value
        {
            let txn = env.begin_rw_txn().unwrap();
            txn.put("test_table", b"key1", b"value1").unwrap();
            txn.commit().unwrap();
        }

        // Read it back
        {
            let txn = env.begin_ro_txn().unwrap();
            let val = txn.get("test_table", b"key1").unwrap();
            assert_eq!(val, Some(b"value1".to_vec()));
        }

        // Delete it
        {
            let txn = env.begin_rw_txn().unwrap();
            txn.del("test_table", b"key1").unwrap();
            txn.commit().unwrap();
        }

        // Verify it's gone
        {
            let txn = env.begin_ro_txn().unwrap();
            let val = txn.get("test_table", b"key1").unwrap();
            assert_eq!(val, None);
        }

        drop(env);
        cleanup(&dir);
    }

    #[test]
    fn get_nonexistent_key() {
        let (env, dir) = open_test_env();

        let txn = env.begin_ro_txn().unwrap();
        let val = txn.get("test_table", b"missing").unwrap();
        assert_eq!(val, None);

        drop(txn);
        drop(env);
        cleanup(&dir);
    }

    #[test]
    fn delete_nonexistent_key() {
        let (env, dir) = open_test_env();

        // Deleting a nonexistent key should succeed (not error)
        let txn = env.begin_rw_txn().unwrap();
        txn.del("test_table", b"missing").unwrap();
        txn.commit().unwrap();

        drop(env);
        cleanup(&dir);
    }

    #[test]
    fn transaction_isolation() {
        let (env, dir) = open_test_env();
        let env = std::sync::Arc::new(env);

        // Write initial data
        {
            let txn = env.begin_rw_txn().unwrap();
            txn.put("test_table", b"key1", b"v1").unwrap();
            txn.commit().unwrap();
        }

        // Open a read transaction (snapshot)
        let ro_txn = env.begin_ro_txn().unwrap();

        // Write new data from a different thread (MDBX forbids overlapping
        // RO + RW transactions on the same thread).
        let env2 = env.clone();
        std::thread::spawn(move || {
            let txn = env2.begin_rw_txn().unwrap();
            txn.put("test_table", b"key1", b"v2").unwrap();
            txn.put("test_table", b"key2", b"new").unwrap();
            txn.commit().unwrap();
        })
        .join()
        .unwrap();

        // The read transaction should still see the old data (snapshot isolation)
        assert_eq!(
            ro_txn.get("test_table", b"key1").unwrap(),
            Some(b"v1".to_vec())
        );
        assert_eq!(ro_txn.get("test_table", b"key2").unwrap(), None);

        drop(ro_txn);
        drop(env);
        cleanup(&dir);
    }

    #[test]
    fn abort_on_drop() {
        let (env, dir) = open_test_env();

        // Write but don't commit — transaction should be aborted on drop
        {
            let txn = env.begin_rw_txn().unwrap();
            txn.put("test_table", b"key1", b"value1").unwrap();
            // drop without commit
        }

        // Value should not be persisted
        {
            let txn = env.begin_ro_txn().unwrap();
            assert_eq!(txn.get("test_table", b"key1").unwrap(), None);
        }

        drop(env);
        cleanup(&dir);
    }

    #[test]
    fn clear_table() {
        let (env, dir) = open_test_env();

        // Insert some data
        {
            let txn = env.begin_rw_txn().unwrap();
            txn.put("test_table", b"a", b"1").unwrap();
            txn.put("test_table", b"b", b"2").unwrap();
            txn.put("test_table", b"c", b"3").unwrap();
            txn.commit().unwrap();
        }

        // Clear the table
        {
            let txn = env.begin_rw_txn().unwrap();
            txn.clear_table("test_table").unwrap();
            txn.commit().unwrap();
        }

        // All keys should be gone
        {
            let txn = env.begin_ro_txn().unwrap();
            assert_eq!(txn.get("test_table", b"a").unwrap(), None);
            assert_eq!(txn.get("test_table", b"b").unwrap(), None);
            assert_eq!(txn.get("test_table", b"c").unwrap(), None);
        }

        drop(env);
        cleanup(&dir);
    }

    #[test]
    fn multiple_tables() {
        let (env, dir) = open_test_env();

        // Write to both tables
        {
            let txn = env.begin_rw_txn().unwrap();
            txn.put("test_table", b"key", b"val_a").unwrap();
            txn.put("other_table", b"key", b"val_b").unwrap();
            txn.commit().unwrap();
        }

        // Read from both — same key, different values
        {
            let txn = env.begin_ro_txn().unwrap();
            assert_eq!(
                txn.get("test_table", b"key").unwrap(),
                Some(b"val_a".to_vec())
            );
            assert_eq!(
                txn.get("other_table", b"key").unwrap(),
                Some(b"val_b".to_vec())
            );
        }

        drop(env);
        cleanup(&dir);
    }

    #[test]
    fn upsert_overwrites() {
        let (env, dir) = open_test_env();

        {
            let txn = env.begin_rw_txn().unwrap();
            txn.put("test_table", b"key", b"first").unwrap();
            txn.commit().unwrap();
        }

        {
            let txn = env.begin_rw_txn().unwrap();
            txn.put("test_table", b"key", b"second").unwrap();
            txn.commit().unwrap();
        }

        {
            let txn = env.begin_ro_txn().unwrap();
            assert_eq!(
                txn.get("test_table", b"key").unwrap(),
                Some(b"second".to_vec())
            );
        }

        drop(env);
        cleanup(&dir);
    }

    #[test]
    fn prefix_iteration() {
        let (env, dir) = open_test_env();

        // Insert keys with different prefixes
        {
            let txn = env.begin_rw_txn().unwrap();
            txn.put("test_table", b"abc:1", b"v1").unwrap();
            txn.put("test_table", b"abc:2", b"v2").unwrap();
            txn.put("test_table", b"abc:3", b"v3").unwrap();
            txn.put("test_table", b"abd:1", b"other").unwrap();
            txn.put("test_table", b"abb:0", b"before").unwrap();
            txn.commit().unwrap();
        }

        // Iterate with prefix "abc:"
        {
            let txn = env.begin_ro_txn().unwrap();
            let cursor = txn.cursor("test_table").unwrap();
            let results: Vec<_> = cursor
                .prefix_iter(b"abc:")
                .collect::<Result<Vec<_>, _>>()
                .unwrap();

            assert_eq!(results.len(), 3);
            assert_eq!(&*results[0].0, b"abc:1");
            assert_eq!(&*results[0].1, b"v1");
            assert_eq!(&*results[1].0, b"abc:2");
            assert_eq!(&*results[1].1, b"v2");
            assert_eq!(&*results[2].0, b"abc:3");
            assert_eq!(&*results[2].1, b"v3");
        }

        drop(env);
        cleanup(&dir);
    }

    #[test]
    fn prefix_iteration_empty() {
        let (env, dir) = open_test_env();

        // Empty table — prefix iteration should yield nothing
        {
            let txn = env.begin_ro_txn().unwrap();
            let cursor = txn.cursor("test_table").unwrap();
            let results: Vec<_> = cursor
                .prefix_iter(b"anything")
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(results.is_empty());
        }

        drop(env);
        cleanup(&dir);
    }

    #[test]
    fn prefix_iteration_no_match() {
        let (env, dir) = open_test_env();

        {
            let txn = env.begin_rw_txn().unwrap();
            txn.put("test_table", b"xyz:1", b"v1").unwrap();
            txn.commit().unwrap();
        }

        // Prefix that doesn't match any key
        {
            let txn = env.begin_ro_txn().unwrap();
            let cursor = txn.cursor("test_table").unwrap();
            let results: Vec<_> = cursor
                .prefix_iter(b"abc:")
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(results.is_empty());
        }

        drop(env);
        cleanup(&dir);
    }

    #[test]
    fn unknown_table_errors() {
        let (env, dir) = open_test_env();

        let txn = env.begin_ro_txn().unwrap();
        let result = txn.get("nonexistent_table", b"key");
        assert!(result.is_err());

        drop(txn);
        drop(env);
        cleanup(&dir);
    }

    #[test]
    fn copy_checkpoint() {
        let (env, dir) = open_test_env();

        // Write data
        {
            let txn = env.begin_rw_txn().unwrap();
            txn.put("test_table", b"key", b"value").unwrap();
            txn.commit().unwrap();
        }

        // Copy to a checkpoint directory (must not pre-exist — MDBX creates it)
        let checkpoint_dir = dir.join("checkpoint");
        env.copy_to(&checkpoint_dir).unwrap();

        // Open the checkpoint and verify data
        let env2 = Environment::open(&checkpoint_dir, EnvConfig::default(), TEST_TABLES).unwrap();
        {
            let txn = env2.begin_ro_txn().unwrap();
            assert_eq!(
                txn.get("test_table", b"key").unwrap(),
                Some(b"value".to_vec())
            );
        }

        drop(env2);
        drop(env);
        cleanup(&dir);
    }
}
