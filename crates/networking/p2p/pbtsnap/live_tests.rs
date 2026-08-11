//! `pbtsnap/1` across a real RLPx connection between two in-process nodes.
//!
//! Everything else in this crate tests the protocol in pieces: the codecs
//! round-trip in memory, the server is called as a function, and the
//! negotiation rules are checked through extracted helpers. The three seams
//! that only exist *between* those pieces — the capability arm in
//! `exchange_hello_messages`, the request arm in `handle_incoming_message`, and
//! the response fan-out that resolves an in-flight `outgoing_request` — were
//! compiled but never executed by any test. The third one was wrong: a correct
//! `PbtLeafRange` decoded, fell through to `MessageNotHandled` (which is not
//! fatal), and the requester's oneshot was never fired, so `outgoing_request`
//! reported a `Timeout` against a peer that had answered correctly and on time.
//!
//! So these tests bind real sockets, run the real handshake, and assert on what
//! the client can conclude from the bytes it got back.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use ethereum_types::H256;
use ethrex_binary_trie::trie::{RangeProofError, verify_range};
use ethrex_blockchain::Blockchain;
use ethrex_common::types::{BlockHeader, ChainConfig, ForkId, Genesis, GenesisAccount};
use ethrex_common::{Address, U256};
use ethrex_storage::{EngineType, Store};
use secp256k1::{PublicKey, SECP256K1, SecretKey};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_util::task::TaskTracker;

use crate::network::P2PContext;
use crate::peer_table::{PeerTable, PeerTableServer, PeerTableServerProtocol};
use crate::rlpx::connection::server::PeerConnection;
use crate::rlpx::message::Message;
use crate::rlpx::p2p::Capability;
use crate::rlpx::pbtsnap::{GetPbtLeafRange, PbtLeafRange};
use crate::rlpx::utils::decompress_pubkey;
use crate::snap::constants::MAX_RESPONSE_BYTES;
use crate::types::{NetworkConfig, Node};

/// The pivot header's timestamp. Anything past the schedule works; this is well
/// clear of it so the header is unambiguously post-flip.
const PIVOT_TIMESTAMP: u64 = 1_000;

/// When the binary-tree commitment activates: after genesis, before the pivot.
const SCHEDULE_TIMESTAMP: u64 = 500;

/// Long enough that a slow CI box does not fail a handshake that would have
/// completed, short enough that a genuinely stuck connection fails the test
/// rather than the suite's own timeout.
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(20);

/// A test node: its store, its identity, and the context its connections run
/// in. Held together because the tracker and the peer table have to outlive the
/// connection actors that reference them.
struct TestNode {
    context: P2PContext,
    store: Store,
    node: Node,
    table: PeerTable,
    _tracker: TaskTracker,
}

fn genesis_account(nonce: u64, balance: u64, storage: &[(u64, u64)]) -> GenesisAccount {
    GenesisAccount {
        code: Bytes::new(),
        storage: storage
            .iter()
            .map(|(slot, value)| (U256::from(*slot), U256::from(*value)))
            .collect(),
        balance: U256::from(balance),
        nonce,
    }
}

/// The genesis both nodes run. Identical on both sides by construction:
/// `validate_status` compares genesis hashes and fork ids, so a handshake
/// between two differently-configured stores never reaches `Established` and
/// nothing below it could be tested at all.
///
/// The schedule is in the *future*, which is the shape every real chain has and
/// the only one usable here. Activation at or before genesis makes
/// `Genesis::compute_state_root` commit the binary root in the genesis header,
/// which changes the genesis hash — so two nodes differing only in whether they
/// schedule would fail `validate_status` on the genesis check and never reach
/// `Established` at all.
fn shared_genesis(scheduled: bool) -> Genesis {
    let mut alloc = BTreeMap::new();
    alloc.insert(
        Address::repeat_byte(0x11),
        genesis_account(1, 1_000, &[(1, 2), (900, 3)]),
    );
    alloc.insert(
        Address::repeat_byte(0x22),
        genesis_account(2, 500, &[(5, 7)]),
    );
    alloc.insert(Address::repeat_byte(0x33), genesis_account(0, 1, &[]));
    Genesis {
        config: ChainConfig {
            chain_id: 3151908,
            binary_tree_time: scheduled.then_some(SCHEDULE_TIMESTAMP),
            ..Default::default()
        },
        alloc,
        gas_limit: 0x1c9c380,
        timestamp: 0,
        ..Default::default()
    }
}

