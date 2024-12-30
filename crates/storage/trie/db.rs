pub mod in_memory;
#[cfg(feature = "libmdbx")]
pub mod libmdbx;
#[cfg(feature = "libmdbx")]
pub mod libmdbx_dupsort;
#[cfg(feature = "redb")]
pub mod redb;
#[cfg(feature = "redb")]
pub mod redb_multitable;
mod utils;

use crate::error::TrieError;

pub trait TrieDB {
    fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, TrieError>;
    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), TrieError>;
    // fn put_batch(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), TrieError>;
    fn put_batch(&self, key_values: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), TrieError>;
}
