pub mod in_memory;
#[cfg(feature = "libmdbx")]
pub mod libmdbx;
#[cfg(feature = "libmdbx")]
pub mod libmdbx_dupsort;

use crate::error::TrieError;

pub trait TrieDB {
    fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, TrieError>;
    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), TrieError>;
}
