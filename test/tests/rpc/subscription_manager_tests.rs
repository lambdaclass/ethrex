use ethrex_common::{H256, types::BlockHeader};
use ethrex_rpc::subscription_manager::{
    MAX_TOTAL_SUBSCRIPTIONS, SUBSCRIBER_CHANNEL_CAPACITY, SubscriptionManager,
    SubscriptionManagerProtocol,
};
use tokio::sync::mpsc;

#[tokio::test]
async fn subscribe_returns_unique_ids() {
    let manager = SubscriptionManager::spawn();

    let (tx1, _rx1) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let (tx2, _rx2) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);

    let id1 = manager.subscribe(tx1).await.unwrap().unwrap();
    let id2 = manager.subscribe(tx2).await.unwrap().unwrap();

    assert_ne!(id1, id2);
    assert!(id1.starts_with("0x"));
    assert!(id2.starts_with("0x"));
}

#[tokio::test]
async fn unsubscribe_existing_returns_true() {
    let manager = SubscriptionManager::spawn();

    let (tx, _rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let id = manager.subscribe(tx).await.unwrap().unwrap();

    let removed = manager.unsubscribe(id).await.unwrap();
    assert!(removed);
}

#[tokio::test]
async fn unsubscribe_nonexistent_returns_false() {
    let manager = SubscriptionManager::spawn();

    let removed = manager.unsubscribe("0xdeadbeef".to_string()).await.unwrap();
    assert!(!removed);
}

#[tokio::test]
async fn unsubscribe_twice_returns_false_second_time() {
    let manager = SubscriptionManager::spawn();

    let (tx, _rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let id = manager.subscribe(tx).await.unwrap().unwrap();

    assert!(manager.unsubscribe(id.clone()).await.unwrap());
    assert!(!manager.unsubscribe(id).await.unwrap());
}

#[tokio::test]
async fn new_head_fans_out_to_all_subscribers() {
    let manager = SubscriptionManager::spawn();

    let (tx1, mut rx1) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let (tx2, mut rx2) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);

    let id1 = manager.subscribe(tx1).await.unwrap().unwrap();
    let id2 = manager.subscribe(tx2).await.unwrap().unwrap();

    let header = BlockHeader::default();
    manager.new_head(header).unwrap();

    let msg1 = tokio::time::timeout(std::time::Duration::from_secs(2), rx1.recv())
        .await
        .expect("timed out waiting for subscriber 1")
        .expect("channel closed");
    let msg2 = tokio::time::timeout(std::time::Duration::from_secs(2), rx2.recv())
        .await
        .expect("timed out waiting for subscriber 2")
        .expect("channel closed");

    // Verify notification envelope structure.
    let v1: serde_json::Value = serde_json::from_str(&msg1).unwrap();
    assert_eq!(v1["jsonrpc"], "2.0");
    assert_eq!(v1["method"], "eth_subscription");
    assert_eq!(v1["params"]["subscription"], id1);
    assert!(v1["params"]["result"]["hash"].is_string());

    let v2: serde_json::Value = serde_json::from_str(&msg2).unwrap();
    assert_eq!(v2["params"]["subscription"], id2);
}

#[tokio::test]
async fn new_head_removes_dead_subscribers() {
    let manager = SubscriptionManager::spawn();

    let (tx_alive, mut rx_alive) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let (tx_dead, rx_dead) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);

    let _id_alive = manager.subscribe(tx_alive).await.unwrap().unwrap();
    let id_dead = manager.subscribe(tx_dead).await.unwrap().unwrap();

    // Drop the receiver so the dead subscriber's channel is closed.
    drop(rx_dead);

    let header = BlockHeader::default();
    manager.new_head(header).unwrap();

    // The alive subscriber should still receive the notification.
    let msg = tokio::time::timeout(std::time::Duration::from_secs(2), rx_alive.recv())
        .await
        .expect("timed out waiting for alive subscriber")
        .expect("channel closed");
    assert!(!msg.is_empty());

    // The dead subscriber should have been cleaned up — unsubscribe returns false.
    // Give the actor a moment to process the new_head and clean up.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let removed = manager.unsubscribe(id_dead).await.unwrap();
    assert!(!removed, "dead subscriber should have been cleaned up");
}