/// A store on [`shared_genesis`] whose canonical head is a post-activation
/// header committing the genesis binary root — the minimum shape a node needs
/// to be able to serve a `pbtsnap` range, and the same shape the unit-level
/// server fixture builds.
///
/// The unscheduled variant keeps the same *height* deliberately, so the two
/// nodes' fork ids are computed over the same latest header and the handshake
/// cannot fail for a reason unrelated to what is being tested.
async fn served_store(scheduled: bool) -> (Store, BlockHeader) {
    let mut store = Store::new("", EngineType::InMemory).expect("in-memory store");
    let genesis = shared_genesis(scheduled);
    let genesis_hash = genesis.get_block().hash();
    store
        .add_initial_state(genesis)
        .await
        .expect("genesis lands");

    // An unscheduled chain does no binary-trie work at all, so there is no root
    // to commit and the pivot header is a placeholder that only keeps the two
    // nodes at the same height.
    let root = store
        .get_binary_trie_root(genesis_hash)
        .expect("root read")
        .unwrap_or_default();

    let header = BlockHeader {
        number: 1,
        parent_hash: genesis_hash,
        timestamp: PIVOT_TIMESTAMP,
        state_root: root,
        ..Default::default()
    };
    let hash = header.hash();
    store
        .add_block_header(hash, header.clone())
        .await
        .expect("pivot header");
    store
        .forkchoice_update(vec![(1, hash)], 1, hash, None, None)
        .await
        .expect("fcu");
    // The half that makes the state *servable*: without it the root resolves to
    // no canonical block and every request is refused.
    if scheduled {
        store.set_binary_trie_root(hash, root).expect("record root");
    }
    (store, header)
}

/// Build a node listening on an ephemeral port, and return it plus the
/// join handle of its accept loop.
///
/// A hand-rolled accept loop rather than `serve_p2p_requests`: that one binds
/// the port itself from `NetworkConfig`, which leaves a test with no way to ask
/// for an unused one. Binding first and reading the port back is the only
/// collision-free way to run two nodes on one machine.
async fn spawn_node(
    secret: SecretKey,
    store: Store,
    pool: Arc<rayon::ThreadPool>,
    listen: bool,
) -> (TestNode, Option<tokio::task::JoinHandle<()>>) {
    let public_key = decompress_pubkey(&PublicKey::from_secret_key(SECP256K1, &secret));
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);

    let (listener, port) = if listen {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind an ephemeral port");
        let port = listener.local_addr().expect("local addr").port();
        (Some(listener), port)
    } else {
        (None, 0)
    };

    let node = Node::new(ip, port, port, public_key);
    let tracker = TaskTracker::new();
    // `target_peers` must be non-zero: `initialize_connection` disconnects with
    // `TooManyPeers` the moment the table says the target is reached.
    let table = PeerTableServer::spawn(node.node_id(), 8, store.clone());
    let blockchain = Arc::new(Blockchain::default_with_store_and_pool(store.clone(), pool));
    let context = P2PContext::new(
        node.clone(),
        NetworkConfig::from_node(&node),
        tracker.clone(),
        secret,
        table.clone(),
        store.clone(),
        blockchain,
        "ethrex/test".to_string(),
        None,
        60_000,
        10.0,
    )
    .expect("p2p context");

    let accept = listener.map(|listener| {
        let context = context.clone();
        tokio::spawn(async move {
            // One connection is all any test here makes. The `PeerConnection`
            // is deliberately leaked into the task's scope rather than dropped:
            // it holds the only handle to the receiving actor.
            let (stream, peer_addr) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(_) => return,
            };
            let permit = Arc::new(Semaphore::new(1))
                .try_acquire_owned()
                .expect("a fresh semaphore has a permit");
            let connection = PeerConnection::spawn_as_receiver(context, peer_addr, stream, permit);
            // Park forever holding the handle; the test drops the whole task.
            std::future::pending::<()>().await;
            drop(connection);
        })
    });

    (
        TestNode {
            context,
            store,
            node,
            table,
            _tracker: tracker,
        },
        accept,
    )
}

