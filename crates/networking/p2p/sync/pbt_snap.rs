//! The `pbtsnap/1` sync driver: download a pivot's binary-tree state, check it,
//! and land it.
//!
//! ## What this is not
//!
//! There is no assembler. Leaves are stored exactly as received — the client
//! never reconstructs accounts, storage roots or code layout out of them, so
//! there is no reverse embedding to get wrong. What survives of that idea is
//! about thirty lines: a syntactic check on the keys, and the extraction of the
//! code hashes the EVM will later need to look up by hash.
//!
//! ## What holds it together
//!
//! One invariant, applied twice. Every response must verify against the pivot
//! header's `state_root` before it is appended ([`verify_range`]), and the
//! assembled leaf set must merkleize back to that same root before a single row
//! is written ([`Store::install_binary_snapshot`]). The first check localises a
//! lie to the peer and the request that produced it; the second is the one that
//! actually gates the disk, and it owns the verdict alone — the download layer
//! deliberately does not pre-check the root, so there is exactly one place that
//! can say yes.
//!
//! ## The whole keyspace is downloaded, including the code zone
//!
//! The plan called for downloading zones `0x00` (accounts) and `0xff` (overflow
//! storage) and *deriving* zone `0x01` (code chunks) locally from the bytecodes,
//! on the grounds that chunks are a pure function of the code. They are — but
//! the set of chunks *in the tree* is not a function of the live accounts.
//! `apply_account_update` in `ethrex-common`'s `pbt_state` removes an account's
//! header and storage prefixes and, by design, leaves its code chunks alone
//! ("Code chunks are content-addressed and shared, so they stay", and the
//! `removed_storage_keeps_the_code` test pins it). So a chain where any account
//! was ever removed while running code no surviving account runs holds chunks
//! that no live code hash can reach. Deriving zone `0x01` on such a chain
//! produces a leaf set that merkleizes to the wrong root and the sync can never
//! complete — not a soundness failure, a liveness one, and an unfixable one
//! from the client's side.
//!
//! Downloading the code zone costs bandwidth and removes that failure mode
//! entirely, so the scan is simply the whole keyspace: one cursor from the
//! first leaf to the last, zones and all. Bytecodes are still fetched, because
//! the trie commits to code as *chunks* while the EVM fetches whole bytecode by
//! hash and only `ACCOUNT_CODES` answers that.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use ethrex_binary_trie::embedding::{
    ACCOUNT_KEY_LENGTH, ACCOUNT_ZONE, BASIC_DATA_LEAF_KEY, CODE_HASH_LEAF_KEY, CODE_KEY_LENGTH,
    CODE_ZONE, DELEGATION_LEAF_KEY, HEADER_STORAGE_OFFSET, HEADER_STORAGE_SLOTS,
    STORAGE_KEY_LENGTH, STORAGE_ZONE,
};
use ethrex_binary_trie::trie::{RangeProofError, increment_key, verify_range};
use ethrex_blockchain::Blockchain;
use ethrex_common::H256;
use ethrex_common::constants::EMPTY_KECCAK_HASH;
use ethrex_common::types::{BlockHeader, Code};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::Store;
use ethrex_storage::error::StoreError;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::pbtsnap::client::{PbtProviderError, PbtSnapProvider, PeerPbtSnapProvider};
use crate::peer_handler::PeerHandler;
use crate::rlpx::pbtsnap::GetPbtLeafRange;
use crate::snap::constants::{BYTECODE_CHUNK_SIZE, MAX_RESPONSE_BYTES};
use crate::sync::SyncError;
use crate::sync::snap_sync::{
    HeaderPhase, SnapBlockSyncState, block_is_stale, download_headers_to_sync_head,
    store_block_bodies, update_pivot,
};

#[cfg(test)]
mod tests;

