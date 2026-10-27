//! Predicate queries: discv5 lookups that look for nodes whose ENR passes a
//! consumer-supplied test, and report back what they found.
//!
//! discv5 has no index by ENR content, so a query is a random walk: a Kademlia
//! lookup toward a random target, whose only job is to make many nodes hand
//! over records. Every record a walk sees is run through the predicate. A match
//! counts only once its node has itself answered one of this query's FINDNODEs,
//! which keeps dead records out of the result; this is sigp/discv5's rule.
//! Matched nodes are asked before anyone else, so that check costs no extra
//! round of the walk.
//!
//! Convergence says nothing about whether more matches exist, so a walk that
//! runs out of nodes to ask is replaced by a fresh one toward a new random
//! target. The query ends once it has `want` answered matches, at its deadline,
//! or after [`MAX_IDLE_WALKS`] walks in a row that turned up no node it had not
//! already seen. The last rule is what stops a small network from being asked
//! the same questions until the deadline.
//!
//! Queries advance as responses arrive rather than on the crawl's timer, which
//! at steady state ticks every 10 s: far too slow for a duty a slot or two away.

use super::lookup::{IterativeLookup, LOOKUP_ALPHA, LookupId};
use crate::types::{Node, NodeRecord};
use bytes::Bytes;
use ethrex_common::H256;
use rustc_hash::{FxHashMap, FxHashSet};
use std::{
    collections::{VecDeque, hash_map::Entry},
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::oneshot;
use tracing::debug;

/// Predicate queries allowed to run at once, besides the background crawl.
/// Each keeps [`LOOKUP_ALPHA`] FINDNODEs in flight, so this bounds the UDP
/// traffic queries add. Lighthouse caps itself at four subnet queries.
pub const MAX_ACTIVE_QUERIES: usize = 4;
/// Longest a query may run, whatever the caller asks for. Matches sigp/discv5's
/// default query timeout.
pub const MAX_QUERY_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a query waits for a FINDNODE's first NODES packet before giving its
/// slot to another node. A first contact costs a WHOAREYOU round trip on top of
/// the request itself. A late answer is still used: the request stays
/// acceptable for as long as the server's pending-FINDNODE window allows.
pub const QUERY_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
/// How often running queries are checked for silent nodes and deadlines.
pub const QUERY_TICK_INTERVAL: Duration = Duration::from_millis(100);
/// Consecutive walks without a single node the query had not already seen,
/// after which the query gives up as [`QueryOutcome::Exhausted`].
pub const MAX_IDLE_WALKS: usize = 3;

/// The consumer's test for a record it wants. Runs inside the discovery
/// server's message loop on every record a query sees, so keep it cheap, as
/// [`PeerFilter::accepts`](crate::peer_filter::PeerFilter::accepts) is.
pub type NodePredicate = Box<dyn Fn(&NodeRecord) -> bool + Send>;

/// Why a query stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryOutcome {
    /// It found as many answered matches as were wanted.
    Satisfied,
    /// It ran out of nodes to ask first.
    Exhausted,
    /// It reached its deadline first.
    TimedOut,
}

/// What a predicate query found, and enough about how it went to explain a
/// short result.
#[derive(Debug, Clone)]
pub struct FindNodesResult {
    /// Matching records whose node answered one of the query's FINDNODEs, in
    /// the order they were confirmed. Can exceed `want` by a few when several
    /// answers land together.
    pub records: Vec<NodeRecord>,
    pub outcome: QueryOutcome,
    /// FINDNODE requests the query sent.
    pub findnodes_sent: usize,
    /// Distinct nodes whose records the query saw, matching or not.
    pub nodes_seen: usize,
    /// Random walks the query started.
    pub walks: usize,
}

/// Why a query could not run at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum FindNodesError {
    #[error("discovery is not running")]
    NotRunning,
    #[error("discv5 is disabled")]
    Discv5Disabled,
    #[error("{MAX_ACTIVE_QUERIES} queries are already running")]
    TooManyQueries,
}

/// A predicate query as it crosses into the discovery server. Built only by
/// [`DiscoveryHandle::find_nodes`](super::DiscoveryHandle::find_nodes).
///
/// The answer travels back on `reply` rather than as a request's return value:
/// a request handler answers when it returns, and a query is only answered
/// several NODES messages later.
pub struct FindNodesRequest {
    pub(crate) predicate: NodePredicate,
    pub(crate) want: usize,
    pub(crate) timeout: Duration,
    pub(crate) reply: oneshot::Sender<Result<FindNodesResult, FindNodesError>>,
}

