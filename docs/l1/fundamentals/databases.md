# Databases

Ethrex uses a versioning system to ensure we don't run on invalid data if we restart the node after a breaking change to the DB structure. This system consists of a `STORE_SCHEMA_VERSION` constant, defined in `crates/storage/lib.rs` that must be increased after any breaking change and that is checked every time we start the node.

## Schema version checks at startup

The version is stored in `metadata.json` inside the datadir and compared with the binary's `STORE_SCHEMA_VERSION` before the database is opened:

- **Same version**: the node starts normally.
- **Database older than the binary**: the pending migrations run automatically, one version at a time, and `metadata.json` is updated after each step. Migrations are forward-only.
- **Database newer than the binary**: the node refuses to start with an `Incompatible DB Version` error and leaves the database untouched. There is no downgrade path, but nothing needs to be erased: start an ethrex build that supports the database's schema version (the one that wrote it, or newer) and the node resumes where it left off. `ethrex removedb` is only needed if you intend to abandon the database and resync from scratch.
- **No version file next to an existing database**: the database predates versioning and cannot be migrated; a resync (`ethrex removedb`) is required.

The newer-database case is easy to reach when the same datadir is shared by binaries built from different branches (for example, an image built from `main` and one built from a devnet branch): the first newer binary that opens the datadir stamps its own version, and the older one then refuses to open it.
