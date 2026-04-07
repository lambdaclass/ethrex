//! Async file system utilities for snap sync
//!
//! This module provides async wrappers around file system operations to avoid
//! blocking the tokio runtime during disk I/O.
//!
//! # Why Async File I/O Matters
//!
//! During snap sync, we perform many file operations (writing snapshots, reading them back,
//! cleanup). If these operations are synchronous, they block the tokio runtime thread,
//! preventing network operations from making progress. This can cause:
//! - Peer timeouts while waiting for disk I/O
//! - Reduced throughput as network and disk operations cannot overlap
//! - Poor utilization of available bandwidth
//!
//! # Approach
//!
//! We use two strategies depending on the operation:
//!
//! ## `tokio::fs` (for simple operations)
//! Used for: `create_dir_all`, `read`, `remove_dir_all`, `write`
//!
//! Note: `tokio::fs` internally uses `spawn_blocking` for most operations.
//! The benefit is that callers get a clean async API without managing the
//! blocking task spawning themselves, and tokio handles the thread pool.
//!
//! ## Explicit `spawn_blocking` (for iterator-based operations)
//! Used for: `read_dir`
//!
//! `std::fs::read_dir` returns a `ReadDir` iterator that yields `DirEntry` items.
//! While `tokio::fs::read_dir` exists, it returns an async stream that requires
//! careful handling of ownership and lifetimes. Using explicit `spawn_blocking`
//! with the sync version is simpler for our use case (reading all entries into
//! a Vec at once).
//!
//! # Thread Pool Implications
//!
//! Operations using `spawn_blocking` run on tokio's blocking thread pool (default: 512 threads).
//! For snap sync workloads (typically <100 concurrent directory reads), this is more than
//! sufficient. If you need to limit concurrency, consider using a semaphore.
//!
//! # Error Handling
//!
//! All functions return `SnapError::FileSystem` with:
//! - `operation`: What we were trying to do (for debugging)
//! - `path`: Which file/directory failed
//! - `kind`: The underlying `std::io::ErrorKind`

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::error::SnapError;

/// Ensures a directory exists, creating it if necessary.
///
/// This is an async version of the pattern:
/// ```ignore
/// if !std::fs::exists(path)? {
///     std::fs::create_dir_all(path)?;
/// }
/// ```
pub async fn ensure_dir_exists(path: &Path) -> Result<(), SnapError> {
    match tokio::fs::try_exists(path).await {
        Ok(true) => Ok(()),
        Ok(false) => tokio::fs::create_dir_all(path)
            .await
            .map_err(|e| SnapError::FileSystem {
                operation: "create directory",
                path: path.to_path_buf(),
                kind: e.kind(),
            }),
        Err(e) => Err(SnapError::FileSystem {
            operation: "check exists",
            path: path.to_path_buf(),
            kind: e.kind(),
        }),
    }
}

