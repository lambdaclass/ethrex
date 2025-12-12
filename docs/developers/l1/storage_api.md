# Storage Backend API

We use a thin, minimal interface for storage backends:

- Thin: Minimal set of operations that databases must provide
- Simple: Avoids type-system complexity and focuses on core functionality

Rather than implementing business logic in each database backend, this API provides low-level primitives that higher-level code can build upon.
This eliminates code duplication and makes adding new database backends trivial.

The API differentiates between three types of database access:

- Read views `StorageReadView`: read-only views of the database, with no atomicity guarantees between operations.
- Write batches `StorageWriteBatch`: write batch functionality, with atomicity guarantees at commit time.
- Locked views `StorageLockedView`: read-only views of a point in time (snapshots), right now it's only used during snap-sync.
