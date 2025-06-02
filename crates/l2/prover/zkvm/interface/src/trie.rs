use std::collections::HashMap;

use ethrex_common::{
    types::{AccountInfo, AccountState, AccountUpdate},
    H160, U256,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode, error::RLPDecodeError};
use ethrex_storage::{hash_address, hash_key};
use ethrex_trie::{Trie, TrieError};
use ethrex_vm::ProverDB;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    TrieError(#[from] TrieError),
    #[error(transparent)]
    RLPDecode(#[from] RLPDecodeError),
    #[error("Missing storage trie for address {0}")]
    MissingStorageTrie(H160),
    #[error("Missing storage for address {0}")]
    StorageNotFound(H160),
}