/// Reads all file paths from a directory.
///
/// # Why `spawn_blocking`?
///
/// We use `spawn_blocking` instead of `tokio::fs::read_dir` because:
/// 1. `tokio::fs::read_dir` returns an async `ReadDir` stream that requires
///    `.next().await` for each entry, adding complexity
/// 2. We always need all paths upfront (to pass to rocksdb's `ingest_external_file`
///    or to iterate over for reading), so streaming provides no benefit
/// 3. The sync version in `spawn_blocking` is simpler and equally efficient
///
/// # Thread Pool Usage
///
/// This function runs on tokio's blocking thread pool. The actual directory
/// read is typically fast (metadata only, no file contents), so thread pool
/// saturation is unlikely even with many concurrent calls.
///
/// # Ordering
///
/// Returns paths sorted alphabetically for deterministic processing order.
/// This is important for reproducible behavior across runs.
///
/// # Errors
///
/// - Returns `SnapError::FileSystem` if the directory cannot be read
/// - Returns `SnapError::FileSystem` if any directory entry cannot be read
///   (e.g., permission denied, corrupted filesystem)
pub async fn read_dir_paths(dir: &Path) -> Result<Vec<PathBuf>, SnapError> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(|e| SnapError::FileSystem {
                operation: "read directory",
                path: dir.clone(),
                kind: e.kind(),
            })?
            .map(|entry| {
                entry.map(|e| e.path()).map_err(|e| SnapError::FileSystem {
                    operation: "read directory entry",
                    path: dir.clone(),
                    kind: e.kind(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        paths.sort();
        Ok(paths)
    })
    .await?
}

/// Reads the contents of a file asynchronously.
pub async fn read_file(path: &Path) -> Result<Vec<u8>, SnapError> {
    tokio::fs::read(path)
        .await
        .map_err(|e| SnapError::FileSystem {
            operation: "read file",
            path: path.to_path_buf(),
            kind: e.kind(),
        })
}

/// Removes a directory and all its contents asynchronously.
pub async fn remove_dir_all(path: &Path) -> Result<(), SnapError> {
    tokio::fs::remove_dir_all(path)
        .await
        .map_err(|e| SnapError::FileSystem {
            operation: "remove directory",
            path: path.to_path_buf(),
            kind: e.kind(),
        })
}

/// Writes data to a file asynchronously.
///
/// Creates parent directories if they don't exist.
pub async fn write_file(path: &Path, contents: &[u8]) -> Result<(), SnapError> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        ensure_dir_exists(parent).await?;
    }

    tokio::fs::write(path, contents)
        .await
        .map_err(|e| SnapError::FileSystem {
            operation: "write file",
            path: path.to_path_buf(),
            kind: e.kind(),
        })
}

/// Removes a directory if it exists, ignoring NotFound errors.
pub async fn remove_dir_all_if_exists(path: &Path) -> Result<(), SnapError> {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(SnapError::FileSystem {
            operation: "remove directory",
            path: path.to_path_buf(),
            kind: e.kind(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_ensure_dir_exists_creates_new() {
        let temp = tempdir().unwrap();
        let new_dir = temp.path().join("new_dir");

        assert!(!new_dir.exists());
        ensure_dir_exists(&new_dir).await.unwrap();
        assert!(new_dir.exists());
    }

    #[tokio::test]
    async fn test_ensure_dir_exists_idempotent() {
        let temp = tempdir().unwrap();
        let existing_dir = temp.path().join("existing");
        std::fs::create_dir(&existing_dir).unwrap();

        // Should not fail if directory already exists
        ensure_dir_exists(&existing_dir).await.unwrap();
        assert!(existing_dir.exists());
    }

    #[tokio::test]
    async fn test_read_dir_paths() {
        let temp = tempdir().unwrap();

        // Create some files
        std::fs::write(temp.path().join("b.txt"), b"b").unwrap();
        std::fs::write(temp.path().join("a.txt"), b"a").unwrap();
        std::fs::write(temp.path().join("c.txt"), b"c").unwrap();

        let paths = read_dir_paths(temp.path()).await.unwrap();

        assert_eq!(paths.len(), 3);
        // Should be sorted
        assert!(paths[0].ends_with("a.txt"));
        assert!(paths[1].ends_with("b.txt"));
        assert!(paths[2].ends_with("c.txt"));
    }

    #[tokio::test]
    async fn test_read_write_file() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("test.bin");

        let data = b"hello world";
        write_file(&file_path, data).await.unwrap();

        let read_data = read_file(&file_path).await.unwrap();
        assert_eq!(read_data, data);
    }

    #[tokio::test]
    async fn test_remove_dir_all() {
        let temp = tempdir().unwrap();
        let dir = temp.path().join("to_remove");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("file.txt"), b"data").unwrap();

        assert!(dir.exists());
        remove_dir_all(&dir).await.unwrap();
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn test_remove_dir_all_if_exists() {
        let temp = tempdir().unwrap();
        let non_existent = temp.path().join("does_not_exist");

        // Should not fail for non-existent directory
        remove_dir_all_if_exists(&non_existent).await.unwrap();
    }
}
