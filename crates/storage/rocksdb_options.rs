//! RocksDB option overrides loaded from a file at startup.
//!
//! ethrex opens RocksDB with a set of tuned defaults. A [`RocksDbOptions`] holds
//! values that replace some of those defaults, so a different tuning can be tried
//! by editing a file instead of rebuilding the client. Every field is optional; a
//! field that is not set keeps the built-in default.
//!
//! The file is JSON:
//!
//! ```json
//! {
//!   "db": {
//!     "max_background_jobs": 12
//!   },
//!   "column_families": {
//!     "account_trie_nodes": {
//!       "write_buffer_size": 268435456,
//!       "block_size": 4096
//!     }
//!   }
//! }
//! ```

use crate::api::tables::TABLES;
use crate::error::StoreError;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Overrides for the options ethrex opens RocksDB with.
///
/// The default value overrides nothing.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RocksDbOptions {
    /// Options that apply to the whole database.
    pub db: RocksDbDbOptions,
    /// Options of individual column families, keyed by table name.
    pub column_families: BTreeMap<String, RocksDbCfOptions>,
}

/// Database-wide RocksDB options. Each field is named after the RocksDB option it sets.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RocksDbDbOptions {
    /// Maximum number of concurrent background flushes and compactions.
    pub max_background_jobs: Option<i32>,
    /// Maximum number of files kept open; `-1` keeps every file open.
    pub max_open_files: Option<i32>,
    /// Total memtable size, in bytes, across all column families before a flush is forced.
    pub db_write_buffer_size: Option<usize>,
    /// Total WAL size, in bytes, before the column families holding it back are flushed.
    pub max_total_wal_size: Option<u64>,
    /// Bytes written to an SST file between syncs to disk.
    pub bytes_per_sync: Option<u64>,
    /// Bytes written to the WAL between syncs to disk.
    pub wal_bytes_per_sync: Option<u64>,
    /// Readahead, in bytes, used by compactions.
    pub compaction_readahead_size: Option<usize>,
}

/// RocksDB options of one column family. Each field is named after the RocksDB
/// option it sets, except [`bloom_filter_bits_per_key`](Self::bloom_filter_bits_per_key).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RocksDbCfOptions {
    /// Size of a memtable, in bytes.
    pub write_buffer_size: Option<usize>,
    /// Maximum number of memtables, counting the ones being flushed.
    pub max_write_buffer_number: Option<i32>,
    /// Number of memtables merged together before they are flushed.
    pub min_write_buffer_number_to_merge: Option<i32>,
    /// Target size of an SST file at level 1, in bytes.
    pub target_file_size_base: Option<u64>,
    /// Maximum total size of level 1, in bytes.
    pub max_bytes_for_level_base: Option<u64>,
    /// Number of level 0 files that triggers a compaction.
    pub level_zero_file_num_compaction_trigger: Option<i32>,
    /// Number of level 0 files at which writes are slowed down.
    pub level_zero_slowdown_writes_trigger: Option<i32>,
    /// Number of level 0 files at which writes are stopped.
    pub level_zero_stop_writes_trigger: Option<i32>,
    /// Compression of the SST blocks.
    pub compression: Option<RocksDbCompression>,
    /// Size of an SST data block, in bytes.
    pub block_size: Option<usize>,
    /// Bits per key of the SST bloom filter. Setting it gives the column family a
    /// bloom filter if it had none; a filter cannot be removed this way.
    pub bloom_filter_bits_per_key: Option<f64>,
}

/// Compression algorithms ethrex's RocksDB is built with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RocksDbCompression {
    None,
    Lz4,
}

