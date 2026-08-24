use bytes::Bytes;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;

use crate::rlpx::snap::{
    AccountRange, AccountRangeUnit, ByteCodes, GetAccountRange, GetByteCodes, GetStorageRanges,
    GetTrieNodes, StorageRanges, StorageSlot, TrieNodes,
};
use ethrex_common::types::AccountStateSlimCodec;

use super::constants::MAX_RESPONSE_BYTES;
use super::error::SnapError;
use super::proof_to_encodable;

/// Maximum number of item lookups served per snap serving request that walks a peer-supplied
/// list (`GetByteCodes` hashes, `GetStorageRanges` account hashes, `GetTrieNodes` paths).
///
/// Each item is a DB/trie probe (a trie-node lookup traverses several nodes). The response-byte
/// clamp only advances on a *hit*, so an all-miss/empty request returns almost nothing yet
/// still probes every item — O(N) disk/CPU work the message-rate limiter (which counts
/// messages, not items) doesn't catch. Capping the item *count* bounds that walk directly.
///
/// Sits comfortably above our own client's request batches (`NODE_BATCH_SIZE` = 500,
/// `STORAGE_BATCH_SIZE` = 300), and snap responses may be partial, so an honest peer whose
/// request exceeds the cap simply re-requests the remainder. 1024 is in line with geth's
/// per-request lookup caps, taken here as a reference point.
pub const MAX_SERVE_LOOKUPS: usize = 1024;

// Request Processing

pub async fn process_account_range_request(
    request: GetAccountRange,
    store: Store,
) -> Result<AccountRange, SnapError> {
    tokio::task::spawn_blocking(move || {
        // `responseBytes` is a soft limit chosen by the requester; clamp it to our server
        // maximum so a peer cannot request an unbounded walk of the whole state trie
        // (a single such request buffers every account into memory before replying — OOM).
        let byte_budget = request.response_bytes.min(MAX_RESPONSE_BYTES);
        let mut accounts = vec![];
        let mut bytes_used = 0;
        for (hash, account) in store.iter_accounts_from(request.root_hash, request.starting_hash)? {
            debug_assert!(hash >= request.starting_hash);
            bytes_used += 32 + AccountStateSlimCodec(account).length() as u64;
            accounts.push(AccountRangeUnit { hash, account });
            if hash >= request.limit_hash || bytes_used >= byte_budget {
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
        // Same soft-limit clamp as the account-range handler: bound the attacker-supplied
        // budget so a peer cannot walk a contract's entire storage trie into memory.
        let byte_budget = request.response_bytes.min(MAX_RESPONSE_BYTES);
        let mut slots = vec![];
        let mut proof = vec![];
        let mut bytes_used = 0;

        // Cap the number of accounts probed per request (`MAX_SERVE_LOOKUPS`): each opens a
        // storage trie, and an all-miss `account_hashes` list returns nothing, so the byte
        // budget alone wouldn't bound the walk. Partial responses are protocol-legal.
        for hashed_address in request.account_hashes.into_iter().take(MAX_SERVE_LOOKUPS) {
            let mut account_slots = vec![];
            let mut res_capped = false;

            if let Some(storage_iter) =
                store.iter_storage_from(request.root_hash, hashed_address, request.starting_hash)?
            {
                for (hash, data) in storage_iter {
                    debug_assert!(hash >= request.starting_hash);
                    bytes_used += 64_u64; // slot size
                    account_slots.push(StorageSlot { hash, data });
                    if hash >= request.limit_hash || bytes_used >= byte_budget {
                        if bytes_used >= byte_budget {
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

            // Absent accounts / empty ranges return no slots and aren't included; the
            // lookup-count cap (not a byte charge) bounds an all-miss `account_hashes` walk.
            if !account_slots.is_empty() {
                slots.push(account_slots);
            }

            if bytes_used >= byte_budget {
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
        // Clamp the peer-supplied budget: `hashes` can be a large (or snappy-inflated,
        // repeated) list, so an unbounded `bytes` would let one request buffer many large
        // bytecodes into memory.
        let byte_budget = request.bytes.min(MAX_RESPONSE_BYTES);
        let mut codes = vec![];
        let mut bytes_used = 0;
        // Cap the number of bytecodes served per request (`MAX_SERVE_LOOKUPS`), bounding disk
        // lookups regardless of the byte budget — including an all-miss walk, which returns
        // almost nothing so the byte budget alone wouldn't stop it. Partial responses are
        // protocol-legal.
        for code_hash in request.hashes.into_iter().take(MAX_SERVE_LOOKUPS) {
            if let Some(code) = store.get_account_code(code_hash)? {
                let code = code.code_bytes();
                bytes_used += code.len() as u64;
                codes.push(code);
            }
            if bytes_used >= byte_budget {
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
        // Clamp the peer-supplied budget so a single request cannot pull an unbounded
        // number of trie nodes into memory.
        let mut byte_budget = request.bytes.min(MAX_RESPONSE_BYTES);
        // Hard cap on the number of trie-node lookups per request. Each lookup traverses the
        // trie, so this bounds disk/CPU work directly; the byte budget only caps response size.
        // Tracked across pathsets and applied *before* the lookup so an oversized pathset can't
        // blow past it in a single call.
        let mut remaining_lookups = MAX_SERVE_LOOKUPS;
        for paths in request.paths {
            if paths.is_empty() {
                return Err(SnapError::BadRequest(
                    "zero-item pathset requested".to_string(),
                ));
            }
            // Every entry in a pathset is a separate trie lookup. Truncate the pathset to the
            // remaining lookup budget (partial responses are protocol-legal) so the count cap
            // bounds the work actually done — otherwise a peer could pack many all-miss paths
            // into a single pathset and probe them all in one call.
            let paths: Vec<_> = paths
                .into_iter()
                .take(remaining_lookups)
                .map(|bytes| bytes.to_vec())
                .collect();
            remaining_lookups -= paths.len();
            let trie_nodes = store.get_trie_nodes(request.root_hash, paths, byte_budget)?;
            let returned_bytes = trie_nodes
                .iter()
                .fold(0u64, |acc, nodes| acc + nodes.len() as u64);
            nodes.extend(trie_nodes.iter().map(|nodes| Bytes::copy_from_slice(nodes)));
            // Byte budget bounds response *size*; the lookup-count cap bounds the walk.
            byte_budget = byte_budget.saturating_sub(returned_bytes);
            if byte_budget == 0 || remaining_lookups == 0 {
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
