
#[cfg(feature = "libmdbx")]
pub mod libmdbx {
    use std::{path::PathBuf, sync::Arc};

    use libmdbx::{
        orm::{table_info, Database, Table},
        table,
    };

    table!(
        /// Test table.
        (TestNodes) Vec<u8> => Vec<u8>
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
}