/// What the server should do next for a query.
#[derive(Debug)]
pub(crate) enum QueryStep {
    /// Send a FINDNODE toward `target` to each of `nodes`, then report each
    /// request through [`NodeQuery::record_sent`].
    Send {
        target: H256,
        nodes: Vec<(H256, Node)>,
    },
    /// Requests are in flight; nothing to do until one answers or times out.
    Wait,
    /// The walk is spent: start another through [`NodeQuery::start_walk`].
    Walk,
    /// Answer the caller through [`NodeQuery::finish`].
    Finish(QueryOutcome),
    /// The caller stopped waiting; drop the query.
    Abandoned,
}

/// One running predicate query.
pub(crate) struct NodeQuery {
    pub(crate) id: LookupId,
    predicate: NodePredicate,
    want: usize,
    deadline: Instant,
    reply: Option<oneshot::Sender<Result<FindNodesResult, FindNodesError>>>,
    /// The current walk, `None` before the first one and between two.
    walk: Option<IterativeLookup>,
    target: H256,
    walks: usize,
    /// Whether the current walk has seen a node no earlier walk had.
    walk_found_new: bool,
    idle_walks: usize,
    /// Every node whose record this query has seen, across walks.
    seen: FxHashSet<H256>,
    /// Nodes this query has sent a FINDNODE to.
    queried: FxHashSet<H256>,
    /// Nodes that answered one of this query's FINDNODEs.
    answered: FxHashSet<H256>,
    /// Matching records whose node has not answered yet.
    candidates: FxHashMap<H256, (Node, NodeRecord)>,
    /// Candidates not asked yet, in the order they matched.
    to_verify: VecDeque<H256>,
    /// Matching records whose node answered: the result.
    matched: Vec<NodeRecord>,
    matched_ids: FxHashSet<H256>,
    /// Requests still waiting for their first NODES packet, by when sent.
    in_flight: FxHashMap<(H256, Bytes), Instant>,
    findnodes_sent: usize,
}

// Hand-written because the predicate and the reply channel are not `Debug`.
impl std::fmt::Debug for NodeQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeQuery")
            .field("id", &self.id)
            .field("want", &self.want)
            .field("deadline", &self.deadline)
            .field("target", &self.target)
            .field("walks", &self.walks)
            .field("matched", &self.matched.len())
            .field("candidates", &self.candidates.len())
            .field("in_flight", &self.in_flight.len())
            .field("findnodes_sent", &self.findnodes_sent)
            .finish_non_exhaustive()
    }
}

impl NodeQuery {
    /// A query with no walk yet: its first [`Self::poll`] asks for one.
    pub(crate) fn new(id: LookupId, request: FindNodesRequest, now: Instant) -> Self {
        let FindNodesRequest {
            predicate,
            want,
            timeout,
            reply,
        } = request;
        Self {
            id,
            predicate,
            want,
            deadline: now + timeout.min(MAX_QUERY_TIMEOUT),
            reply: Some(reply),
            walk: None,
            target: H256::zero(),
            walks: 0,
            walk_found_new: false,
            idle_walks: 0,
            seen: Default::default(),
            queried: Default::default(),
            answered: Default::default(),
            candidates: Default::default(),
            to_verify: Default::default(),
            matched: Vec::new(),
            matched_ids: Default::default(),
            in_flight: Default::default(),
            findnodes_sent: 0,
        }
    }

    /// Decide what happens next. Call again after acting on a
    /// [`QueryStep::Send`] or [`QueryStep::Walk`].
    pub(crate) fn poll(&mut self, now: Instant) -> QueryStep {
        if self.reply.as_ref().is_none_or(|reply| reply.is_closed()) {
            return QueryStep::Abandoned;
        }
        if self.matched.len() >= self.want {
            return QueryStep::Finish(QueryOutcome::Satisfied);
        }
        if now >= self.deadline {
            return QueryStep::Finish(QueryOutcome::TimedOut);
        }
        let nodes = self.next_requests();
        if !nodes.is_empty() {
            return QueryStep::Send {
                target: self.target,
                nodes,
            };
        }
        if !self.in_flight.is_empty() {
            return QueryStep::Wait;
        }
        // Nothing left to ask and nothing pending: this walk is over.
        if self.walk.take().is_some() {
            if self.walk_found_new {
                self.idle_walks = 0;
            } else {
                self.idle_walks += 1;
            }
        }
        if self.idle_walks >= MAX_IDLE_WALKS {
            return QueryStep::Finish(QueryOutcome::Exhausted);
        }
        QueryStep::Walk
    }

