//! Single range requests for the snap/2 download.
//!
//! These differ from their snap/1 counterparts only in shape: they answer with
//! a value instead of writing into the caller's bookkeeping, because the snap/2
//! driver schedules account and storage work against one shared frontier and
//! has to fold both kinds of answer into the same place.
//!
//! Both verify the response before returning it. `caps/snap.md` has every
//! response verified against the root the request named, and for storage that
//! root is the account's own, which is only correct while the account leaf that
//! carried it came from the same pivot. The driver guarantees that; these do
//! not check it.

use ethrex_common::{
    BigEndianHash, H256, U256,
    types::{AccountState, BlockHeader},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::verify_range;
use tracing::debug;

use crate::{
    peer_table::RequestPermit,
    rlpx::{
        connection::server::PeerConnection,
        message::Message as RLPxMessage,
        snap::{AccountRange, GetAccountRange, GetStorageRanges, StorageRanges},
    },
    snap::{
        constants::{HASH_MAX, MAX_RESPONSE_BYTES, PEER_REPLY_TIMEOUT},
        encodable_to_proof,
    },
};

/// What one account-range request produced.
pub struct AccountRangeOutcome {
    pub peer_id: H256,
    pub accounts: Vec<(H256, AccountState)>,
    /// Where to resume, or `None` once the range is served in full.
    pub remaining: Option<H256>,
    /// False when the peer failed to answer or answered unverifiably, in which
    /// case `accounts` is empty and `remaining` still covers the whole range.
    pub verified: bool,
}

/// One account's slots from a storage-range request.
pub struct StorageServed {
    pub account_hash: H256,
    pub slots: Vec<(H256, U256)>,
    /// Where to resume, for a contract too large to be served in one response.
    pub remaining: Option<H256>,
}

/// What one storage-range request produced.
pub struct StorageRangeOutcome {
    pub peer_id: H256,
    pub served: Vec<StorageServed>,
    /// Requested accounts the response did not reach. A peer may truncate from
    /// the tail, so these are simply re-requested.
    pub unserved: Vec<H256>,
    pub verified: bool,
}

/// Request the accounts in `[start, end]` at `pivot`'s state root.
pub async fn request_account_range(
    peer_id: H256,
    mut connection: PeerConnection,
    permit: RequestPermit,
    pivot: &BlockHeader,
    start: H256,
    end: H256,
) -> AccountRangeOutcome {
    let failed = || AccountRangeOutcome {
        peer_id,
        accounts: Vec::new(),
        remaining: Some(start),
        verified: false,
    };

    let state_root = pivot.state_root;
    let response = connection
        .outgoing_request(
            RLPxMessage::GetAccountRange(GetAccountRange {
                id: rand::random(),
                root_hash: state_root,
                starting_hash: start,
                limit_hash: end,
                response_bytes: MAX_RESPONSE_BYTES,
            }),
            PEER_REPLY_TIMEOUT,
        )
        .await;
    drop(permit);

    let Ok(RLPxMessage::AccountRange(AccountRange {
        accounts, proof, ..
    })) = response
    else {
        debug!(%peer_id, "snap/2 account range request failed");
        return failed();
    };
    if accounts.is_empty() {
        return failed();
    }

    let hashes: Vec<H256> = accounts.iter().map(|unit| unit.hash).collect();
    let encoded: Vec<Vec<u8>> = accounts
        .iter()
        .map(|unit| unit.account.encode_to_vec())
        .collect();
    let Ok(should_continue) = verify_range(
        state_root,
        &start,
        &hashes,
        &encoded,
        &encodable_to_proof(&proof),
    ) else {
        debug!(%peer_id, "snap/2 account range failed verification");
        return failed();
    };

    // The peer may overshoot the requested limit; anything past it belongs to
    // the next range and is dropped rather than double-counted.
    let accounts: Vec<(H256, AccountState)> = accounts
        .into_iter()
        .filter(|unit| unit.hash <= end)
        .map(|unit| (unit.hash, unit.account))
        .collect();

    let remaining = should_continue
        .then(|| hashes.last().copied().and_then(next_hash))
        .flatten()
        .filter(|next| *next <= end);

    AccountRangeOutcome {
        peer_id,
        accounts,
        remaining,
        verified: true,
    }
}

/// Request storage for `accounts`, verifying each against the matching entry in
/// `roots`.
///
/// `start` is the first slot hash wanted. It is only meaningful for a single
/// account, since the request carries one starting hash for the whole batch, so
/// a resuming contract is requested on its own.
pub async fn request_storage_ranges(
    peer_id: H256,
    mut connection: PeerConnection,
    permit: RequestPermit,
    pivot: &BlockHeader,
    accounts: Vec<H256>,
    roots: Vec<H256>,
    start: H256,
) -> StorageRangeOutcome {
    let failed = |accounts: Vec<H256>| StorageRangeOutcome {
        peer_id,
        served: Vec::new(),
        unserved: accounts,
        verified: false,
    };

    let response = connection
        .outgoing_request(
            RLPxMessage::GetStorageRanges(GetStorageRanges {
                id: rand::random(),
                root_hash: pivot.state_root,
                account_hashes: accounts.clone(),
                starting_hash: start,
                limit_hash: HASH_MAX,
                response_bytes: MAX_RESPONSE_BYTES,
            }),
            PEER_REPLY_TIMEOUT,
        )
        .await;
    drop(permit);

    let Ok(RLPxMessage::StorageRanges(StorageRanges { slots, proof, .. })) = response else {
        debug!(%peer_id, "snap/2 storage range request failed");
        return failed(accounts);
    };
    if slots.is_empty() || slots.len() > accounts.len() {
        return failed(accounts);
    }

    let proof = encodable_to_proof(&proof);
    let last = slots.len() - 1;
    let mut served = Vec::with_capacity(slots.len());

    for (index, account_slots) in slots.into_iter().enumerate() {
        if account_slots.is_empty() {
            return failed(accounts);
        }
        let (Some(account_hash), Some(root)) = (accounts.get(index), roots.get(index)) else {
            return failed(accounts);
        };

        let hashes: Vec<H256> = account_slots.iter().map(|slot| slot.hash).collect();
        let encoded: Vec<Vec<u8>> = account_slots
            .iter()
            .map(|slot| slot.data.encode_to_vec())
            .collect();

        // Only the final account in a response may be cut short, and only that
        // one carries a proof. The rest are complete ranges.
        let should_continue = if index == last && !proof.is_empty() {
            match verify_range(*root, &start, &hashes, &encoded, &proof) {
                Ok(should_continue) => should_continue,
                Err(_) => {
                    debug!(%peer_id, "snap/2 storage range failed verification");
                    return failed(accounts);
                }
            }
        } else {
            if verify_range(*root, &start, &hashes, &encoded, &[]).is_err() {
                debug!(%peer_id, "snap/2 storage range failed verification");
                return failed(accounts);
            }
            false
        };

        served.push(StorageServed {
            account_hash: *account_hash,
            slots: account_slots
                .into_iter()
                .map(|slot| (slot.hash, slot.data))
                .collect(),
            remaining: should_continue
                .then(|| hashes.last().copied().and_then(next_hash))
                .flatten(),
        });
    }

    let unserved = accounts[served.len()..].to_vec();
    StorageRangeOutcome {
        peer_id,
        served,
        unserved,
        verified: true,
    }
}

/// The first hash after `hash`, or `None` at the top of the space.
fn next_hash(hash: H256) -> Option<H256> {
    hash.into_uint()
        .checked_add(U256::one())
        .map(|next| H256::from_uint(&next))
}
