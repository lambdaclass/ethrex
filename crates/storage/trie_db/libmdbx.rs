use ethrex_trie::{Nibbles, error::TrieError};
use libmdbx::orm::{Database, Table};
use std::{marker::PhantomData, sync::Arc};
/// Libmdbx implementation for the TrieDB trait, with get and put operations.
pub struct LibmdbxTrieDB<T: Table> {
    db: Arc<Database>,
    phantom: PhantomData<T>,
}

use ethrex_trie::{TrieDB, db::nibbles_to_fixed_size};

impl<T> LibmdbxTrieDB<T>
where
    T: Table<Key = [u8; 33], Value = Vec<u8>>,
{
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            phantom: PhantomData,
        }
    }
}

impl<T> TrieDB for LibmdbxTrieDB<T>
where
    T: Table<Key = [u8; 33], Value = Vec<u8>>,
{
    fn get(&self, key: Nibbles) -> Result<Option<Vec<u8>>, TrieError> {
        let txn = self.db.begin_read().map_err(TrieError::DbError)?;
        txn.get::<T>(nibbles_to_fixed_size(key))
            .map_err(TrieError::DbError)
    }

    fn put_batch(&self, key_values: Vec<(Nibbles, Vec<u8>)>) -> Result<(), TrieError> {
        let txn = self.db.begin_readwrite().map_err(TrieError::DbError)?;
        for (key, value) in key_values {
            txn.upsert::<T>(nibbles_to_fixed_size(key), value)
                .map_err(TrieError::DbError)?;
        }
        txn.commit().map_err(TrieError::DbError)
    }
}

#[cfg(test)]
mod test {
    use super::LibmdbxTrieDB;
    use crate::trie_db::test_utils::libmdbx::{TestNodes, new_db};
    use ethrex_trie::Nibbles;
    use ethrex_trie::Trie;
    use ethrex_trie::TrieDB;

    use libmdbx::{
        orm::{Database, table},
        table_info,
    };
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn simple_addition() {
        table!(
            /// [u8;33] to Node table
            ( Nodes )  [u8;33] => Vec<u8>
        );
        let inner_db = new_db::<Nodes>();
        let key = Nibbles::from_hex(vec![1, 2, 3, 4]);
        let db = LibmdbxTrieDB::<Nodes>::new(inner_db);
        assert_eq!(db.get(key.clone()).unwrap(), None);
        db.put(key.clone(), "value".into()).unwrap();
        assert_eq!(db.get(key).unwrap(), Some("value".into()));
    }

    #[test]
    fn different_tables() {
        table!(
            /// vec to vec
            ( TableA ) [u8;33] => Vec<u8>
        );
        table!(
            /// vec to vec
            ( TableB ) [u8;33] => Vec<u8>
        );
        let tables = [table_info!(TableA), table_info!(TableB)]
            .into_iter()
            .collect();

        let inner_db = Arc::new(Database::create(None, &tables).unwrap());
        let db_a = LibmdbxTrieDB::<TableA>::new(inner_db.clone());
        let db_b = LibmdbxTrieDB::<TableB>::new(inner_db.clone());
        let key = Nibbles::from_hex(vec![1, 2, 3, 4]);
        db_a.put(key.clone(), "value".into()).unwrap();
        assert_eq!(db_b.get(key).unwrap(), None);
    }

    #[test]
    fn get_old_state() {
        let db = new_db::<TestNodes>();
        let mut trie = Trie::new(Box::new(LibmdbxTrieDB::<TestNodes>::new(db.clone())));

        trie.insert([0; 32].to_vec(), [0; 32].to_vec()).unwrap();
        trie.insert([1; 32].to_vec(), [1; 32].to_vec()).unwrap();

        let root = trie.hash().unwrap();

        trie.insert([0; 32].to_vec(), [2; 32].to_vec()).unwrap();
        trie.insert([1; 32].to_vec(), [3; 32].to_vec()).unwrap();

        assert_eq!(trie.get(&[0; 32].to_vec()).unwrap(), Some([2; 32].to_vec()));
        assert_eq!(trie.get(&[1; 32].to_vec()).unwrap(), Some([3; 32].to_vec()));

        let trie = Trie::open(Box::new(LibmdbxTrieDB::<TestNodes>::new(db.clone())), root);

        assert_eq!(trie.get(&[0; 32].to_vec()).unwrap(), Some([0; 32].to_vec()));
        assert_eq!(trie.get(&[1; 32].to_vec()).unwrap(), Some([1; 32].to_vec()));
    }

