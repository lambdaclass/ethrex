//! Rolling the snap/2 flat state forward when the pivot moves.
//!
//! From devp2p `caps/snap.md`, "Synchronization algorithm":
//!
//! > 3. As the chain advances from `P` to `P+K`, fetch BALs for `P+1..P+K` via
//! >    `GetBlockAccessLists`, verify each against the `block_access_list_hash`
//! >    of its header, and apply the resulting state diff to the partial flat
//! >    state. `P+K` is then the target for any remaining range requests.
//! > 4. Repeat step 3 if the pivot advances again during catch-up.
//!
//! Every pivot move runs this before the download resumes against the new root,
//! so the flat state is never more than one pivot behind the range requests.

use std::{future::Future, sync::Arc};

use ethrex_common::{
    H256,
    constants::{AMSTERDAM_MAX_CODE_SIZE, EMPTY_BLOCK_ACCESS_LIST_HASH},
    types::{BlockHeader, block_access_list::BlockAccessList},
    validate_bal_code_sizes,
};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::Store;
use tracing::{debug, info, warn};

use crate::{
    peer_handler::{PeerHandler, PeerHandlerError},
    peer_table::{PeerTable, PeerTableServerProtocol as _},
    snap::constants::{BAL_MAX_RETRIES_PER_BLOCK, BAL_REQUEST_BATCH_SIZE},
    sync::{
        SyncDiagnostics, SyncError,
        snap2::{DownloadCursor, FlatState, apply_bal_flat},
    },
};

/// Furthest the flat state may be rolled forward before the access lists it
/// needs are assumed gone.
///
/// [EIP-7928] requires the execution layer to retain access lists for at least
/// the weak subjectivity period, 3533 epochs, so that is the largest gap a peer
/// is obliged to be able to serve. Past it a catch-up would stall partway
/// through, and `caps/snap.md` has the node discard its partial state and
/// resync instead.
///
/// [EIP-7928]: https://eips.ethereum.org/EIPS/eip-7928
pub const MAX_CATCH_UP_BLOCKS: u64 = 3533 * 32;

/// Whether rolling the flat state from `from` to `to` would span more blocks
/// than peers are required to retain access lists for.
pub fn catch_up_exceeds_retention(from: &BlockHeader, to: &BlockHeader) -> bool {
    to.number.saturating_sub(from.number) > MAX_CATCH_UP_BLOCKS
}

/// Apply the access lists of every block in `(from, to]` to the flat state.
///
/// Blocks are applied in strict order. `caps/snap.md`: "BALs **must** be
/// applied in strict block order against the correct fork, with each BAL hash
/// verified before application. A wrong-fork or out-of-order BAL produces an
/// invalid state root, detected at the final root check."
///
/// Unlike the snap/1 replay this does not check a state root per block: there
/// is no complete trie to hash mid-download, and the flat state deliberately
/// holds leaves from several roots. The single check is the reconstruction's.
pub async fn catch_up(
    store: &Store,
    peers: &mut PeerHandler,
    flat: &FlatState,
    cursor: &DownloadCursor,
    from: &BlockHeader,
    to: &BlockHeader,
    diagnostics: &Arc<tokio::sync::RwLock<SyncDiagnostics>>,
) -> Result<(), SyncError> {
    let peers: &PeerHandler = peers;
    catch_up_via(
        store,
        &peers.peer_table,
        move |pending, exclude| async move { peers.request_snap2_bals(&pending, &exclude).await },
        flat,
        cursor,
        from,
        to,
        diagnostics,
    )
    .await
}

/// What one request step yields: the entries in request order and the peer
/// that served them, or nothing once no peer could be asked.
type ServedBals = Option<(Vec<Option<BlockAccessList>>, H256)>;

