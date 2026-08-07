//! Async file system utilities for snap sync
//!
//! Provides async wrappers around file system operations to avoid blocking the
//! tokio runtime during disk I/O. `tokio::fs` is used for simple operations
//! (internally delegates to `spawn_blocking`), while explicit `spawn_blocking`
//! is used for `read_dir` to avoid async stream complexity.

use std::path::{Path, PathBuf};

use super::error::SnapError;

/// Ensures a directory exists, creating it if necessary.
pub async fn ensure_dir_exists(path: &Path) -> Result<(), SnapError> {
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|e| SnapError::FileSystem {
            operation: "create directory",
            path: path.to_path_buf(),
            kind: e.kind(),
        })
}

/// Reads all file paths from a directory, sorted alphabetically.
///
/// Uses `spawn_blocking` with sync `read_dir` since we always need all paths
/// upfront (for `ingest_external_file` or batch iteration).
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

        ensure_dir_exists(&existing_dir).await.unwrap();
        assert!(existing_dir.exists());
    }

    #[tokio::test]
    async fn test_read_dir_paths() {
        let temp = tempdir().unwrap();

        std::fs::write(temp.path().join("b.txt"), b"b").unwrap();
        std::fs::write(temp.path().join("a.txt"), b"a").unwrap();
        std::fs::write(temp.path().join("c.txt"), b"c").unwrap();

        let paths = read_dir_paths(temp.path()).await.unwrap();

        assert_eq!(paths.len(), 3);
        assert!(paths[0].ends_with("a.txt"));
        assert!(paths[1].ends_with("b.txt"));
        assert!(paths[2].ends_with("c.txt"));
    }

    #[tokio::test]
    async fn test_read_file() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("test.bin");

        let data = b"hello world";
        std::fs::write(&file_path, data).unwrap();

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
}