    #[test]
    fn get_old_state_with_removals() {
        let db = new_db::<TestNodes>();
        let mut trie = Trie::new(Box::new(LibmdbxTrieDB::<TestNodes>::new(db.clone())));

        trie.insert([0; 32].to_vec(), [0; 32].to_vec()).unwrap();
        trie.insert([1; 32].to_vec(), [1; 32].to_vec()).unwrap();
        trie.insert([2; 32].to_vec(), [2; 32].to_vec()).unwrap();

        let root = trie.hash().unwrap();

        trie.insert([0; 32].to_vec(), vec![0x04]).unwrap();
        trie.remove(&[1; 32].to_vec()).unwrap();
        trie.insert([2; 32].to_vec(), vec![0x05]).unwrap();
        trie.remove(&[0; 32].to_vec()).unwrap();

        assert_eq!(trie.get(&[0; 32].to_vec()).unwrap(), None);
        assert_eq!(trie.get(&[1; 32].to_vec()).unwrap(), None);
        assert_eq!(trie.get(&[2; 32].to_vec()).unwrap(), Some(vec![0x05]));

        let trie = Trie::open(Box::new(LibmdbxTrieDB::<TestNodes>::new(db.clone())), root);

        assert_eq!(trie.get(&[0; 32].to_vec()).unwrap(), Some([0; 32].to_vec()));
        assert_eq!(trie.get(&[1; 32].to_vec()).unwrap(), Some([1; 32].to_vec()));
        assert_eq!(trie.get(&[2; 32].to_vec()).unwrap(), Some([2; 32].to_vec()));
    }

    #[test]
    fn revert() {
        let db = new_db::<TestNodes>();
        let mut trie = Trie::new(Box::new(LibmdbxTrieDB::<TestNodes>::new(db.clone())));

        trie.insert([0; 32].to_vec(), [0; 32].to_vec()).unwrap();
        trie.insert([1; 32].to_vec(), [1; 32].to_vec()).unwrap();

        let root = trie.hash().unwrap();

        trie.insert([0; 32].to_vec(), [2; 32].to_vec()).unwrap();
        trie.insert([1; 32].to_vec(), [3; 32].to_vec()).unwrap();

        let mut trie = Trie::open(Box::new(LibmdbxTrieDB::<TestNodes>::new(db.clone())), root);

        trie.insert([2; 32].to_vec(), [4; 32].to_vec()).unwrap();

        assert_eq!(trie.get(&[0; 32].to_vec()).unwrap(), Some([0; 32].to_vec()));
        assert_eq!(trie.get(&[1; 32].to_vec()).unwrap(), Some([1; 32].to_vec()));
        assert_eq!(trie.get(&[2; 32].to_vec()).unwrap(), Some([4; 32].to_vec()));
    }

    #[test]
    fn revert_with_removals() {
        let db = new_db::<TestNodes>();
        let mut trie = Trie::new(Box::new(LibmdbxTrieDB::<TestNodes>::new(db.clone())));

        trie.insert([0; 32].to_vec(), [0; 32].to_vec()).unwrap();
        trie.insert([1; 32].to_vec(), [1; 32].to_vec()).unwrap();
        trie.insert([2; 32].to_vec(), [2; 32].to_vec()).unwrap();

        let root = trie.hash().unwrap();

        trie.insert([0; 32].to_vec(), [4; 32].to_vec()).unwrap();
        trie.remove(&[1; 32].to_vec()).unwrap();
        trie.insert([2; 32].to_vec(), [5; 32].to_vec()).unwrap();
        trie.remove(&[0; 32].to_vec()).unwrap();

        let mut trie = Trie::open(Box::new(LibmdbxTrieDB::<TestNodes>::new(db.clone())), root);

        trie.remove(&[2; 32].to_vec()).unwrap();

        assert_eq!(trie.get(&[0; 32].to_vec()).unwrap(), Some([0; 32].to_vec()));
        assert_eq!(trie.get(&[1; 32].to_vec()).unwrap(), Some([1; 32].to_vec()));
        assert_eq!(trie.get(&vec![0x02]).unwrap(), None);
    }

    #[test]
    fn resume_trie() {
        use crate::trie_db::test_utils::libmdbx::{new_db_with_path, open_db};

        let trie_dir = TempDir::new().expect("Failed to create temp dir");
        let trie_dir = trie_dir.path();

        // Create new trie from clean DB
        let db = new_db_with_path::<TestNodes>(trie_dir.into());
        let mut trie = Trie::new(Box::new(LibmdbxTrieDB::<TestNodes>::new(db.clone())));

        trie.insert([0; 32].to_vec(), [1; 32].to_vec()).unwrap();
        trie.insert([1; 32].to_vec(), [2; 32].to_vec()).unwrap();
        trie.insert([2; 32].to_vec(), [4; 32].to_vec()).unwrap();

        // Save current root
        let root = trie.hash().unwrap();

        // Release DB
        drop(db);
        drop(trie);

        let db2 = open_db::<TestNodes>(trie_dir.to_str().unwrap());
        // Create a new trie based on the previous trie's DB
        let trie = Trie::open(Box::new(LibmdbxTrieDB::<TestNodes>::new(db2)), root);

        assert_eq!(trie.get(&[0; 32].to_vec()).unwrap(), Some([1; 32].to_vec()));
        assert_eq!(trie.get(&[1; 32].to_vec()).unwrap(), Some([2; 32].to_vec()));
        assert_eq!(trie.get(&[2; 32].to_vec()).unwrap(), Some([4; 32].to_vec()));
    }
}
