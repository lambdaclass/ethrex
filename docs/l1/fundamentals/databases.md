# Databases

Ethrex uses a versioning system to ensure we don't run on invalid data if we restart the node after a breaking change to the DB structure. This system consists of a `STORE_SCHEMA_VERSION` constant, defined in `crates/storage/lib.rs` that must be increased after any breaking change and that is checked every time we start the node.

## Schema version checks at startup

The version is stored in `metadata.json` inside the datadir and compared with the binary's `STORE_SCHEMA_VERSION` before the database is opened:

- **Same version**: the node starts normally.
- **Database older than the binary**: the pending migrations run automatically, one version at a time, and `metadata.json` is updated after each step. Migrations are forward-only.
- **Database newer than the binary**: the node refuses to start with an `Incompatible DB Version` error and leaves the database untouched. There is no downgrade path, but nothing needs to be erased: start an ethrex build that supports the database's schema version (the one that wrote it, or newer) and the node resumes where it left off. `ethrex removedb` is only needed if you intend to abandon the database and resync from scratch.
- **No version file next to an existing database**: the database predates versioning and cannot be migrated; a resync (`ethrex removedb`) is required.

The newer-database case is easy to reach when the same datadir is shared by binaries built from different branches (for example, an image built from `main` and one built from a devnet branch): the first newer binary that opens the datadir stamps its own version, and the older one then refuses to open it.

## RocksDB options file

Ethrex opens RocksDB with built-in options tuned per table. To try a different tuning without rebuilding, pass a JSON file with `--rocksdb.options-file <PATH>` (or `ETHREX_ROCKSDB_OPTIONS_FILE`). The options in the file replace the built-in ones; everything left out keeps its default.

```json
{
  "db": {
    "max_background_jobs": 12
  },
  "column_families": {
    "account_trie_nodes": {
      "write_buffer_size": 268435456,
      "block_size": 4096
    },
    "storage_trie_nodes": {
      "compression": "lz4"
    }
  }
}
```

`db` holds the database-wide options:

- `max_background_jobs`
- `max_open_files`
- `db_write_buffer_size`
- `max_total_wal_size`
- `bytes_per_sync`
- `wal_bytes_per_sync`
- `compaction_readahead_size`

`column_families` holds the options of individual tables, keyed by table name (the names in `crates/storage/api/tables.rs`, such as `headers`, `account_trie_nodes` or `storage_flatkeyvalue`):

- `write_buffer_size`
- `max_write_buffer_number`
- `min_write_buffer_number_to_merge`
- `target_file_size_base`
- `max_bytes_for_level_base`
- `level_zero_file_num_compaction_trigger`
- `level_zero_slowdown_writes_trigger`
- `level_zero_stop_writes_trigger`
- `compression`: `"none"` or `"lz4"`
- `block_size`
- `bloom_filter_bits_per_key`: also gives a bloom filter to a table that has none by default; a filter cannot be removed this way

Each option is named after the RocksDB option it sets and takes the same value, with sizes in bytes. A file that names an unknown option or table is rejected at startup, so a typo cannot silently leave a default in place.

The file is read every time the node starts. Options that only affect new data, such as `block_size` and `compression`, apply to the SST files written from then on; existing files keep their layout until they are compacted. To confirm what the database was opened with, look at the newest `OPTIONS-*` file in the datadir.