#[tokio::test]
async fn new_head_with_no_subscribers_does_not_panic() {
    let manager = SubscriptionManager::spawn();
    let header = BlockHeader::default();
    // Should not panic or error.
    manager.new_head(header).unwrap();
}

#[tokio::test]
async fn notification_contains_block_hash() {
    let manager = SubscriptionManager::spawn();

    let (tx, mut rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let _id = manager.subscribe(tx).await.unwrap().unwrap();

    let header = BlockHeader::default();
    let expected_hash = format!("{:#x}", header.hash());

    manager.new_head(header).unwrap();

    let msg = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");

    let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
    assert_eq!(
        v["params"]["result"]["hash"].as_str().unwrap(),
        expected_hash
    );
}

fn syncing(current_block: u64) -> ethrex_rpc::subscription_manager::SyncStatus {
    use ethrex_rpc::subscription_manager::{SyncStatus, SyncingStatusRpc};
    SyncStatus::Syncing(SyncingStatusRpc {
        starting_block: 0,
        current_block,
        highest_block: 100,
    })
}

fn next_result(rx: &mut mpsc::Receiver<String>) -> Option<serde_json::Value> {
    let notification: serde_json::Value = serde_json::from_str(&rx.try_recv().ok()?).unwrap();
    Some(notification["params"]["result"].clone())
}

#[tokio::test]
async fn sync_status_change_reaches_only_syncing_subscribers() {
    use ethrex_rpc::subscription_manager::SyncStatus;
    let manager = SubscriptionManager::spawn();

    let (heads_tx, mut heads_rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let (tx1, mut rx1) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let (tx2, mut rx2) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    manager.subscribe(heads_tx).await.unwrap().unwrap();
    manager.subscribe_syncing(tx1).await.unwrap().unwrap();
    manager.subscribe_syncing(tx2).await.unwrap().unwrap();
    assert!(next_result(&mut rx1).is_none());

    manager.new_sync_status(syncing(5)).unwrap();
    manager.new_sync_status(SyncStatus::Synced).unwrap();
    manager.unsubscribe(String::new()).await.unwrap();

    let expected = serde_json::json!({
        "syncing": true,
        "startingBlock": "0x0",
        "currentBlock": "0x5",
        "highestBlock": "0x64",
    });
    for rx in [&mut rx1, &mut rx2] {
        assert_eq!(next_result(rx), Some(expected.clone()));
        assert_eq!(next_result(rx), Some(serde_json::Value::Bool(false)));
        assert!(next_result(rx).is_none());
    }
    assert!(heads_rx.try_recv().is_err());
}

#[tokio::test]
async fn sync_status_is_notified_only_when_it_changes() {
    use ethrex_rpc::subscription_manager::SyncStatus;
    let manager = SubscriptionManager::spawn();

    let (tx, mut rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    manager.subscribe_syncing(tx).await.unwrap().unwrap();

    manager.new_sync_status(syncing(5)).unwrap();
    manager.new_sync_status(syncing(6)).unwrap();
    manager.new_sync_status(SyncStatus::Synced).unwrap();
    manager.new_sync_status(SyncStatus::Synced).unwrap();
    manager.unsubscribe(String::new()).await.unwrap();

    assert_eq!(next_result(&mut rx).unwrap()["currentBlock"], "0x5");
    assert_eq!(next_result(&mut rx), Some(serde_json::Value::Bool(false)));
    assert!(next_result(&mut rx).is_none());

    manager.new_sync_status(syncing(7)).unwrap();
    let (late_tx, mut late_rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    manager.subscribe_syncing(late_tx).await.unwrap().unwrap();
    assert!(next_result(&mut late_rx).is_none());
    manager.new_sync_status(syncing(8)).unwrap();
    manager.unsubscribe(String::new()).await.unwrap();
    assert_eq!(next_result(&mut late_rx).unwrap()["currentBlock"], "0x8");
    assert!(next_result(&mut late_rx).is_none());
    assert_eq!(next_result(&mut rx).unwrap()["currentBlock"], "0x7");
    assert!(next_result(&mut rx).is_none());
}

#[tokio::test]
async fn sync_status_owed_to_a_full_channel_is_sent_later() {
    use ethrex_rpc::subscription_manager::SyncStatus;
    let manager = SubscriptionManager::spawn();

    let (tx, mut rx) = mpsc::channel(1);
    tx.try_send("busy".to_string()).unwrap();
    manager.subscribe_syncing(tx).await.unwrap().unwrap();
    manager.new_sync_status(SyncStatus::Synced).unwrap();
    manager.unsubscribe(String::new()).await.unwrap();
    assert_eq!(rx.try_recv().unwrap(), "busy");
    assert!(next_result(&mut rx).is_none());

    manager.new_sync_status(SyncStatus::Synced).unwrap();
    manager.unsubscribe(String::new()).await.unwrap();
    assert_eq!(next_result(&mut rx), Some(serde_json::Value::Bool(false)));
    assert!(next_result(&mut rx).is_none());
}

#[tokio::test]
async fn unsubscribe_stops_sync_status_notifications() {
    use ethrex_rpc::subscription_manager::SyncStatus;
    let manager = SubscriptionManager::spawn();

    let (tx, mut rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let id = manager.subscribe_syncing(tx).await.unwrap().unwrap();
    manager.new_sync_status(syncing(5)).unwrap();
    assert!(manager.unsubscribe(id.clone()).await.unwrap());
    assert_eq!(next_result(&mut rx).unwrap()["syncing"], true);

    manager.new_sync_status(SyncStatus::Synced).unwrap();
    assert!(!manager.unsubscribe(id).await.unwrap());
    assert!(next_result(&mut rx).is_none());
}

#[tokio::test]
async fn subscribe_returns_none_at_global_cap() {
    let manager = SubscriptionManager::spawn();

    // Fill the actor to the cap. Senders are kept alive in `keep_alive` so
    // the channels do not get cleaned up.
    let mut keep_alive = Vec::with_capacity(MAX_TOTAL_SUBSCRIPTIONS);
    for _ in 0..MAX_TOTAL_SUBSCRIPTIONS {
        let (tx, rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
        let id = manager.subscribe(tx).await.unwrap();
        assert!(id.is_some());
        keep_alive.push(rx);
    }

    // The next subscribe must be refused.
    let (tx_overflow, _rx_overflow) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let result = manager.subscribe(tx_overflow).await.unwrap();
    assert!(result.is_none(), "expected None at MAX_TOTAL_SUBSCRIPTIONS");
}

#[tokio::test]
async fn new_pending_transaction_reaches_only_its_subscribers() {
    let manager = SubscriptionManager::spawn();

    let (heads_tx, mut heads_rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let (pending_tx, mut pending_rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let _heads_id = manager.subscribe(heads_tx).await.unwrap().unwrap();
    let pending_id = manager
        .subscribe_pending_transactions(pending_tx)
        .await
        .unwrap()
        .unwrap();

    let hash = H256::from_low_u64_be(0xabc);
    manager.new_pending_transaction(hash).unwrap();

    let msg = tokio::time::timeout(std::time::Duration::from_secs(2), pending_rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
    assert_eq!(v["params"]["subscription"], pending_id);
    assert_eq!(v["params"]["result"], format!("{hash:#x}"));
    assert!(
        heads_rx.try_recv().is_err(),
        "newHeads subscriber got a tx hash"
    );

    assert!(manager.unsubscribe(pending_id.clone()).await.unwrap());
    assert!(!manager.unsubscribe(pending_id).await.unwrap());
}