/// How many times one range request is re-asked after an answer that did not
/// verify.
///
/// Separate from the provider's own transport retries: this is the budget for
/// *lies*. A tampered response scores its peer down and the next selection
/// almost certainly picks someone else, so a single byzantine peer among honest
/// ones costs a round trip rather than the sync.
const RANGE_VERIFY_ATTEMPTS: u32 = 3;

/// A ceiling on round trips for one pivot, so a peer that answers with exactly
/// one leaf every time cannot hold the sync open indefinitely.
///
/// It is not a leaf count: the progress rule means every answered request
/// advances the cursor by at least one leaf, so this bounds work rather than
/// correctness. Devnet states are far below it; a real one that legitimately
/// needs more rounds should be raising `response_bytes`, not this.
const MAX_RANGE_ROUNDS: u32 = 1_000_000;

/// Why a PBT state download did not produce an installable snapshot.
#[derive(Debug, thiserror::Error)]
pub enum PbtSyncError {
    #[error(transparent)]
    Provider(#[from] PbtProviderError),
    #[error("a served range did not verify against the pivot root: {0}")]
    Range(#[from] RangeProofError),
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A key in a zone the embedding does not define. Zones `2..=254` are
    /// reserved, and a client must not silently install state in one.
    #[error("leaf key in undefined zone {0}")]
    UnknownZone(u8),
    /// A key whose length disagrees with its zone's fixed width. The key order
    /// this protocol resumes through (`increment_key`) is only a true successor
    /// function because the key set is prefix-free, which holds only if every
    /// zone's width is what it claims — so this is load-bearing, not tidiness.
    #[error("leaf key in zone {zone} is {length} bytes, expected {expected}")]
    BadKeyLength {
        zone: u8,
        length: usize,
        expected: usize,
    },
    /// An account-zone sub-index the embedding does not assign.
    ///
    /// The one client-side rule the root check cannot subsume: a server could
    /// hold such a leaf legitimately if the embedding ever grew, so the root
    /// would verify and the client would install state it cannot interpret.
    #[error("account-zone leaf at unassigned sub-index {0}")]
    UnknownSubIndex(u8),
    /// The peer answered `has_more` at the greatest possible key, which has no
    /// successor to resume from.
    #[error("the range cursor ran past the end of the keyspace")]
    CursorExhausted,
    #[error("gave up on a leaf range after {0} answers that did not verify")]
    RangeUnverified(u32),
    #[error("the download exceeded {0} rounds without finishing")]
    TooManyRounds(u32),
    #[error("peer returned {got} bytecodes for {want} hashes")]
    BytecodeCountMismatch { want: usize, got: usize },
    #[error("bytecode for {expected:#x} hashes to {actual:#x}")]
    BytecodeHashMismatch { expected: H256, actual: H256 },
    #[error(
        "bytecode for {hash:#x} is {actual} bytes, but its account's basic data says {expected}"
    )]
    BytecodeSizeMismatch {
        hash: H256,
        expected: u32,
        actual: usize,
    },
}

/// How many pivots one cycle will try before giving up.
///
/// Pivot restart *is* the v1 healing story: a failed download has written
/// nothing, so starting over on a fresher pivot is the whole recovery. It is
/// bounded because with a serving window of roughly `DB_COMMIT_THRESHOLD`
/// blocks, an unbounded retry loop on a link slower than that window is a
/// livelock rather than a recovery — the pivot ages out faster than the state
/// arrives, forever, silently.
const MAX_PIVOTS_PER_CYCLE: u32 = 3;