impl RocksDbOptions {
    /// Reads and validates the overrides in the JSON file at `path`.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path).map_err(|err| {
            StoreError::Custom(format!(
                "Failed to read RocksDB options file {}: {err}",
                path.display()
            ))
        })?;
        Self::from_json(&contents).map_err(|err| {
            StoreError::Custom(format!(
                "Invalid RocksDB options file {}: {err}",
                path.display()
            ))
        })
    }

    /// Parses and validates overrides from a JSON string.
    ///
    /// Unknown options and unknown column families are errors rather than being
    /// ignored, so a typo cannot silently leave a default in place.
    pub fn from_json(json: &str) -> Result<Self, StoreError> {
        let options: Self =
            serde_json::from_str(json).map_err(|err| StoreError::Custom(err.to_string()))?;
        options.validate()?;
        Ok(options)
    }

    /// Whether no option is overridden.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn validate(&self) -> Result<(), StoreError> {
        for (name, cf_options) in &self.column_families {
            if !TABLES.contains(&name.as_str()) {
                return Err(StoreError::Custom(format!(
                    "unknown column family `{name}`, expected one of: {}",
                    TABLES.join(", ")
                )));
            }
            if let Some(bits_per_key) = cf_options.bloom_filter_bits_per_key
                && !(bits_per_key.is_finite() && bits_per_key > 0.0)
            {
                return Err(StoreError::Custom(format!(
                    "`bloom_filter_bits_per_key` of column family `{name}` must be a positive number, got {bits_per_key}"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tables::ACCOUNT_TRIE_NODES;

    #[test]
    fn empty_file_overrides_nothing() {
        let options = RocksDbOptions::from_json("{}").unwrap();
        assert!(options.is_empty());
        assert_eq!(options, RocksDbOptions::default());
    }

    #[test]
    fn parses_db_and_column_family_options() {
        let options = RocksDbOptions::from_json(
            r#"{
                "db": { "max_background_jobs": 12, "max_total_wal_size": 1073741824 },
                "column_families": {
                    "account_trie_nodes": {
                        "write_buffer_size": 268435456,
                        "compression": "lz4",
                        "block_size": 4096,
                        "bloom_filter_bits_per_key": 12.5
                    }
                }
            }"#,
        )
        .unwrap();

        assert!(!options.is_empty());
        assert_eq!(options.db.max_background_jobs, Some(12));
        assert_eq!(options.db.max_total_wal_size, Some(1 << 30));
        assert_eq!(options.db.max_open_files, None);

        let cf_options = &options.column_families[ACCOUNT_TRIE_NODES];
        assert_eq!(cf_options.write_buffer_size, Some(256 << 20));
        assert_eq!(cf_options.compression, Some(RocksDbCompression::Lz4));
        assert_eq!(cf_options.block_size, Some(4096));
        assert_eq!(cf_options.bloom_filter_bits_per_key, Some(12.5));
        assert_eq!(cf_options.max_write_buffer_number, None);
    }

    #[test]
    fn rejects_unknown_options() {
        for json in [
            r#"{ "database": {} }"#,
            r#"{ "db": { "max_background_job": 12 } }"#,
            r#"{ "column_families": { "account_trie_nodes": { "blocksize": 4096 } } }"#,
            r#"{ "column_families": { "account_trie_nodes": { "compression": "zstd" } } }"#,
        ] {
            assert!(RocksDbOptions::from_json(json).is_err(), "accepted {json}");
        }
    }

    #[test]
    fn rejects_unknown_column_families() {
        let err =
            RocksDbOptions::from_json(r#"{ "column_families": { "acount_trie_nodes": {} } }"#)
                .unwrap_err();
        assert!(err.to_string().contains("acount_trie_nodes"), "{err}");
    }

    #[test]
    fn rejects_non_positive_bloom_filter_bits() {
        for bits_per_key in ["0", "-1.5"] {
            let json = format!(
                r#"{{ "column_families": {{ "account_trie_nodes": {{ "bloom_filter_bits_per_key": {bits_per_key} }} }} }}"#
            );
            assert!(RocksDbOptions::from_json(&json).is_err(), "accepted {json}");
        }
    }

    #[test]
    fn reports_the_path_of_an_unreadable_or_invalid_file() {
        let dir = tempfile::tempdir().unwrap();

        let missing = dir.path().join("missing.json");
        let err = RocksDbOptions::from_file(&missing).unwrap_err();
        assert!(err.to_string().contains("missing.json"), "{err}");

        let invalid = dir.path().join("invalid.json");
        std::fs::write(&invalid, "{ not json").unwrap();
        let err = RocksDbOptions::from_file(&invalid).unwrap_err();
        assert!(err.to_string().contains("invalid.json"), "{err}");
    }
}