    /// Start a walk toward `target` from `seeds`, contacts whose records we
    /// already hold, so the predicate runs on the seeds too.
    ///
    /// `could_dial` says whether the consumer could still want a node; one it
    /// cannot is never reported as a match.
    pub(crate) fn start_walk(
        &mut self,
        target: H256,
        seeds: &[(H256, Node, NodeRecord)],
        could_dial: impl Fn(&H256) -> bool,
    ) {
        self.target = target;
        self.walks += 1;
        self.walk_found_new = false;
        self.walk = Some(IterativeLookup::new(
            target,
            seeds
                .iter()
                .map(|(node_id, node, _)| (*node_id, node.clone()))
                .collect(),
        ));
        for (node_id, node, record) in seeds {
            self.observe(*node_id, node, record, &could_dial);
        }
    }

    /// Note a FINDNODE sent to `node_id` on this query's behalf.
    pub(crate) fn record_sent(&mut self, node_id: H256, req_id: Bytes, now: Instant) {
        self.in_flight.insert((node_id, req_id), now);
        self.findnodes_sent += 1;
    }

    /// Take in one NODES packet answering this query's request `req_id` to
    /// `sender`. `records` are the ones that verified, as `(id, node, record)`.
    ///
    /// A response split across several packets frees its request's slot on
    /// the first; every packet is still matched and fed to the walk.
    pub(crate) fn on_nodes(
        &mut self,
        sender: H256,
        req_id: &Bytes,
        records: &[(H256, Node, NodeRecord)],
        could_dial: impl Fn(&H256) -> bool,
    ) {
        self.in_flight.remove(&(sender, req_id.clone()));
        self.answered.insert(sender);
        if let Some((_, record)) = self.candidates.remove(&sender) {
            self.accept(sender, record);
        }
        for (node_id, node, record) in records {
            self.observe(*node_id, node, record, &could_dial);
        }
        if let Some(walk) = &mut self.walk {
            walk.feed_results(
                records
                    .iter()
                    .map(|(node_id, node, _)| (*node_id, node.clone()))
                    .collect(),
            );
        }
    }

    /// Give up on requests that have not answered within
    /// [`QUERY_REQUEST_TIMEOUT`], freeing their slots for other nodes.
    pub(crate) fn expire_requests(&mut self, now: Instant) {
        self.in_flight
            .retain(|_, sent_at| now.saturating_duration_since(*sent_at) < QUERY_REQUEST_TIMEOUT);
    }

    /// Answer the caller.
    pub(crate) fn finish(mut self, outcome: QueryOutcome) {
        debug!(
            protocol = "discv5",
            query = self.id.0,
            ?outcome,
            matched = self.matched.len(),
            want = self.want,
            findnodes_sent = self.findnodes_sent,
            nodes_seen = self.seen.len(),
            walks = self.walks,
            "Predicate query finished"
        );
        let result = FindNodesResult {
            records: std::mem::take(&mut self.matched),
            outcome,
            findnodes_sent: self.findnodes_sent,
            nodes_seen: self.seen.len(),
            walks: self.walks,
        };
        if let Some(reply) = self.reply.take() {
            // The caller may have stopped waiting; nothing to do about that.
            let _ = reply.send(Ok(result));
        }
    }

    /// Fill the free request slots: matched nodes first, since they count only
    /// once they answer, then the walk's closest unasked nodes.
    fn next_requests(&mut self) -> Vec<(H256, Node)> {
        let mut slots = LOOKUP_ALPHA.saturating_sub(self.in_flight.len());
        let mut nodes = Vec::with_capacity(slots);
        while slots > 0
            && let Some(node_id) = self.to_verify.pop_front()
        {
            // Gone if it answered a walk request since it matched.
            let Some((node, _)) = self.candidates.get(&node_id) else {
                continue;
            };
            if !self.queried.insert(node_id) {
                continue;
            }
            if let Some(walk) = &mut self.walk {
                walk.mark_queried(&node_id);
            }
            nodes.push((node_id, node.clone()));
            slots -= 1;
        }
        while slots > 0
            && let Some(walk) = &mut self.walk
            && let Some((node_id, node)) = walk.next_to_query(1).pop()
        {
            self.queried.insert(node_id);
            nodes.push((node_id, node));
            slots -= 1;
        }
        nodes
    }

