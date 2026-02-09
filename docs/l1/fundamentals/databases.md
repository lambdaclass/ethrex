# Databases

Ethrex uses a versioning system to ensure we don't run on invalid data if we restart the node after a breaking change to the DB structure. This system consists of a `STORE_SCHEMA_VERSION` constant, defined in `crates/storage/lib.rs` that must be increased after any breaking change and that is checked every time we start the node.
