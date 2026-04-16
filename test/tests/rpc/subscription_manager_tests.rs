use ethrex_common::types::BlockHeader;
use ethrex_rpc::subscription_manager::{
    SubscriptionManager, SubscriptionManagerProtocol, SUBSCRIBER_CHANNEL_CAPACITY,
};
use tokio::sync::mpsc;

#[tokio::test]
async fn subscribe_returns_unique_ids() {
    let manager = SubscriptionManager::spawn();

    let (tx1, _rx1) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let (tx2, _rx2) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);

    let id1 = manager.subscribe(tx1).await.unwrap();
    let id2 = manager.subscribe(tx2).await.unwrap();

    assert_ne!(id1, id2);
    assert!(id1.starts_with("0x"));
    assert!(id2.starts_with("0x"));
}

#[tokio::test]
async fn unsubscribe_existing_returns_true() {
    let manager = SubscriptionManager::spawn();

    let (tx, _rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let id = manager.subscribe(tx).await.unwrap();

    let removed = manager.unsubscribe(id).await.unwrap();
    assert!(removed);
}

#[tokio::test]
async fn unsubscribe_nonexistent_returns_false() {
    let manager = SubscriptionManager::spawn();

    let removed = manager
        .unsubscribe("0xdeadbeef".to_string())
        .await
        .unwrap();
    assert!(!removed);
}

#[tokio::test]
async fn unsubscribe_twice_returns_false_second_time() {
    let manager = SubscriptionManager::spawn();

    let (tx, _rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let id = manager.subscribe(tx).await.unwrap();

    assert!(manager.unsubscribe(id.clone()).await.unwrap());
    assert!(!manager.unsubscribe(id).await.unwrap());
}

#[tokio::test]
async fn new_head_fans_out_to_all_subscribers() {
    let manager = SubscriptionManager::spawn();

    let (tx1, mut rx1) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let (tx2, mut rx2) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);

    let id1 = manager.subscribe(tx1).await.unwrap();
    let id2 = manager.subscribe(tx2).await.unwrap();

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

    let _id_alive = manager.subscribe(tx_alive).await.unwrap();
    let id_dead = manager.subscribe(tx_dead).await.unwrap();

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
    let _id = manager.subscribe(tx).await.unwrap();

    let header = BlockHeader::default();
    let expected_hash = format!("{:#x}", header.hash());

    manager.new_head(header).unwrap();

    let msg = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out")
        .expect("channel closed");

    let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
    assert_eq!(v["params"]["result"]["hash"].as_str().unwrap(), expected_hash);
}
