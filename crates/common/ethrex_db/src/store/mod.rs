//! Page-based persistent storage.
//!
//! This module implements memory-mapped file storage with Copy-on-Write
//! concurrency, inspired by LMDB and Paprika.

mod db_address;
mod metrics;
mod page;
mod page_header;
mod page_types;
mod paged_db;
mod trie_store;

pub use db_address::DbAddress;
pub use metrics::{DbMetrics, MetricsSnapshot};
pub use page::{Page, PAGE_SIZE};
pub use page_header::{PageHeader, PageType};
pub use page_types::{DataPage, RootPage, LeafPage, AbandonedPage};
pub use paged_db::{PagedDb, DbError, BatchContext, ReadOnlyBatch, CommitOptions, Snapshot};
pub use trie_store::{PersistentTrie, StateTrie, StorageTrie, AccountData, PagedStateTrie};