/// The `pbtsnap` sync cycle: headers, then a pivot's binary state, then the
/// switch-over.
///
/// Reuses the snap cycle's header phase verbatim — headers are headers under
/// either commitment — and replaces everything after it.
pub(crate) async fn sync_cycle_pbt(
    peers: &mut PeerHandler,
    blockchain: Arc<Blockchain>,
    snap_enabled: &AtomicBool,
    sync_head: H256,
    store: Store,
    diagnostics: &Arc<tokio::sync::RwLock<super::SyncDiagnostics>>,
) -> Result<(), SyncError> {
    let mut block_sync_state = SnapBlockSyncState::new(store.clone());
    diagnostics.write().await.sync_mode = "pbtsnap".to_string();

    match download_headers_to_sync_head(
        peers,
        sync_head,
        &store,
        &mut block_sync_state,
        diagnostics,
    )
    .await?
    {
        HeaderPhase::FullSync => {
            info!("Sync head is found, switching to FullSync");
            return fall_back_to_full(
                peers,
                blockchain,
                snap_enabled,
                sync_head,
                store,
                diagnostics,
            )
            .await;
        }
        HeaderPhase::Abandoned => return Ok(()),
        HeaderPhase::Downloaded => {}
    }

    let provider = PeerPbtSnapProvider::new(peers.clone());
    let mut pivot_header = {
        let pivot_hash = *block_sync_state
            .block_hashes
            .last()
            .ok_or(SyncError::NoBlockHeaders)?;
        store
            .get_block_header_by_hash(pivot_hash)?
            .ok_or(SyncError::CorruptDB)?
    };

    let mut last_error = None;
    for attempt in 1..=MAX_PIVOTS_PER_CYCLE {
        while block_is_stale(&pivot_header) {
            pivot_header = update_pivot(
                pivot_header.number,
                pivot_header.timestamp,
                peers,
                &mut block_sync_state,
                diagnostics,
            )
            .await?;
        }

        // Decision 12. A pivot before the activation timestamp commits an MPT,
        // which this protocol cannot download; that is a legitimate outcome on a
        // chain whose flip has not happened yet, so it falls back rather than
        // failing.
        if !store
            .get_chain_config()
            .is_binary_tree_active(pivot_header.timestamp)
        {
            warn!(
                pivot = pivot_header.number,
                timestamp = pivot_header.timestamp,
                "Pivot is before the binary-tree activation; falling back to full sync"
            );
            return fall_back_to_full(
                peers,
                blockchain,
                snap_enabled,
                sync_head,
                store,
                diagnostics,
            )
            .await;
        }

        {
            let mut diag = diagnostics.write().await;
            diag.current_phase = "pbt_leaves".to_string();
            diag.pivot_block_number = Some(pivot_header.number);
            diag.pivot_timestamp = Some(pivot_header.timestamp);
        }
        info!(
            pivot = pivot_header.number,
            attempt, "Downloading binary-tree state"
        );

        match sync_pbt_state(&provider, &store, &pivot_header).await {
            Ok(root) => {
                info!(pivot = pivot_header.number, %root, "Installed the pivot's binary state");
                return finish_pbt_cycle(
                    peers,
                    snap_enabled,
                    &store,
                    &block_sync_state,
                    &pivot_header,
                )
                .await;
            }
            Err(error) => {
                // Nothing was written, so the only state to discard is the
                // pivot itself. The commonest cause by far is a pivot that aged
                // out of every peer's layer window, which a fresher one fixes.
                warn!(
                    pivot = pivot_header.number,
                    attempt, %error, "Binary-tree state download failed; re-pivoting"
                );
                last_error = Some(error);
                if attempt < MAX_PIVOTS_PER_CYCLE {
                    pivot_header = update_pivot(
                        pivot_header.number,
                        pivot_header.timestamp,
                        peers,
                        &mut block_sync_state,
                        diagnostics,
                    )
                    .await?;
                }
            }
        }
    }
    Err(last_error
        .map(SyncError::from)
        .unwrap_or(SyncError::NoBlockHeaders))
}

/// Abandon the PBT cycle and full-sync instead.
///
/// Clears the snap checkpoint on the way out: leaving one behind makes the
/// manager loop re-enter this branch after the full sync completes, which is
/// the same cleanup `sync_cycle` does when it auto-switches on a short chain.
async fn fall_back_to_full(
    peers: &mut PeerHandler,
    blockchain: Arc<Blockchain>,
    snap_enabled: &AtomicBool,
    sync_head: H256,
    store: Store,
    diagnostics: &Arc<tokio::sync::RwLock<super::SyncDiagnostics>>,
) -> Result<(), SyncError> {
    snap_enabled.store(false, Ordering::Relaxed);
    store.clear_snap_state().await?;
    super::full::sync_cycle_full(
        peers,
        blockchain,
        CancellationToken::new(),
        sync_head,
        store,
        diagnostics,
    )
    .await
}