/// [`catch_up`] with the request step injected, so tests can script what the
/// peers answer while the real peer table scores and ranks them. The step takes
/// the pending block hashes and the peers not to ask.
#[allow(clippy::too_many_arguments)]
async fn catch_up_via<F>(
    store: &Store,
    peer_table: &PeerTable,
    request_bals: impl Fn(Vec<H256>, Vec<H256>) -> F,
    flat: &FlatState,
    cursor: &DownloadCursor,
    from: &BlockHeader,
    to: &BlockHeader,
    diagnostics: &Arc<tokio::sync::RwLock<SyncDiagnostics>>,
) -> Result<(), SyncError>
where
    F: Future<Output = Result<ServedBals, PeerHandlerError>>,
{
    if to.number <= from.number {
        return Ok(());
    }
    let headers = gap_headers(store, from, to)?;
    info!(
        from = from.number,
        to = to.number,
        blocks = headers.len(),
        "snap/2 catch-up: rolling flat state forward"
    );

    for batch in headers.chunks(BAL_REQUEST_BATCH_SIZE) {
        let mut applied = 0usize;
        // Failures since the last block that applied. Every failure concerns the
        // first pending block — the run stops at the first bad or missing entry —
        // so this is that block's retry count.
        let mut attempts = 0u32;
        // Peers that answered without the first pending block since it was last
        // requested. A retry skips them, so one well-ranked peer that lacks a
        // list cannot take every attempt while a peer holding it is never asked;
        // a stall then means the peers we could ask all answered without it.
        let mut tried: Vec<H256> = Vec::new();

        while applied < batch.len() {
            let pending: Vec<H256> = batch[applied..]
                .iter()
                .map(|header| header.hash())
                .collect();

            let (response, peer_id) = loop {
                diagnostics.write().await.snap2_bal_requests_sent += 1;
                match request_bals(pending.clone(), tried.clone()).await? {
                    Some(served) => break served,
                    // Every peer not yet tried for this block has been. Ask the
                    // tried ones again, until the retry cap below settles it.
                    None if !tried.is_empty() => tried.clear(),
                    None => {
                        diagnostics.write().await.snap2_peer_failures += 1;
                        return Err(SyncError::Snap2CatchUpStalled(
                            batch[applied].number,
                            "no peer could serve block access lists".to_string(),
                        ));
                    }
                }
            };

            // A response may be truncated from the tail but must cover at least
            // the first slot, or nothing progresses.
            if response.is_empty() {
                tried.push(peer_id);
                attempts += 1;
                let _ = if attempts >= BAL_MAX_RETRIES_PER_BLOCK {
                    peer_table.record_critical_failure(peer_id)
                } else {
                    peer_table.record_failure(peer_id)
                };
                diagnostics.write().await.snap2_peer_failures += 1;
                if attempts >= BAL_MAX_RETRIES_PER_BLOCK {
                    return Err(SyncError::Snap2CatchUpStalled(
                        batch[applied].number,
                        "peers returned no entries".to_string(),
                    ));
                }
                continue;
            }

            for bal in response {
                let header = &batch[applied];

                let Some(bal) = bal else {
                    // The peer does not hold this one. Stop the run here and
                    // re-request the rest; another peer may have it. This is a
                    // failed attempt like any other: left uncharged, a peer
                    // holding nothing would stay the best ranked and be asked
                    // for the same block forever.
                    diagnostics.write().await.snap2_bals_unavailable += 1;
                    tried.push(peer_id);
                    attempts += 1;
                    if attempts >= BAL_MAX_RETRIES_PER_BLOCK {
                        let _ = peer_table.record_critical_failure(peer_id);
                        return Err(SyncError::Snap2CatchUpStalled(
                            header.number,
                            "peers report the block access list unavailable".to_string(),
                        ));
                    }
                    let _ = peer_table.record_failure(peer_id);
                    break;
                };

                // A list that is malformed is as bad as one with the wrong hash:
                // the peer served data it should not have. Charged the same way,
                // or the peer would stay best ranked and serve it again after
                // the restart this error causes.
                let malformed = bal
                    .validate_ordering()
                    .map_err(|err| format!("bad ordering: {err}"))
                    .and_then(|()| {
                        validate_bal_code_sizes(&bal, AMSTERDAM_MAX_CODE_SIZE)
                            .map_err(|err| format!("bad code size: {err}"))
                    });
                if let Err(reason) = malformed {
                    let _ = peer_table.record_critical_failure(peer_id);
                    diagnostics.write().await.snap2_validation_failures += 1;
                    return Err(SyncError::Snap2CatchUpStalled(header.number, reason));
                }
                let expected = header
                    .block_access_list_hash
                    .unwrap_or(*EMPTY_BLOCK_ACCESS_LIST_HASH);
                let actual = bal.compute_hash(&NativeCrypto);
                if actual != expected {
                    let _ = peer_table.record_critical_failure(peer_id);
                    diagnostics.write().await.snap2_validation_failures += 1;
                    return Err(SyncError::Snap2CatchUpStalled(
                        header.number,
                        format!("access list hash {actual:?} does not match header {expected:?}"),
                    ));
                }

                let stats = apply_bal_flat(flat, cursor, store, &bal)?;
                debug!(
                    block = header.number,
                    accounts = stats.accounts_written,
                    slots = stats.slots_written,
                    skipped_accounts = stats.accounts_skipped,
                    skipped_slots = stats.slots_skipped,
                    "snap/2 catch-up applied an access list"
                );

                // Serving this list onward costs nothing and a peer that has
                // just synced is the one best placed to help the next.
                if let Err(err) = store.store_block_access_list(header.hash(), &bal) {
                    warn!(block = header.number, "failed to persist a BAL: {err}");
                }

                applied += 1;
                attempts = 0;
                tried.clear();
                diagnostics.write().await.snap2_blocks_replayed += 1;
                // Credit goes per verified and applied list, never for the
                // reply as such: one made only of unavailable markers served
                // nothing.
                let _ = peer_table.record_success(peer_id);
            }
        }
    }

    Ok(())
}

