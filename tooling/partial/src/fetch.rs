//! Verified fetches: every function here asks peers through [`Peers`] and returns only what
//! [`crate::verify`] accepted. A key that cannot be proven is an error, never a zero.

use std::collections::BTreeMap;
use std::sync::Arc;

use ethrex_common::H256;
use ethrex_common::types::BlockHeader;
use futures::stream::{self, StreamExt};
use tracing::debug;

use crate::peers::{NetError, Peers};
use crate::state::{HEADER_WINDOW, Key, Miss, Store};
use crate::verify::{
    self, AccountQuery, CodeQuery, Proven, ProvenAccount, SlotProof, TrustedHash, VerifiedBlock,
    VerifiedBody, VerifiedHeader, VerifiedReceipts, VerifyError,
};

/// Attempts per fetch before giving up. A lying peer is penalized and the next attempt may
/// land elsewhere.
pub const ATTEMPTS: usize = 3;

/// Proof requests in flight at once for one batch of keys.
pub const CONCURRENCY: usize = 16;

/// A fetch failed after its attempts.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// No peer answered.
    #[error(transparent)]
    Net(#[from] NetError),
    /// Every answer failed verification, or the data is unavailable from peers.
    #[error("unprovable: {0}")]
    Unprovable(#[from] VerifyError),
    /// The request cannot be served by this node.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
}

enum Failure {
    Net(NetError),
    Lie(crate::peers::PeerKey, VerifyError),
    Honest(VerifyError),
}

/// Runs `attempt` up to [`ATTEMPTS`] times. A reply that blames its peer penalizes it.
async fn retry<T, F, Fut>(peers: &dyn Peers, mut attempt: F) -> Result<T, FetchError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, Failure>>,
{
    let mut last: Option<FetchError> = None;
    for _ in 0..ATTEMPTS {
        match attempt().await {
            Ok(value) => return Ok(value),
            Err(Failure::Net(NetError::NoPeer)) => return Err(NetError::NoPeer.into()),
            Err(Failure::Net(NetError::Stopped)) => return Err(NetError::Stopped.into()),
            Err(Failure::Net(err)) => last = Some(err.into()),
            Err(Failure::Lie(peer, err)) => {
                debug!(%peer, %err, "peer sent bad data");
                peers.penalize(peer);
                last = Some(err.into());
            }
            Err(Failure::Honest(err)) => last = Some(err.into()),
        }
    }
    Err(last.unwrap_or(FetchError::Net(NetError::NoPeer)))
}

fn classify(peer: crate::peers::PeerKey, err: VerifyError) -> Failure {
    if err.blames_peer() {
        Failure::Lie(peer, err)
    } else {
        Failure::Honest(err)
    }
}

/// Fetches and verifies the header the head source vouched for.
pub async fn header(peers: &dyn Peers, trusted: TrustedHash) -> Result<VerifiedHeader, FetchError> {
    retry(peers, || async {
        let reply = peers
            .headers(trusted.hash(), 1)
            .await
            .map_err(Failure::Net)?;
        let header = reply
            .value
            .into_iter()
            .next()
            .ok_or_else(|| Failure::Net(NetError::Rejected("no header returned".into())))?;
        verify::header(trusted, header).map_err(|err| classify(reply.peer, err))
    })
    .await
}

/// Fetches and verifies the headers strictly between `anchor` and `target`, returning the
/// chain ascending from the anchor's child through `target`. One attempt: a break means a
/// reorg or a short reply, and the caller reanchors either way.
pub async fn chain(
    peers: &dyn Peers,
    anchor: &VerifiedHeader,
    target: VerifiedHeader,
) -> Result<Vec<VerifiedHeader>, FetchError> {
    let gap = target.number().saturating_sub(anchor.number());
    let between = if gap > 1 {
        let reply = peers
            .headers(target.header().parent_hash, gap.saturating_sub(1))
            .await?;
        reply.value
    } else {
        Vec::new()
    };
    Ok(verify::chain(anchor, target, between)?)
}

/// Fetches and verifies the block access lists of `chain`, in order. One attempt: a missing
/// or bad BAL sends the caller to reanchor.
pub async fn bals(
    peers: &dyn Peers,
    chain: Vec<VerifiedHeader>,
) -> Result<Vec<VerifiedBlock>, FetchError> {
    let headers: Vec<BlockHeader> = chain.iter().map(|h| h.header().clone()).collect();
    let reply = peers.block_access_lists(headers).await?;
    let mut bals = reply.value.into_iter();
    let mut blocks = Vec::with_capacity(chain.len());
    for header in chain {
        let bal = bals.next().flatten();
        match verify::bal(header, bal) {
            Ok(block) => blocks.push(block),
            Err(err) => {
                if err.blames_peer() {
                    peers.penalize(reply.peer);
                }
                return Err(err.into());
            }
        }
    }
    Ok(blocks)
}

/// Fetches and verifies ancestors of the store's oldest held header, oldest first, so the
/// header window reaches `number`. Fails if the window cannot reach it.
pub async fn ancestors(
    peers: &dyn Peers,
    store: &Store,
    number: u64,
) -> Result<Vec<VerifiedHeader>, FetchError> {
    let oldest = store
        .headers()
        .next()
        .cloned()
        .ok_or(FetchError::Unsupported("no headers"))?;
    let held = store.headers().count();
    let wanted = oldest.number().saturating_sub(number);
    let room = HEADER_WINDOW.saturating_sub(held);
    let count = wanted.min(u64::try_from(room).unwrap_or(u64::MAX));
    if count == 0 || number >= oldest.number() {
        return Err(FetchError::Unsupported(
            "block hashes older than the header window",
        ));
    }
    retry(peers, || async {
        let reply = peers
            .headers(oldest.header().parent_hash, count)
            .await
            .map_err(Failure::Net)?;
        verify::ancestors(&oldest, reply.value).map_err(|err| classify(reply.peer, err))
    })
    .await
}

/// Fetches and verifies `header`'s body.
pub async fn body(peers: &dyn Peers, header: VerifiedHeader) -> Result<VerifiedBody, FetchError> {
    retry(peers, || async {
        let reply = peers
            .bodies(vec![header.header().clone()])
            .await
            .map_err(Failure::Net)?;
        let body = reply
            .value
            .into_iter()
            .next()
            .ok_or_else(|| Failure::Net(NetError::Rejected("no body returned".into())))?;
        verify::body(header.clone(), body).map_err(|err| classify(reply.peer, err))
    })
    .await
}

/// Fetches and verifies the bodies of `headers`, in order. Peers cap replies by size, so a
/// short reply is progress, not a failure: the rest is asked for again.
pub async fn bodies(
    peers: &dyn Peers,
    headers: Vec<VerifiedHeader>,
) -> Result<Vec<VerifiedBody>, FetchError> {
    let mut verified = Vec::with_capacity(headers.len());
    let mut remaining = headers;
    let mut stalls = 0;
    while !remaining.is_empty() {
        let request: Vec<BlockHeader> = remaining.iter().map(|h| h.header().clone()).collect();
        let reply = match peers.bodies(request).await {
            Ok(reply) => reply,
            Err(NetError::NoPeer | NetError::Stopped) => {
                return Err(FetchError::Net(NetError::NoPeer));
            }
            Err(err) => {
                stalls += 1;
                if stalls >= ATTEMPTS {
                    return Err(err.into());
                }
                continue;
            }
        };
        if reply.value.is_empty() {
            stalls += 1;
            if stalls >= ATTEMPTS {
                return Err(FetchError::Net(NetError::Rejected(
                    "empty body reply".into(),
                )));
            }
            continue;
        }
        let mut progressed = 0;
        for body in reply.value {
            let Some(header) = remaining.first().cloned() else {
                break;
            };
            match verify::body(header, body) {
                Ok(body) => {
                    verified.push(body);
                    remaining.remove(0);
                    progressed += 1;
                }
                Err(err) => {
                    if err.blames_peer() {
                        peers.penalize(reply.peer);
                    }
                    break;
                }
            }
        }
        if progressed == 0 {
            stalls += 1;
            if stalls >= ATTEMPTS {
                return Err(FetchError::Net(NetError::Rejected(
                    "body replies verified nothing new".into(),
                )));
            }
        }
    }
    Ok(verified)
}

/// Fetches and verifies the receipts of `header`.
pub async fn receipts(
    peers: &dyn Peers,
    header: VerifiedHeader,
) -> Result<VerifiedReceipts, FetchError> {
    retry(peers, || async {
        let reply = peers
            .receipts(header.header().clone())
            .await
            .map_err(Failure::Net)?;
        verify::receipts(header.clone(), reply.value).map_err(|err| classify(reply.peer, err))
    })
    .await
}

/// Proves one account at `at`.
async fn account(
    peers: &dyn Peers,
    at: &VerifiedHeader,
    address: ethrex_common::Address,
) -> Result<ProvenAccount, FetchError> {
    let query = AccountQuery::new(at, address);
    retry(peers, || async {
        let reply = peers
            .account_range(query.request())
            .await
            .map_err(Failure::Net)?;
        query
            .verify(reply.value)
            .map_err(|err| classify(reply.peer, err))
    })
    .await
}

/// Proves one slot of a proven account.
async fn slot(
    peers: &dyn Peers,
    account: &ProvenAccount,
    slot: ethrex_common::U256,
) -> Result<Proven, FetchError> {
    match account.slot(slot) {
        SlotProof::Known(proven) => Ok(proven),
        SlotProof::Wanted(query) => {
            retry(peers, || async {
                let reply = peers
                    .storage_range(query.request())
                    .await
                    .map_err(Failure::Net)?;
                query
                    .verify(reply.value)
                    .map_err(|err| classify(reply.peer, err))
            })
            .await
        }
    }
}

/// Proves code by hash.
pub async fn code(peers: &dyn Peers, hash: H256) -> Result<Proven, FetchError> {
    let query = CodeQuery::new(hash);
    retry(peers, || async {
        let reply = peers
            .byte_codes(vec![query.request()])
            .await
            .map_err(Failure::Net)?;
        let code = reply.value.into_iter().next().flatten();
        query.verify(code).map_err(|err| classify(reply.peer, err))
    })
    .await
}

/// Proves `keys` at `at`, up to `concurrency` requests at once. Slots of one account share
/// its account proof. A key that cannot be proven never sinks keys it does not depend on:
/// a failed account proof fails every key of that account, a failed slot proof only itself.
pub async fn each(
    peers: &Arc<dyn Peers>,
    at: &VerifiedHeader,
    keys: impl IntoIterator<Item = Key>,
    concurrency: usize,
) -> Vec<(Key, Result<Proven, FetchError>)> {
    let mut by_account: BTreeMap<ethrex_common::Address, (bool, Vec<ethrex_common::U256>)> =
        BTreeMap::new();
    for key in keys {
        match key {
            Key::Account(address) => by_account.entry(address).or_default().0 = true,
            Key::Slot(address, slot) => by_account.entry(address).or_default().1.push(slot),
        }
    }
    let concurrency = concurrency.max(1);
    stream::iter(by_account)
        .map(|(address, (wants_account, slots))| {
            let peers = Arc::clone(peers);
            async move {
                let mut out = Vec::with_capacity(slots.len().saturating_add(1));
                let proven_account = match account(peers.as_ref(), at, address).await {
                    Ok(proven) => proven,
                    Err(err) => {
                        let message = err.to_string();
                        if wants_account {
                            out.push((Key::Account(address), Err(err)));
                        }
                        for slot in slots {
                            out.push((
                                Key::Slot(address, slot),
                                Err(FetchError::Unprovable(VerifyError::RootUnavailable {
                                    root: at.header().state_root,
                                })),
                            ));
                        }
                        debug!(%address, %message, "account proof failed");
                        return out;
                    }
                };
                if wants_account {
                    out.push((Key::Account(address), Ok(proven_account.proven())));
                }
                let slot_results: Vec<(Key, Result<Proven, FetchError>)> = stream::iter(slots)
                    .map(|s| {
                        let peers = Arc::clone(&peers);
                        let account = proven_account.clone();
                        async move {
                            let result = slot(peers.as_ref(), &account, s).await;
                            (Key::Slot(address, s), result)
                        }
                    })
                    .buffer_unordered(concurrency)
                    .collect()
                    .await;
                out.extend(slot_results);
                out
            }
        })
        .buffer_unordered(concurrency)
        .collect::<Vec<Vec<_>>>()
        .await
        .into_iter()
        .flatten()
        .collect()
}

/// What proving a miss yields.
#[derive(Clone, Debug)]
pub enum Fetched {
    /// A proven fact for the store.
    Proven(Proven),
    /// Verified ancestor headers for the header window, oldest first.
    Ancestors(Vec<VerifiedHeader>),
}

/// Proves the misses of a run at the store's anchor. Account and storage misses become
/// proofs, code misses byte-code fetches, and block-hash misses one ancestor fetch reaching
/// the oldest number asked for.
pub async fn misses(
    peers: &Arc<dyn Peers>,
    store: &Store,
    misses: impl IntoIterator<Item = Miss>,
) -> Result<Vec<Fetched>, FetchError> {
    let at = store.anchor().clone();
    let mut keys = Vec::new();
    let mut codes = Vec::new();
    let mut oldest_block: Option<u64> = None;
    for miss in misses {
        match miss {
            Miss::Account(address) => keys.push(Key::Account(address)),
            Miss::Storage(address, slot) => keys.push(Key::Slot(address, slot)),
            Miss::Code(hash) => codes.push(hash),
            Miss::BlockHash(number) => {
                oldest_block = Some(oldest_block.map_or(number, |n: u64| n.min(number)));
            }
        }
    }
    let mut fetched = Vec::new();
    let proofs = each(peers, &at, keys, CONCURRENCY).await;
    for (_, result) in proofs {
        fetched.push(Fetched::Proven(result?));
    }
    let codes: Vec<Result<Proven, FetchError>> = stream::iter(codes)
        .map(|hash| {
            let peers = Arc::clone(peers);
            async move { code(peers.as_ref(), hash).await }
        })
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;
    for result in codes {
        fetched.push(Fetched::Proven(result?));
    }
    if let Some(number) = oldest_block {
        fetched.push(Fetched::Ancestors(
            ancestors(peers.as_ref(), store, number).await?,
        ));
    }
    Ok(fetched)
}