/// The switch-over, mirroring `snap_sync`'s: the pivot's body, then the
/// forkchoice update that makes it the head.
async fn finish_pbt_cycle(
    peers: &mut PeerHandler,
    snap_enabled: &AtomicBool,
    store: &Store,
    block_sync_state: &SnapBlockSyncState,
    pivot_header: &BlockHeader,
) -> Result<(), SyncError> {
    store_block_bodies(vec![pivot_header.clone()], peers.clone(), store.clone()).await?;
    let block = store
        .get_block_by_hash(pivot_header.hash())
        .await?
        .ok_or(SyncError::CorruptDB)?;
    store.add_block(block).await?;

    let numbers_and_hashes = block_sync_state
        .block_hashes
        .iter()
        .rev()
        .enumerate()
        .map(|(i, hash)| (pivot_header.number - i as u64, *hash))
        .collect::<Vec<_>>();
    store
        .forkchoice_update(
            numbers_and_hashes,
            pivot_header.number,
            pivot_header.hash(),
            None,
            None,
        )
        .await?;

    store.clear_snap_state().await?;
    snap_enabled.store(false, Ordering::Relaxed);
    Ok(())
}

/// Download the binary-tree state committed by `pivot`, verify it, and install
/// it.
///
/// On success the store holds `pivot.state_root` and
/// [`Store::get_binary_trie_root`] answers for the pivot's hash. On failure
/// nothing has been written — every rejection happens before
/// [`Store::install_binary_snapshot`] is reached, and that call itself refuses
/// before writing.
///
/// The pivot's **header must already be stored**: installation resolves the
/// root it must match through it, and a snapshot with no header to anchor it is
/// exactly the thing that must not be installable.
pub async fn sync_pbt_state<P: PbtSnapProvider>(
    provider: &P,
    store: &Store,
    pivot: &BlockHeader,
) -> Result<H256, PbtSyncError> {
    let root = pivot.state_root;
    let leaves = download_leaves(provider, root).await?;
    info!(
        leaves = leaves.len(),
        pivot = pivot.number,
        "Downloaded a binary-tree state snapshot"
    );

    let wanted = code_requests(&leaves)?;
    let codes = download_codes(provider, &wanted).await?;

    let installed = store
        .install_binary_snapshot(pivot.hash(), leaves, codes)
        .await?;
    Ok(installed)
}

/// Walk the whole keyspace at `root`, verifying every answer before keeping it.
async fn download_leaves<P: PbtSnapProvider>(
    provider: &P,
    root: H256,
) -> Result<Vec<(Vec<u8>, [u8; 32])>, PbtSyncError> {
    let mut leaves: Vec<(Vec<u8>, [u8; 32])> = Vec::new();
    // The empty origin is the "from the first leaf" sentinel, and an empty
    // limit is "no upper bound" — the store reconciles that sentinel with
    // `prove_range`'s literal comparison, which would otherwise make the first
    // leaf a terminator and cap every response at one leaf.
    let mut origin: Vec<u8> = Vec::new();
    let mut request_id = 0u64;

    for round in 0..MAX_RANGE_ROUNDS {
        request_id = request_id.wrapping_add(1);
        let request = GetPbtLeafRange {
            id: request_id,
            root_hash: root,
            origin: Bytes::from(origin.clone()),
            limit: Bytes::new(),
            response_bytes: MAX_RESPONSE_BYTES,
        };

        let (batch, has_more) = fetch_verified_range(provider, root, &origin, request).await?;
        validate_keys(&batch)?;

        let last = batch.last().map(|(key, _)| key.clone());
        leaves.extend(batch);

        if !has_more {
            debug!(rounds = round + 1, "Leaf download complete");
            return Ok(leaves);
        }
        // `verify_range` only reports `has_more` from a *right* walk, which
        // exists only when the range is non-empty; an empty verified range is
        // always `has_more: false`. So this is unreachable rather than merely
        // unexpected, and it is an error rather than a break because treating
        // it as completion is how a truncated state would get installed.
        let last = last.ok_or(PbtSyncError::CursorExhausted)?;
        origin = increment_key(&last).ok_or(PbtSyncError::CursorExhausted)?;
    }
    Err(PbtSyncError::TooManyRounds(MAX_RANGE_ROUNDS))
}