/// Dial `server` from `client` and wait until the connection is `Established`,
/// which is exactly when it registers itself in the dialer's peer table.
///
/// Polling the table rather than sleeping a fixed interval: `outgoing_request`
/// issued before establishment is dropped on the floor by the actor and only
/// surfaces as a five-second timeout, which would be indistinguishable from the
/// response-routing bug these tests exist to catch.
async fn connect(client: &TestNode, server: &TestNode) -> (PeerConnection, Vec<Capability>) {
    let dialer = PeerConnection::spawn_as_initiator(client.context.clone(), &server.node);
    let server_id = server.node.node_id();
    let deadline = tokio::time::Instant::now() + HANDSHAKE_DEADLINE;

    loop {
        let peers = client
            .table
            .get_peers_with_capabilities()
            .await
            .expect("peer table responds");
        if let Some((_, connection, capabilities)) =
            peers.into_iter().find(|(id, _, _)| *id == server_id)
        {
            drop(dialer);
            return (connection, capabilities);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the handshake did not complete within {HANDSHAKE_DEADLINE:?}",
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn to_leaves(response: &PbtLeafRange) -> Vec<(Vec<u8>, [u8; 32])> {
    response
        .leaves
        .iter()
        .map(|leaf| (leaf.key.to_vec(), leaf.value.0))
        .collect()
}

fn to_proof(proof: &[Bytes]) -> Vec<Vec<u8>> {
    proof.iter().map(|node| node.to_vec()).collect()
}

fn whole_keyspace(root: H256, id: u64) -> Message {
    Message::GetPbtLeafRange(GetPbtLeafRange {
        id,
        root_hash: root,
        origin: Bytes::new(),
        limit: Bytes::new(),
        response_bytes: MAX_RESPONSE_BYTES,
    })
}

/// Two distinct keys, so the two nodes have distinct identities and the peer
/// table on each side can tell them apart.
fn keys() -> (SecretKey, SecretKey) {
    (
        SecretKey::from_slice(&[0x11; 32]).expect("valid key"),
        SecretKey::from_slice(&[0x22; 32]).expect("valid key"),
    )
}

/// One rayon pool shared by both nodes' `Blockchain`s. A fresh pool per
/// `Blockchain` is seventeen threads, and two of them in one test process is
/// enough to matter on a constrained CI box.
fn shared_pool() -> Arc<rayon::ThreadPool> {
    Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .expect("rayon pool"),
    )
}

/// The headline: a request goes out over a real socket, the server reads its
/// own binary trie, and the client verifies the answer against the pivot root
/// it asked about.
///
/// Every assertion here is on what the *client* holds after decoding wire
/// bytes, not on what the server computed.
#[tokio::test]
async fn a_pbtsnap_leaf_range_crosses_a_live_connection() {
    let (server_key, client_key) = keys();
    let pool = shared_pool();
    let (server_store, pivot) = served_store(true).await;
    let (client_store, _) = served_store(true).await;

    let (server, _accept) = spawn_node(server_key, server_store.clone(), pool.clone(), true).await;
    let (client, _) = spawn_node(client_key, client_store, pool, false).await;

    let (mut connection, capabilities) = connect(&client, &server).await;

    // The peer's *advertised* list, carried in its Hello and recorded in the
    // dialer's peer table. Advertisement rather than negotiation on purpose:
    // `Established::negotiated_pbtsnap_capability` is written and never read by
    // anything — the same is true of `negotiated_snap_capability`, so this is
    // the codebase's existing shape rather than a pbtsnap omission — and what
    // peer selection actually filters on is this list.
    assert!(
        capabilities.contains(&Capability::pbtsnap(1)),
        "a scheduled peer must advertise pbtsnap/1, got {capabilities:?}",
    );
    // The call a sync driver makes, which is the only thing that turns the
    // advertisement into a usable peer.
    assert!(
        client
            .table
            .get_best_peer(vec![Capability::pbtsnap(1)])
            .await
            .expect("peer table responds")
            .is_some(),
        "a pbtsnap peer must be selectable by capability",
    );

    let root = pivot.state_root;
    let response = connection
        .outgoing_request(whole_keyspace(root, 7), Duration::from_secs(15))
        .await
        .expect("the peer must answer a pbtsnap request it can serve");
    let Message::PbtLeafRange(response) = response else {
        panic!("expected a PbtLeafRange, got {response}");
    };

    assert_eq!(response.id, 7, "the request id must survive the round trip");
    assert!(!response.leaves.is_empty(), "the server holds state");

    let verified = verify_range(
        root,
        &[],
        &to_leaves(&response),
        &to_proof(&response.left_proof),
        &to_proof(&response.right_proof),
    )
    .expect("an honestly served range must verify against the pivot root");
    assert!(
        !verified.has_more,
        "the whole keyspace was requested and it fits in one response"
    );

    // The bytes that crossed the wire carry the server's own leaf set, not a
    // truncation or a re-ordering the codec happened to survive.
    let served = server
        .store
        .binary_leaf_range_proof(root, &[], &[], 100_000)
        .expect("the server holds its own root")
        .leaves;
    assert_eq!(
        to_leaves(&response),
        served,
        "the decoded leaves must be exactly what the server holds",
    );
}

/// A root the peer cannot serve comes back as an empty range, and the client
/// must reject it rather than conclude the state is empty.
///
/// This is the refusal path of the dispatch arm — the common one in production,
/// since a pivot ages out of a peer's layer window — and it is the case where a
/// silent acceptance would be worst: an empty range that verified would let a
/// client install an empty state under a real root.
#[tokio::test]
async fn an_unservable_root_answers_an_empty_range_the_client_refuses() {
    let (server_key, client_key) = keys();
    let pool = shared_pool();
    let (server_store, _) = served_store(true).await;
    let (client_store, _) = served_store(true).await;

    let (server, _accept) = spawn_node(server_key, server_store, pool.clone(), true).await;
    let (client, _) = spawn_node(client_key, client_store, pool, false).await;
    let (mut connection, _) = connect(&client, &server).await;

    let unknown = H256::repeat_byte(0x9c);
    let response = connection
        .outgoing_request(whole_keyspace(unknown, 11), Duration::from_secs(15))
        .await
        .expect("a refusal is still a reply: the peer must not go silent");
    let Message::PbtLeafRange(response) = response else {
        panic!("expected a PbtLeafRange, got {response}");
    };

    assert_eq!(response.id, 11);
    assert!(response.leaves.is_empty(), "nothing can be served");

    let error = verify_range(
        unknown,
        &[],
        &to_leaves(&response),
        &to_proof(&response.left_proof),
        &to_proof(&response.right_proof),
    )
    .expect_err("a forged emptiness must not verify");
    // Not `matches!(_, _)`: naming the variant is the point. An empty response
    // with an empty left walk is rejected because a non-empty tree's walk is
    // never empty, which is what makes emptiness provable rather than claimed.
    assert!(matches!(error, RangeProofError::Proof(_)), "got {error:?}");
}

/// The client must not accept a range that verifies against *some* tree when it
/// asked about a different one.
///
/// The lie available to a peer that does not hold the pivot is to answer with a
/// range from a tree it does hold. Here the peer serves honestly from its own
/// root and the client checks the reply against the root it actually wanted;
/// the bytes are a real server's real response, and they must still be refused.
#[tokio::test]
async fn a_range_served_from_another_tree_does_not_verify_against_the_pivot() {
    let (server_key, client_key) = keys();
    let pool = shared_pool();
    let (server_store, pivot) = served_store(true).await;
    let (client_store, _) = served_store(true).await;

    let (server, _accept) = spawn_node(server_key, server_store, pool.clone(), true).await;
    let (client, _) = spawn_node(client_key, client_store, pool, false).await;
    let (mut connection, _) = connect(&client, &server).await;

    let response = connection
        .outgoing_request(whole_keyspace(pivot.state_root, 3), Duration::from_secs(15))
        .await
        .expect("serve");
    let Message::PbtLeafRange(response) = response else {
        panic!("expected a PbtLeafRange, got {response}");
    };
    assert!(!response.leaves.is_empty());

    // The same bytes, checked against the root the client was actually syncing
    // to. Nothing about the response changes; only the question does.
    let other_pivot = H256::repeat_byte(0x42);
    assert_ne!(other_pivot, pivot.state_root);
    let error = verify_range(
        other_pivot,
        &[],
        &to_leaves(&response),
        &to_proof(&response.left_proof),
        &to_proof(&response.right_proof),
    )
    .expect_err("a range from another tree must not pass for the pivot's");
    // `Proof`, not `RootMismatch`: the boundary walks are themselves bound to
    // the root they are checked against, so a wrong root is caught while
    // hashing the left walk and never reaches the re-merkleization. Naming the
    // variant rather than accepting any error is deliberate — an assertion of
    // "some error" here would also pass if the response were malformed, which
    // is a different failure and would hide this one.
    assert!(matches!(error, RangeProofError::Proof(_)), "got {error:?}");

    // And a single flipped byte in a leaf value the client decoded off the wire
    // is caught too — the verification runs on what arrived, not on what the
    // server meant to send.
    let mut tampered = to_leaves(&response);
    tampered[0].1[0] ^= 1;
    assert!(
        verify_range(
            pivot.state_root,
            &[],
            &tampered,
            &to_proof(&response.left_proof),
            &to_proof(&response.right_proof),
        )
        .is_err(),
        "a tampered leaf must not verify",
    );
}

/// A scheduled chain and an unscheduled one are different chains, and the
/// handshake says so before any capability is looked at.
///
/// This started as a wire test of the advertisement gate — connect a scheduled
/// client to an unscheduled server and assert the peer offers no `pbtsnap`.
/// That test cannot exist. The two nodes never reach `Established`: their
/// genesis hashes agree (a schedule after genesis leaves the genesis header
/// MPT-committed, which is why a real network always schedules into the future)
/// but `binary_tree_time` joins the timestamp fork list, so `validate_status`
/// rejects on the fork id. Measured, not assumed: the pair above produces
/// `0x81f73ca3` and `0xe933f004`.
///
/// Recorded as a test rather than a comment because it is the *stronger* form
/// of the property the devnets care about. An unscheduled node is not merely
/// quiet about `pbtsnap`; it cannot peer with a scheduled one at all, so no
/// amount of capability-gate breakage could expose it to a pbtsnap request from
/// a chain it does not run. The gate itself stays unit-tested in
/// `rlpx::connection::server`'s `capability_tests`, which is the only level at
/// which it is observable.
#[test]
fn a_scheduled_chain_and_an_unscheduled_one_cannot_peer() {
    let scheduled = shared_genesis(true);
    let unscheduled = shared_genesis(false);

    assert_eq!(
        scheduled.get_block().hash(),
        unscheduled.get_block().hash(),
        "the schedule is after genesis, so it must not move the genesis hash",
    );

    let fork_id = |genesis: Genesis| {
        ForkId::new(
            genesis.config,
            genesis.get_block().header,
            PIVOT_TIMESTAMP,
            1,
        )
    };
    assert_ne!(
        fork_id(scheduled),
        fork_id(unscheduled),
        "scheduling the binary tree must change the fork id, or a node on one \
         chain could peer with a node on the other",
    );
}
