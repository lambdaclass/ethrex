//! Snap sync profile manifest types and loading.
//!
//! These types are used by the capture path (in `snap_sync.rs`) and by the
//! external replay tooling crate (`snapsync_profile`).

use std::path::Path;

use ethrex_common::H256;
use serde::{Deserialize, Serialize};

use super::SyncError;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct SnapProfileManifest {
    pub version: u32,
    pub chain_id: u64,
    pub rocksdb_enabled: bool,
    pub pivot: PivotInfo,
    pub post_accounts_insert_state_root: H256,
    pub paths: DatasetPaths,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PivotInfo {
    pub number: u64,
    pub hash: H256,
    pub state_root: H256,
    pub timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DatasetPaths {
    pub account_state_snapshots_dir: String,
    pub account_storages_snapshots_dir: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load and validate a snap profile manifest from `{dataset_root}/manifest.json`.
pub fn load_manifest(dataset_root: &Path) -> Result<SnapProfileManifest, SyncError> {
    let manifest_path = dataset_root.join("manifest.json");
    let contents = std::fs::read_to_string(&manifest_path)
        .map_err(|e| SyncError::ProfileError(format!("Failed to read manifest: {e}")))?;

    let manifest: SnapProfileManifest = serde_json::from_str(&contents)
        .map_err(|e| SyncError::ProfileError(format!("Failed to parse manifest: {e}")))?;

    // Version check
    if manifest.version != 1 {
        return Err(SyncError::ProfileError(format!(
            "Unsupported manifest version: {} (expected 1)",
            manifest.version
        )));
    }

    // Validate that snapshot directories exist and are non-empty
    let acc_dir = dataset_root.join(&manifest.paths.account_state_snapshots_dir);
    validate_non_empty_dir(&acc_dir, "account_state_snapshots")?;

    let storage_dir = dataset_root.join(&manifest.paths.account_storages_snapshots_dir);
    validate_non_empty_dir(&storage_dir, "account_storages_snapshots")?;

    Ok(manifest)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

pub fn validate_non_empty_dir(path: &Path, label: &str) -> Result<(), SyncError> {
    let entries = std::fs::read_dir(path).map_err(|e| {
        SyncError::ProfileError(format!("{label} directory {path:?} cannot be read: {e}"))
    })?;
    if entries.peekable().peek().is_none() {
        return Err(SyncError::ProfileError(format!(
            "{label} directory {path:?} is empty"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_valid_manifest(dir: &Path, rocksdb_enabled: bool, version: u32) {
        let manifest = serde_json::json!({
            "version": version,
            "chain_id": 1,
            "rocksdb_enabled": rocksdb_enabled,
            "pivot": {
                "number": 100,
                "hash": H256::zero(),
                "state_root": H256::zero(),
                "timestamp": 1700000000_u64
            },
            "post_accounts_insert_state_root": H256::zero(),
            "paths": {
                "account_state_snapshots_dir": "account_state_snapshots",
                "account_storages_snapshots_dir": "account_storages_snapshots"
            }
        });
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn create_snapshot_dirs(dir: &Path) {
        let acc_dir = dir.join("account_state_snapshots");
        let storage_dir = dir.join("account_storages_snapshots");
        std::fs::create_dir_all(&acc_dir).unwrap();
        std::fs::create_dir_all(&storage_dir).unwrap();
        // Write a dummy file so the dirs are non-empty
        std::fs::write(acc_dir.join("dummy.rlp.0"), b"placeholder").unwrap();
        std::fs::write(storage_dir.join("dummy.rlp.0"), b"placeholder").unwrap();
    }

    #[test]
    fn load_manifest_accepts_valid_v1() {
        let dir = tempfile::tempdir().unwrap();
        write_valid_manifest(dir.path(), false, 1);
        create_snapshot_dirs(dir.path());

        let manifest = load_manifest(dir.path()).unwrap();
        assert_eq!(manifest.version, 1);
        assert!(!manifest.rocksdb_enabled);
        assert_eq!(manifest.pivot.number, 100);
    }

    #[test]
    fn load_manifest_accepts_rocksdb_enabled() {
        let dir = tempfile::tempdir().unwrap();
        write_valid_manifest(dir.path(), true, 1);
        create_snapshot_dirs(dir.path());

        let manifest = load_manifest(dir.path()).unwrap();
        assert!(manifest.rocksdb_enabled);
    }

    #[test]
    fn load_manifest_rejects_unsupported_version() {
        let dir = tempfile::tempdir().unwrap();
        write_valid_manifest(dir.path(), false, 2);
        create_snapshot_dirs(dir.path());

        let err = load_manifest(dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("Unsupported manifest version"),
            "Expected version error, got: {err}"
        );
    }

    #[test]
    fn load_manifest_rejects_missing_snapshot_dirs() {
        let dir = tempfile::tempdir().unwrap();
        write_valid_manifest(dir.path(), false, 1);
        // Don't create snapshot dirs

        let err = load_manifest(dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("cannot be read"),
            "Expected missing dir error, got: {err}"
        );
    }

    #[test]
    fn load_manifest_rejects_empty_snapshot_dirs() {
        let dir = tempfile::tempdir().unwrap();
        write_valid_manifest(dir.path(), false, 1);
        // Create dirs but leave them empty
        std::fs::create_dir_all(dir.path().join("account_state_snapshots")).unwrap();
        std::fs::create_dir_all(dir.path().join("account_storages_snapshots")).unwrap();

        let err = load_manifest(dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("empty"),
            "Expected empty dir error, got: {err}"
        );
    }

    #[test]
    fn load_manifest_rejects_missing_manifest_file() {
        let dir = tempfile::tempdir().unwrap();
        // No manifest.json written

        let err = load_manifest(dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("Failed to read manifest"),
            "Expected missing file error, got: {err}"
        );
    }

    #[test]
    fn manifest_roundtrip_serde() {
        let manifest = SnapProfileManifest {
            version: 1,
            chain_id: 1,
            rocksdb_enabled: false,
            pivot: PivotInfo {
                number: 42,
                hash: H256::zero(),
                state_root: H256::repeat_byte(0xaa),
                timestamp: 1700000000,
            },
            post_accounts_insert_state_root: H256::repeat_byte(0xbb),
            paths: DatasetPaths {
                account_state_snapshots_dir: "account_state_snapshots".to_string(),
                account_storages_snapshots_dir: "account_storages_snapshots".to_string(),
            },
        };

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let deserialized: SnapProfileManifest = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.version, 1);
        assert_eq!(deserialized.pivot.number, 42);
        assert_eq!(
            deserialized.post_accounts_insert_state_root,
            H256::repeat_byte(0xbb)
        );
    }
}