    fn observe(
        &mut self,
        node_id: H256,
        node: &Node,
        record: &NodeRecord,
        could_dial: &impl Fn(&H256) -> bool,
    ) {
        if self.seen.insert(node_id) {
            self.walk_found_new = true;
        }
        if self.matched_ids.contains(&node_id) || !could_dial(&node_id) || !(self.predicate)(record)
        {
            return;
        }
        if self.answered.contains(&node_id) {
            self.accept(node_id, record.clone());
            return;
        }
        match self.candidates.entry(node_id) {
            Entry::Occupied(mut entry) => {
                if record.seq > entry.get().1.seq {
                    entry.insert((node.clone(), record.clone()));
                }
            }
            Entry::Vacant(entry) => {
                entry.insert((node.clone(), record.clone()));
                // A node already asked in vain gets no second request just for
                // matching; a later walk may still reach it.
                if !self.queried.contains(&node_id) {
                    self.to_verify.push_back(node_id);
                }
            }
        }
    }

    fn accept(&mut self, node_id: H256, record: NodeRecord) {
        if self.matched_ids.insert(node_id) {
            self.matched.push(record);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secp256k1::SecretKey;
    use std::net::{IpAddr, Ipv4Addr};

    const MARKER: &[u8] = b"marker";

    /// A node with a valid signed record, marked when `marked`.
    fn node(port: u16, marked: bool) -> (H256, Node, NodeRecord) {
        let signer = SecretKey::new(&mut rand::rngs::OsRng);
        let public_key = crate::utils::public_key_from_signing_key(&signer);
        let node = Node::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port, port, public_key);
        let mut record = NodeRecord::from_node(&node, 1, &signer).unwrap();
        if marked {
            record
                .edit(&signer, |pairs| {
                    pairs.set_extra(MARKER, vec![1u8]);
                })
                .unwrap();
        }
        (node.node_id(), node, record)
    }

    fn query(want: usize, timeout: Duration) -> (NodeQuery, oneshot::Receiver<QueryReply>) {
        let (reply, rx) = oneshot::channel();
        let request = FindNodesRequest {
            predicate: Box::new(|record: &NodeRecord| record.pairs().extra(MARKER).is_some()),
            want,
            timeout,
            reply,
        };
        (NodeQuery::new(LookupId(1), request, Instant::now()), rx)
    }

    type QueryReply = Result<FindNodesResult, FindNodesError>;

    fn anyone(_: &H256) -> bool {
        true
    }

    fn req(n: u8) -> Bytes {
        Bytes::from(vec![n])
    }

    /// Polls once and returns the nodes it asked to send to, recording each
    /// as sent under request id `[i]` for the i-th node.
    fn send_all(query: &mut NodeQuery) -> Vec<H256> {
        let QueryStep::Send { nodes, .. } = query.poll(Instant::now()) else {
            panic!("expected the query to send");
        };
        nodes
            .into_iter()
            .enumerate()
            .map(|(i, (node_id, _))| {
                query.record_sent(node_id, req(i as u8), Instant::now());
                node_id
            })
            .collect()
    }

    fn finish(mut query: NodeQuery, mut rx: oneshot::Receiver<QueryReply>) -> FindNodesResult {
        let QueryStep::Finish(outcome) = query.poll(Instant::now()) else {
            panic!("expected the query to finish");
        };
        query.finish(outcome);
        rx.try_recv().expect("query must answer").unwrap()
    }

    #[test]
    fn a_match_counts_only_once_its_node_answers() {
        let (mut query, rx) = query(1, Duration::from_secs(5));
        let marked = node(1, true);
        assert!(matches!(query.poll(Instant::now()), QueryStep::Walk));
        query.start_walk(H256::zero(), std::slice::from_ref(&marked), anyone);

        // The match has been seen, yet the query keeps going until it answers.
        assert_eq!(send_all(&mut query), vec![marked.0]);
        assert!(matches!(query.poll(Instant::now()), QueryStep::Wait));

        query.on_nodes(marked.0, &req(0), &[], anyone);
        let result = finish(query, rx);
        assert_eq!(result.outcome, QueryOutcome::Satisfied);
        assert_eq!(result.records, vec![marked.2]);
        assert_eq!(result.findnodes_sent, 1);
    }

    #[test]
    fn a_match_learned_from_a_response_is_asked_before_the_walk() {
        let (mut query, _rx) = query(1, Duration::from_secs(5));
        let seeds: Vec<_> = (1..=3).map(|port| node(port, false)).collect();
        let seed_ids: Vec<_> = seeds.iter().map(|(id, _, _)| *id).collect();
        let extra = node(4, false);
        let marked = node(5, true);
        let _ = query.poll(Instant::now());
        query.start_walk(H256::zero(), &seeds, anyone);
        let asked = send_all(&mut query);
        let mut sorted = asked.clone();
        sorted.sort();
        let mut expected = seed_ids;
        expected.sort();
        assert_eq!(sorted, expected);

        // One answer frees one slot, and the match it brought takes it.
        query.on_nodes(asked[0], &req(0), &[extra, marked.clone()], anyone);
        let QueryStep::Send { nodes, .. } = query.poll(Instant::now()) else {
            panic!("expected the query to send");
        };
        assert_eq!(
            nodes.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![marked.0]
        );
    }

    #[test]
    fn a_node_the_consumer_wrote_off_never_matches() {
        let (mut query, _rx) = query(1, Duration::from_secs(5));
        let marked = node(1, true);
        let _ = query.poll(Instant::now());
        query.start_walk(H256::zero(), std::slice::from_ref(&marked), |_| false);
        send_all(&mut query);
        query.on_nodes(marked.0, &req(0), &[], |_| false);

        assert!(query.matched.is_empty());
    }

    #[test]
    fn the_deadline_returns_what_was_confirmed() {
        let (mut query, rx) = query(2, Duration::ZERO);
        // Confirm one match by hand, then let the zero deadline end it.
        let marked = node(1, true);
        query.observe(marked.0, &marked.1, &marked.2, &anyone);
        query.on_nodes(marked.0, &req(0), &[], anyone);

        let result = finish(query, rx);
        assert_eq!(result.outcome, QueryOutcome::TimedOut);
        assert_eq!(result.records, vec![marked.2]);
    }

    #[test]
    fn a_silent_node_gives_up_its_slot() {
        let (mut query, _rx) = query(1, Duration::from_secs(5));
        let seeds: Vec<_> = (1..=4).map(|port| node(port, false)).collect();
        let _ = query.poll(Instant::now());
        query.start_walk(H256::zero(), &seeds, anyone);
        assert_eq!(send_all(&mut query).len(), LOOKUP_ALPHA);
        assert!(matches!(query.poll(Instant::now()), QueryStep::Wait));

        query.expire_requests(Instant::now() + QUERY_REQUEST_TIMEOUT);
        let QueryStep::Send { nodes, .. } = query.poll(Instant::now()) else {
            panic!("expired requests must free their slots");
        };
        assert_eq!(nodes.len(), 1, "only the one unasked seed is left");
    }

    #[test]
    fn walks_that_find_nothing_new_exhaust_the_query() {
        let (mut query, rx) = query(1, Duration::from_secs(5));
        let seed = node(1, false);
        let mut walks = 0;
        loop {
            match query.poll(Instant::now()) {
                QueryStep::Walk => {
                    walks += 1;
                    query.start_walk(H256::zero(), std::slice::from_ref(&seed), anyone);
                }
                QueryStep::Send { nodes, .. } => {
                    for (node_id, _) in nodes {
                        query.record_sent(node_id, req(0), Instant::now());
                        query.on_nodes(node_id, &req(0), &[], anyone);
                    }
                }
                QueryStep::Finish(outcome) => {
                    assert_eq!(outcome, QueryOutcome::Exhausted);
                    query.finish(outcome);
                    break;
                }
                step => panic!("unexpected step {step:?}"),
            }
        }
        // The first walk is new by definition; then MAX_IDLE_WALKS idle ones.
        assert_eq!(walks, 1 + MAX_IDLE_WALKS);
        let mut rx = rx;
        let result = rx.try_recv().expect("query must answer").unwrap();
        assert_eq!(result.outcome, QueryOutcome::Exhausted);
        assert_eq!(result.walks, walks);
        assert_eq!(result.nodes_seen, 1);
    }

    #[test]
    fn a_caller_that_stops_waiting_abandons_the_query() {
        let (mut query, rx) = query(1, Duration::from_secs(5));
        drop(rx);
        assert!(matches!(query.poll(Instant::now()), QueryStep::Abandoned));
    }
}
