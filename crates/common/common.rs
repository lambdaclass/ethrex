#![cfg_attr(not(feature = "std"), no_std)]
#[macro_use]
extern crate alloc;

pub use ethereum_types::*;
pub mod constants;
pub mod serde_utils;
pub mod types;
pub mod validation;
pub use bytes::Bytes;
pub mod base64;
#[cfg(feature = "std")]
pub use ethrex_trie::{TrieLogger, TrieWitness};
pub mod errors;
pub mod evm;
#[cfg(feature = "std")]
pub mod fd_limit;
#[cfg(feature = "std")]
pub mod genesis_utils;
pub mod rkyv_utils;
pub mod tracing;
pub mod utils;

#[cfg(feature = "std")]
pub type OnceCell<T> = once_cell::sync::OnceCell<T>;
#[cfg(not(feature = "std"))]
pub type OnceCell<T> = core::cell::OnceCell<T>;

pub use errors::InvalidBlockError;
pub use ethrex_crypto::{CryptoError, NativeCrypto};
#[cfg(feature = "std")]
pub use validation::validate_receipts_root;
pub use validation::{
    get_total_blob_gas, validate_block_access_list_hash, validate_block_access_list_size,
    validate_block_pre_execution, validate_gas_used, validate_header_bal_indices,
    validate_requests_hash,
};