/// One range, re-asked until it verifies or the budget runs out.
///
/// The retry is what makes a single byzantine peer survivable: the same request
/// goes back to the peer table, which by then has scored the liar down.
async fn fetch_verified_range<P: PbtSnapProvider>(
    provider: &P,
    root: H256,
    origin: &[u8],
    request: GetPbtLeafRange,
) -> Result<(Vec<(Vec<u8>, [u8; 32])>, bool), PbtSyncError> {
    let mut last_error = None;
    for attempt in 1..=RANGE_VERIFY_ATTEMPTS {
        let response = provider.get_leaf_range(request.clone()).await?;
        let batch: Vec<(Vec<u8>, [u8; 32])> = response
            .leaves
            .iter()
            .map(|leaf| (leaf.key.to_vec(), leaf.value.0))
            .collect();
        let left: Vec<Vec<u8>> = response.left_proof.iter().map(|n| n.to_vec()).collect();
        let right: Vec<Vec<u8>> = response.right_proof.iter().map(|n| n.to_vec()).collect();

        match verify_range(root, origin, &batch, &left, &right) {
            Ok(verified) => return Ok((batch, verified.has_more)),
            Err(error) => {
                debug!(
                    %error,
                    attempt,
                    "A pbtsnap range did not verify against the pivot root"
                );
                last_error = Some(error);
            }
        }
    }
    // Surface the peer's actual lie rather than a bare count: `Proof(..)` and
    // `MissingLeaves` and `RootMismatch` are three quite different diagnoses.
    match last_error {
        Some(error) => Err(PbtSyncError::Range(error)),
        None => Err(PbtSyncError::RangeUnverified(RANGE_VERIFY_ATTEMPTS)),
    }
}

/// What the client insists on beyond "these leaves are genuine".
///
/// Everything here is *syntactic*, and none of it is subsumed by the root
/// check: a peer serving a real chain's state passes all of it, and a peer
/// serving state from a future embedding this client does not implement fails
/// it while still merkleizing correctly. Installing the latter is how a node
/// ends up holding state it cannot read.
fn validate_keys(leaves: &[(Vec<u8>, [u8; 32])]) -> Result<(), PbtSyncError> {
    for (key, _) in leaves {
        let zone = *key.first().ok_or(PbtSyncError::UnknownZone(0))?;
        let expected = match zone {
            ACCOUNT_ZONE => ACCOUNT_KEY_LENGTH,
            CODE_ZONE => CODE_KEY_LENGTH,
            STORAGE_ZONE => STORAGE_KEY_LENGTH,
            other => return Err(PbtSyncError::UnknownZone(other)),
        };
        if key.len() != expected {
            return Err(PbtSyncError::BadKeyLength {
                zone,
                length: key.len(),
                expected,
            });
        }
        if zone == ACCOUNT_ZONE {
            let sub_index = key[ACCOUNT_KEY_LENGTH - 1];
            let header_storage = (HEADER_STORAGE_OFFSET
                ..HEADER_STORAGE_OFFSET + HEADER_STORAGE_SLOTS)
                .contains(&u64::from(sub_index));
            let assigned = matches!(
                sub_index,
                BASIC_DATA_LEAF_KEY | CODE_HASH_LEAF_KEY | DELEGATION_LEAF_KEY
            ) || header_storage;
            if !assigned {
                return Err(PbtSyncError::UnknownSubIndex(sub_index));
            }
        }
    }
    Ok(())
}

