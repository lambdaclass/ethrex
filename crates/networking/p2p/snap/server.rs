use bytes::Bytes;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;

use crate::rlpx::snap::{
    AccountRange, AccountRangeUnit, ByteCodes, GetAccountRange, GetByteCodes, GetStorageRanges,
    GetTrieNodes, StorageRanges, StorageSlot, TrieNodes,
};
use ethrex_common::types::AccountStateSlimCodec;

use super::error::SnapError;
use super::proof_to_encodable;

// Request Processing

pub async fn process_account_range_request(
    request: GetAccountRange,
    store: Store,
) -> Result<AccountRange, SnapError> {
    tokio::task::spawn_blocking(move || {
        let mut accounts = vec![];
        let mut bytes_used = 0;
        for (hash, account) in store.iter_accounts_from(request.root_hash, request.starting_hash)? {
            debug_assert!(hash >= request.starting_hash);
            bytes_used += 32 + AccountStateSlimCodec(account).length() as u64;
            accounts.push(AccountRangeUnit { hash, account });
            if hash >= request.limit_hash || bytes_used >= request.response_bytes {
                break;
            }
        }
        let proof = proof_to_encodable(store.get_account_range_proof(
            request.root_hash,
            request.starting_hash,
            accounts.last().map(|acc| acc.hash),
        )?);
        Ok(AccountRange {
            id: request.id,
            accounts,
            proof,
        })
    })
    .await
    .map_err(|e| SnapError::TaskPanic(e.to_string()))?
}

pub async fn process_storage_ranges_request(
    request: GetStorageRanges,
    store: Store,
) -> Result<StorageRanges, SnapError> {
    tokio::task::spawn_blocking(move || {
        let mut slots = vec![];
        let mut proof = vec![];
        let mut bytes_used = 0;

        for hashed_address in request.account_hashes {
            let mut account_slots = vec![];
            let mut res_capped = false;

            if let Some(storage_iter) =
                store.iter_storage_from(request.root_hash, hashed_address, request.starting_hash)?
            {
                for (hash, data) in storage_iter {
                    debug_assert!(hash >= request.starting_hash);
                    bytes_used += 64_u64; // slot size
                    account_slots.push(StorageSlot { hash, data });
                    if hash >= request.limit_hash || bytes_used >= request.response_bytes {
                        if bytes_used >= request.response_bytes {
                            res_capped = true;
                        }
                        break;
                    }
                }
            }

            // Generate proofs only if the response doesn't contain the full storage range for the account
            // Aka if the starting hash is not zero or if the response was capped due to byte limit
            if !request.starting_hash.is_zero() || res_capped && !account_slots.is_empty() {
                proof.extend(proof_to_encodable(
                    store
                        .get_storage_range_proof(
                            request.root_hash,
                            hashed_address,
                            request.starting_hash,
                            account_slots.last().map(|acc| acc.hash),
                        )?
                        .unwrap_or_default(),
                ));
            }

            if !account_slots.is_empty() {
                slots.push(account_slots);
            }

            if bytes_used >= request.response_bytes {
                break;
            }
        }
        Ok(StorageRanges {
            id: request.id,
            slots,
            proof,
        })
    })
    .await
    .map_err(|e| SnapError::TaskPanic(e.to_string()))?
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
    request: GetTrieNodes,
    store: Store,
) -> Result<TrieNodes, SnapError> {
    tokio::task::spawn_blocking(move || {
        let mut nodes = vec![];
        let mut remaining_bytes = request.bytes;
        for paths in request.paths {
            if paths.is_empty() {
                return Err(SnapError::BadRequest(
                    "zero-item pathset requested".to_string(),
                ));
            }
            let trie_nodes = store.get_trie_nodes(
                request.root_hash,
                paths.into_iter().map(|bytes| bytes.to_vec()).collect(),
                remaining_bytes,
            )?;
            nodes.extend(trie_nodes.iter().map(|nodes| Bytes::copy_from_slice(nodes)));
            remaining_bytes = remaining_bytes
                .saturating_sub(trie_nodes.iter().fold(0, |acc, nodes| acc + nodes.len()) as u64);
            if remaining_bytes == 0 {
                break;
            }
        }

        Ok(TrieNodes {
            id: request.id,
            nodes,
        })
    })
    .await
    .map_err(|e| SnapError::TaskPanic(e.to_string()))?
}
