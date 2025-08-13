#[cfg(feature = "libmdbx")]
pub mod libmdbx {
    use std::{path::PathBuf, sync::Arc};

    use ethrex_trie::NodeHash;
    use libmdbx::{
        orm::{Database, Table, table_info},
        table,
    };

    table!(
        /// Test table.
        (TestNodes) NodeHash => Vec<u8>
    );

    /// Creates a new DB on a given path
    pub fn new_db_with_path<T: Table>(path: PathBuf) -> Arc<Database> {
        let tables = [table_info!(T)].into_iter().collect();
        Arc::new(Database::create(Some(path), &tables).expect("Failed creating db with path"))
    }

    /// Creates a new temporary DB
    pub fn new_db<T: Table>() -> Arc<Database> {
        let tables = [table_info!(T)].into_iter().collect();
        Arc::new(Database::create(None, &tables).expect("Failed to create temp DB"))
    }

    /// Opens a DB from a given path
    pub fn open_db<T: Table>(path: &str) -> Arc<Database> {
        let tables = [table_info!(T)].into_iter().collect();
        Arc::new(Database::open(path, &tables).expect("Failed to open DB"))
    }

    #[track_caller]
    pub fn put_node<T: Table>(db: &Database, hash: T::Key, node: T::Value) {
        let tx = db.begin_readwrite().expect("Begin tx failed");
        tx.upsert::<T>(hash, node).expect("Write failed");
        tx.commit().expect("Commit failed");
    }
}
