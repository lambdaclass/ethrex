use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use ethrex_common::{types::AccountState, H256};
use ethrex_trie::TrieDB;

use crate::{cache::Cache, Store};
