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

/// Minimum budget charged per lookup *attempt* in the snap serving handlers, independent of
/// whether the lookup hit. The response-byte clamp only advances on a hit, so without this a
/// miss/empty/overlong-path request costs nothing and the handler walks the entire
/// peer-supplied list (`hashes` / `account_hashes` / `paths`) — O(N) DB/cache probes with a
/// near-empty response, which the message-rate limiter (counting messages, not items) doesn't
/// catch. Charging a per-attempt minimum bounds that walk against the same byte budget.
/// Sized at a hash's wire length; with the 512 KiB response cap this bounds an all-miss walk
/// to ~16K probes.
const MIN_LOOKUP_COST: u64 = 32;

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

            if !account_slots.is_empty() {
                slots.push(account_slots);
            }

            // Charge every account attempt (even a miss that returned no slots) so an
            // all-miss `account_hashes` list trips the budget instead of opening the storage
            // trie once per entry.
            bytes_used += MIN_LOOKUP_COST;
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
        for code_hash in request.hashes {
            match store.get_account_code(code_hash)? {
                Some(code) => {
                    let code = code.code_bytes();
                    // Charge at least `MIN_LOOKUP_COST` even for a hit, so a long list of
                    // duplicate hashes of a *tiny* code is bounded by probe count, not just by
                    // response bytes.
                    bytes_used += (code.len() as u64).max(MIN_LOOKUP_COST);
                    codes.push(code);
                }
                // A missed lookup still costs a probe; charge it so an all-miss request trips
                // the budget instead of walking the entire `hashes` list.
                None => bytes_used += MIN_LOOKUP_COST,
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
        for paths in request.paths {
            if paths.is_empty() {
                return Err(SnapError::BadRequest(
                    "zero-item pathset requested".to_string(),
                ));
            }
            let trie_nodes = store.get_trie_nodes(
                request.root_hash,
                paths.into_iter().map(|bytes| bytes.to_vec()).collect(),
                byte_budget,
            )?;
            let returned_bytes = trie_nodes
                .iter()
                .fold(0u64, |acc, nodes| acc + nodes.len() as u64);
            nodes.extend(trie_nodes.iter().map(|nodes| Bytes::copy_from_slice(nodes)));
            // Charge at least a per-pathset probe cost so overlong/empty-result pathsets
            // (e.g. a path > 32 bytes, which resolves to an empty node) still consume the
            // budget instead of letting the loop run over every decoded pathset.
            byte_budget = byte_budget.saturating_sub(returned_bytes.max(MIN_LOOKUP_COST));
            if byte_budget == 0 {
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