/// The bytecodes this state needs in `ACCOUNT_CODES`, and the size each
/// account's basic data claims for it.
///
/// Only accounts with a code-hash leaf contribute. A delegated account carries
/// a delegation leaf instead: its "code" *is* that leaf, it is never chunkified
/// and it needs no `ACCOUNT_CODES` row, so it contributes nothing here.
fn code_requests(leaves: &[(Vec<u8>, [u8; 32])]) -> Result<Vec<(H256, u32)>, PbtSyncError> {
    // Keyed by stem so the basic-data and code-hash leaves of one account can
    // be paired without assuming they arrived in the same response.
    let mut code_size_by_stem: HashMap<&[u8], u32> = HashMap::new();
    let mut hash_by_stem: BTreeMap<&[u8], H256> = BTreeMap::new();

    for (key, value) in leaves {
        if key.first() != Some(&ACCOUNT_ZONE) || key.len() != ACCOUNT_KEY_LENGTH {
            continue;
        }
        let stem = &key[..ACCOUNT_KEY_LENGTH - 1];
        match key[ACCOUNT_KEY_LENGTH - 1] {
            BASIC_DATA_LEAF_KEY => {
                // The 4-byte code size at offset 4; see `encode_basic_data`,
                // whose layout follows the EELS branch the crate is ported
                // from rather than EIP-7864's 3-byte field at offset 5.
                let size = u32::from_be_bytes([value[4], value[5], value[6], value[7]]);
                code_size_by_stem.insert(stem, size);
            }
            CODE_HASH_LEAF_KEY => {
                hash_by_stem.insert(stem, H256(*value));
            }
            _ => {}
        }
    }

    // One request per distinct hash, but keep the size claim so a substituted
    // bytecode fails with a targeted error rather than an opaque root mismatch
    // three steps later.
    let mut wanted: BTreeMap<H256, u32> = BTreeMap::new();
    for (stem, hash) in hash_by_stem {
        if hash == *EMPTY_KECCAK_HASH {
            continue;
        }
        let size = code_size_by_stem.get(stem).copied().unwrap_or_default();
        wanted.entry(hash).or_insert(size);
    }
    Ok(wanted.into_iter().collect())
}

/// Fetch and check the bytecodes, in `BYTECODE_CHUNK_SIZE` batches.
async fn download_codes<P: PbtSnapProvider>(
    provider: &P,
    wanted: &[(H256, u32)],
) -> Result<Vec<Code>, PbtSyncError> {
    let mut codes = Vec::with_capacity(wanted.len());
    for batch in wanted.chunks(BYTECODE_CHUNK_SIZE) {
        let hashes: Vec<H256> = batch.iter().map(|(hash, _)| *hash).collect();
        let bytecodes = provider.get_bytecodes(&hashes).await?;
        if bytecodes.len() != hashes.len() {
            return Err(PbtSyncError::BytecodeCountMismatch {
                want: hashes.len(),
                got: bytecodes.len(),
            });
        }
        for ((hash, size), bytecode) in batch.iter().zip(bytecodes) {
            // The driver owns this invariant even though `request_bytecodes`
            // also checks it: the driver is what a hostile provider is aimed
            // at, and a seam that trusts its transport is not a seam.
            let code = Code::from_bytecode(bytecode, &NativeCrypto);
            if code.hash != *hash {
                return Err(PbtSyncError::BytecodeHashMismatch {
                    expected: *hash,
                    actual: code.hash,
                });
            }
            if code.code().len() != *size as usize {
                return Err(PbtSyncError::BytecodeSizeMismatch {
                    hash: *hash,
                    expected: *size,
                    actual: code.code().len(),
                });
            }
            codes.push(code);
        }
    }
    Ok(codes)
}
