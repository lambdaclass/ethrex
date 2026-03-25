use ethrex_storage::Store;

use crate::rlpx::snap::{
    AccountRange, ByteCodes, GetAccountRange, GetByteCodes, GetStorageRanges, GetTrieNodes,
    StorageRanges, TrieNodes,
};

use super::error::SnapError;

// Request Processing

pub async fn process_account_range_request(
    _request: GetAccountRange,
    _store: Store,
) -> Result<AccountRange, SnapError> {
    Err(SnapError::InternalError(
        "snap sync not supported on binary trie branch".to_string(),
    ))
}

pub async fn process_storage_ranges_request(
    _request: GetStorageRanges,
    _store: Store,
) -> Result<StorageRanges, SnapError> {
    Err(SnapError::InternalError(
        "snap sync not supported on binary trie branch".to_string(),
    ))
}

pub async fn process_byte_codes_request(
    request: GetByteCodes,
    store: Store,
) -> Result<ByteCodes, SnapError> {
    tokio::task::spawn_blocking(move || {
        let mut codes = vec![];
        let mut bytes_used = 0;
        for code_hash in request.hashes {
            if let Some(code) = store.get_account_code(code_hash)?.map(|c| c.bytecode) {
                bytes_used += code.len() as u64;
                codes.push(code);
            }
            if bytes_used >= request.bytes {
                break;
            }
        }
        Ok(ByteCodes {
            id: request.id,
            codes,
        })
    })
    .await
    .map_err(|e| SnapError::TaskPanic(e.to_string()))?
}

pub async fn process_trie_nodes_request(
    _request: GetTrieNodes,
    _store: Store,
) -> Result<TrieNodes, SnapError> {
    Err(SnapError::InternalError(
        "snap sync not supported on binary trie branch".to_string(),
    ))
}