/// The headers of every block in `(from, to]`, resolved by walking parent
/// hashes back from `to`.
///
/// The target header is the trusted one — it came from a peer as the new pivot
/// — so the chain is resolved from it rather than read forwards out of the
/// canonical index. During a snap sync there is no canonical index to read:
/// headers downloaded ahead of the pivot are stored by hash, and the chain is
/// only made canonical once the sync commits the pivot at the very end.
///
/// Walking backwards is also what detects a reorg past the pivot. If the chain
/// behind `to` does not reach `from`, the old pivot was orphaned and the flat
/// state built against it describes an abandoned fork. caps/snap.md allows
/// recovering that state by diffing the two forks' access lists; ethrex takes
/// the escape hatch the same section gives instead, and discards. A reorg
/// deeper than the pivot's distance from the head has not occurred on mainnet
/// and is bounded by finality.
pub fn gap_headers(
    store: &Store,
    from: &BlockHeader,
    to: &BlockHeader,
) -> Result<Vec<BlockHeader>, SyncError> {
    let span = to.number.saturating_sub(from.number) as usize;
    let mut headers = Vec::with_capacity(span);
    let mut current = to.clone();

    while current.number > from.number + 1 {
        let parent = store
            .get_block_header_by_hash(current.parent_hash)?
            .ok_or(SyncError::MissingHeaderForBal(current.parent_hash))?;
        headers.push(current);
        current = parent;
    }

    if current.parent_hash != from.hash() {
        return Err(SyncError::ChainReorgDetected {
            expected_parent: from.hash(),
            actual_parent: current.parent_hash,
        });
    }
    headers.push(current);
    headers.reverse();
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        net::{IpAddr, Ipv4Addr},
        sync::Mutex,
        time::Duration,
    };

    use ethrex_common::{
        Address, H512, U256,
        types::block_access_list::{AccountChanges, BalanceChange},
    };
    use ethrex_storage::EngineType;

    use super::*;
    use crate::{
        peer_table::{MIN_SCORE_CRITICAL, PeerTableServer},
        rlpx::{
            connection::server::{ConnectionId, PeerConnection},
            p2p::Capability,
        },
        types::Node,
    };

    /// What a scripted peer answers to every request.
    #[derive(Clone, Copy)]
    enum Answer {
        /// Every entry is the unavailable marker.
        Unavailable,
        /// Every requested list, verbatim.
        Serve,
    }

    type Chain = (
        BlockHeader,
        Vec<BlockHeader>,
        HashMap<H256, BlockAccessList>,
    );

    /// Blocks `1..=len` on top of a genesis-shaped header, each committing to
    /// its own access list. The headers are stored so `gap_headers` can walk
    /// them; the lists are returned by block hash for a peer to serve.
    async fn chain(store: &Store, len: u64) -> Chain {
        let from = BlockHeader::default();
        let mut parent_hash = from.hash();
        let mut headers = Vec::new();
        let mut bals = HashMap::new();
        for number in 1..=len {
            let bal = BlockAccessList::from_accounts(vec![
                AccountChanges::new(Address::from_low_u64_be(number))
                    .with_balance_changes(vec![BalanceChange::new(1, U256::from(number))]),
            ]);
            // Every optional field up to the commitment is set: the RLP encoding
            // a stored header is read back from stops at the first absent one.
            let header = BlockHeader {
                number,
                parent_hash,
                base_fee_per_gas: Some(0),
                withdrawals_root: Some(H256::zero()),
                blob_gas_used: Some(0),
                excess_blob_gas: Some(0),
                parent_beacon_block_root: Some(H256::zero()),
                requests_hash: Some(H256::zero()),
                block_access_list_hash: Some(bal.compute_hash(&NativeCrypto)),
                ..Default::default()
            };
            parent_hash = header.hash();
            store
                .add_block_header(parent_hash, header.clone())
                .await
                .expect("store header");
            bals.insert(parent_hash, bal);
            headers.push(header);
        }
        (from, headers, bals)
    }

    fn node(seed: u8) -> Node {
        Node::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, seed)),
            30303,
            30303,
            H512::from_low_u64_be(seed as u64),
        )
    }

    /// A live peer table with every node registered as a connected snap/2 peer.
    async fn table_with(nodes: &[Node]) -> PeerTable {
        let table = PeerTableServer::spawn(10);
        for node in nodes {
            let registered = table
                .new_connected_peer(
                    node.clone(),
                    PeerConnection::unestablished(),
                    ConnectionId::next(),
                    vec![Capability::snap(2)],
                    vec![Capability::snap(2)],
                    None,
                    false,
                )
                .await
                .expect("register peer");
            assert!(registered, "each test node is distinct");
        }
        table
    }

    /// The request step of a catch-up: the real peer table chooses whom to ask,
    /// skipping `exclude` as the real step does, and the script answers for it.
    /// Every choice is recorded in `asked`.
    async fn scripted_request(
        table: &PeerTable,
        script: &HashMap<H256, Answer>,
        bals: &HashMap<H256, BlockAccessList>,
        asked: &Mutex<Vec<H256>>,
        pending: Vec<H256>,
        exclude: Vec<H256>,
    ) -> Result<ServedBals, PeerHandlerError> {
        let Some((peer_id, _connection, _permit)) = table
            .get_best_peer_excluding(vec![Capability::snap(2)], exclude)
            .await?
        else {
            return Ok(None);
        };
        asked.lock().expect("asked").push(peer_id);
        let reply = match script[&peer_id] {
            Answer::Unavailable => vec![None; pending.len()],
            Answer::Serve => pending.iter().map(|hash| bals.get(hash).cloned()).collect(),
        };
        Ok(Some((reply, peer_id)))
    }

    /// A flat state, with the directory that must outlive it when it is on disk.
    fn flat_state() -> (tempfile::TempDir, FlatState) {
        let dir = tempfile::tempdir().expect("tempdir");
        let flat = FlatState::open(dir.path()).expect("flat state");
        (dir, flat)
    }

    fn diagnostics() -> Arc<tokio::sync::RwLock<SyncDiagnostics>> {
        Arc::new(tokio::sync::RwLock::new(SyncDiagnostics::default()))
    }

    /// A peer answering that it holds none of the lists SHALL be charged for
    /// the reply and earn nothing from it, so the next request goes to a peer
    /// that serves, which SHALL be credited once per applied list.
    #[tokio::test]
    async fn unavailable_reply_demotes_the_peer_and_the_next_request_goes_elsewhere() {
        let store = Store::new("memory", EngineType::InMemory).expect("in-memory store");
        let (from, headers, bals) = chain(&store, 3).await;
        let to = headers.last().expect("chain").clone();
        let (empty, full) = (node(1), node(2));
        let table = table_with(&[empty.clone(), full.clone()]).await;
        // The empty peer starts one point ahead, so it is asked first. Once it
        // is charged the two tie, and a tie goes to the later-registered peer.
        table.record_success(empty.node_id()).expect("score");
        let script = HashMap::from([
            (empty.node_id(), Answer::Unavailable),
            (full.node_id(), Answer::Serve),
        ]);
        let (_dir, flat) = flat_state();
        let (cursor, diagnostics) = (DownloadCursor::new(1), diagnostics());
        let asked = Mutex::new(Vec::new());

        tokio::time::timeout(
            Duration::from_secs(10),
            catch_up_via(
                &store,
                &table,
                |pending, exclude| {
                    scripted_request(&table, &script, &bals, &asked, pending, exclude)
                },
                &flat,
                &cursor,
                &from,
                &to,
                &diagnostics,
            ),
        )
        .await
        .expect("the catch-up must move on to the serving peer instead of looping")
        .expect("catch-up completes");

        assert_eq!(
            asked.into_inner().expect("asked"),
            vec![empty.node_id(), full.node_id()]
        );
        assert_eq!(
            table.get_score(empty.node_id()).await.expect("score"),
            0,
            "the unavailable reply cost the point the peer had"
        );
        assert_eq!(
            table.get_score(full.node_id()).await.expect("score"),
            3,
            "one point per applied list"
        );
        let (best, _, _) = table
            .get_best_peer_excluding(vec![Capability::snap(2)], vec![])
            .await
            .expect("table")
            .expect("a peer");
        assert_eq!(best, full.node_id());
        let diagnostics = diagnostics.read().await;
        assert_eq!(diagnostics.snap2_bals_unavailable, 1);
        assert_eq!(diagnostics.snap2_blocks_replayed, 3);
    }

    /// With the only peer reporting the first pending list unavailable every
    /// time, the catch-up SHALL stop at the per-block retry cap and mark the
    /// peer critical, instead of asking it again forever.
    #[tokio::test]
    async fn unavailable_replies_exhaust_the_retry_cap() {
        let store = Store::new("memory", EngineType::InMemory).expect("in-memory store");
        let (from, headers, bals) = chain(&store, 3).await;
        let to = headers.last().expect("chain").clone();
        let empty = node(1);
        let table = table_with(std::slice::from_ref(&empty)).await;
        let script = HashMap::from([(empty.node_id(), Answer::Unavailable)]);
        let (_dir, flat) = flat_state();
        let (cursor, diagnostics) = (DownloadCursor::new(1), diagnostics());
        let asked = Mutex::new(Vec::new());

        let result = tokio::time::timeout(
            Duration::from_secs(10),
            catch_up_via(
                &store,
                &table,
                |pending, exclude| {
                    scripted_request(&table, &script, &bals, &asked, pending, exclude)
                },
                &flat,
                &cursor,
                &from,
                &to,
                &diagnostics,
            ),
        )
        .await
        .expect("the catch-up must stop at the retry cap instead of looping");

        assert!(
            matches!(result, Err(SyncError::Snap2CatchUpStalled(1, _))),
            "{result:?}"
        );
        assert_eq!(
            asked.into_inner().expect("asked").len(),
            BAL_MAX_RETRIES_PER_BLOCK as usize
        );
        assert_eq!(
            table.get_score(empty.node_id()).await.expect("score"),
            MIN_SCORE_CRITICAL
        );
        assert_eq!(
            diagnostics.read().await.snap2_bals_unavailable,
            u64::from(BAL_MAX_RETRIES_PER_BLOCK)
        );
    }

    /// A peer that lacks the list SHALL not be asked for it again while another
    /// peer is untried, however far ahead it ranks: a stall discards the whole
    /// partial state, so it must mean no peer we can ask has the list, not that
    /// the best-ranked one answered without it three times in a row.
    #[tokio::test]
    async fn unavailable_peer_with_a_lead_does_not_take_every_retry() {
        let store = Store::new("memory", EngineType::InMemory).expect("in-memory store");
        let (from, headers, bals) = chain(&store, 3).await;
        let to = headers.last().expect("chain").clone();
        let (empty, full) = (node(1), node(2));
        let table = table_with(&[empty.clone(), full.clone()]).await;
        // A lead larger than the retry cap: charged once per attempt, the empty
        // peer would still out-rank the full one on every retry.
        let lead = i64::from(BAL_MAX_RETRIES_PER_BLOCK) + 2;
        for _ in 0..lead {
            table.record_success(empty.node_id()).expect("score");
        }
        let script = HashMap::from([
            (empty.node_id(), Answer::Unavailable),
            (full.node_id(), Answer::Serve),
        ]);
        let (_dir, flat) = flat_state();
        let (cursor, diagnostics) = (DownloadCursor::new(1), diagnostics());
        let asked = Mutex::new(Vec::new());

        tokio::time::timeout(
            Duration::from_secs(10),
            catch_up_via(
                &store,
                &table,
                |pending, exclude| {
                    scripted_request(&table, &script, &bals, &asked, pending, exclude)
                },
                &flat,
                &cursor,
                &from,
                &to,
                &diagnostics,
            ),
        )
        .await
        .expect("the catch-up must not loop")
        .expect("the serving peer must be asked before the catch-up stalls");

        assert_eq!(
            asked.into_inner().expect("asked"),
            vec![empty.node_id(), full.node_id()],
            "one miss, then the untried peer"
        );
        assert_eq!(
            table.get_score(empty.node_id()).await.expect("score"),
            lead - 1
        );
        assert_eq!(table.get_score(full.node_id()).await.expect("score"), 3);
        assert_eq!(diagnostics.read().await.snap2_blocks_replayed, 3);
    }
}
