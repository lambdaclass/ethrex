//! Peer-table request timeouts and how sync classifies a failed request.
//!
//! A timed-out `get_best_peer` under mailbox pressure once exited the node
//! (#6823). Peer selection now waits `PEER_SELECTION_TIMEOUT` instead of
//! spawned's five-second default, and a timeout is recoverable wherever a
//! peer-table error reaches the sync classifier.

use std::time::Duration;

use ethrex_p2p::{
    peer_handler::PeerHandlerError,
    peer_table::{
        PEER_LISTING_TIMEOUT, PEER_SELECTION_TIMEOUT, ToPeerTableServerRef as _,
        peer_table_server_protocol,
    },
    rlpx::p2p::Capability,
    snap::SnapError,
    sync::{SyncError, SyncFailureAction},
};
use spawned_concurrency::error::ActorError;
use spawned_test::tasks::Mock;

/// Longer than spawned's default request timeout, shorter than the ones set
/// on the slow peer-table methods.
const SLOW_REPLY: Duration = Duration::from_secs(6);

#[tokio::test(start_paused = true)]
async fn peer_selection_outlasts_the_default_timeout() {
    assert!(PEER_SELECTION_TIMEOUT > SLOW_REPLY);
    let table = Mock::new()
        .on::<peer_table_server_protocol::GetBestPeer>(|_| None)
        .delay::<peer_table_server_protocol::GetBestPeer>(SLOW_REPLY)
        .start()
        .to_peer_table_server_ref();

    let picked = table.get_best_peer(vec![Capability::eth(68)]).await;

    assert!(matches!(picked, Ok(None)), "got {picked:?}");
}

#[tokio::test(start_paused = true)]
async fn peer_listing_outlasts_the_default_timeout() {
    assert!(PEER_LISTING_TIMEOUT > SLOW_REPLY);
    let table = Mock::new()
        .on::<peer_table_server_protocol::GetPeersData>(|_| Vec::new())
        .delay::<peer_table_server_protocol::GetPeersData>(SLOW_REPLY)
        .start()
        .to_peer_table_server_ref();

    assert!(table.get_peers_data().await.is_ok());
}

/// The other requests keep spawned's five-second default.
#[tokio::test(start_paused = true)]
async fn other_requests_keep_the_default_timeout() {
    let table = Mock::new()
        .on::<peer_table_server_protocol::PeerCount>(|_| 0)
        .delay::<peer_table_server_protocol::PeerCount>(SLOW_REPLY)
        .start()
        .to_peer_table_server_ref();

    let count = table.peer_count().await;

    assert!(
        matches!(count, Err(ActorError::RequestTimeout)),
        "got {count:?}"
    );
}

/// Every path a peer-table error takes into the sync classifier: a timeout
/// retries, a stopped table exits while the node runs.
fn peer_table_errors(err: fn() -> ActorError) -> [SyncError; 3] {
    [
        SyncError::PeerTableError(err()),
        SyncError::PeerHandler(PeerHandlerError::PeerTableError(err())),
        SyncError::Snap(SnapError::PeerTable(err())),
    ]
}

#[test]
fn a_peer_table_timeout_retries_the_sync_cycle() {
    for error in peer_table_errors(|| ActorError::RequestTimeout) {
        assert!(error.is_recoverable(), "{error:?}");
        assert_eq!(error.failure_action(false), SyncFailureAction::Retry);
    }
}

/// spawned 0.8: a dropped reply and a refusal at a bound are transient.
#[test]
fn a_dropped_or_refused_peer_table_request_retries_the_sync_cycle() {
    for err in [|| ActorError::ReplyDropped, || ActorError::Overloaded] {
        for error in peer_table_errors(err) {
            assert!(error.is_recoverable(), "{error:?}");
            assert_eq!(error.failure_action(false), SyncFailureAction::Retry);
        }
    }
}

#[test]
fn a_stopped_peer_table_exits_only_while_the_node_runs() {
    for error in peer_table_errors(|| ActorError::ActorStopped) {
        assert!(!error.is_recoverable(), "{error:?}");
        assert_eq!(error.failure_action(false), SyncFailureAction::Exit);
        assert_eq!(
            error.failure_action(true),
            SyncFailureAction::StopForShutdown
        );
    }
}
