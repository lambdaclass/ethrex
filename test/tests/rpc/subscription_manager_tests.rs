use ethrex_common::{
    Address, H256,
    types::{Block, BlockBody, BlockHeader, LegacyTransaction, Log, Receipt, Transaction, TxType},
};
use ethrex_rpc::subscription_manager::{
    LogSubscriptionFilter, MAX_LOGS_ROUTE_BLOCKS, MAX_TOTAL_SUBSCRIPTIONS,
    SUBSCRIBER_CHANNEL_CAPACITY, SubscriptionManager, SubscriptionManagerProtocol, TopicFilter,
};
use ethrex_storage::{EngineType, Store};
use tokio::sync::mpsc;

fn test_store() -> Store {
    Store::new("", EngineType::InMemory).expect("in-memory store")
}

#[tokio::test]
async fn subscribe_returns_unique_ids() {
    let manager = SubscriptionManager::spawn(test_store());

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
    let manager = SubscriptionManager::spawn(test_store());

    let (tx, _rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let id = manager.subscribe(tx).await.unwrap().unwrap();

    let removed = manager.unsubscribe(id).await.unwrap();
    assert!(removed);
}

#[tokio::test]
async fn unsubscribe_nonexistent_returns_false() {
    let manager = SubscriptionManager::spawn(test_store());

    let removed = manager.unsubscribe("0xdeadbeef".to_string()).await.unwrap();
    assert!(!removed);
}

#[tokio::test]
async fn unsubscribe_twice_returns_false_second_time() {
    let manager = SubscriptionManager::spawn(test_store());

    let (tx, _rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let id = manager.subscribe(tx).await.unwrap().unwrap();

    assert!(manager.unsubscribe(id.clone()).await.unwrap());
    assert!(!manager.unsubscribe(id).await.unwrap());
}

#[tokio::test]
async fn new_head_fans_out_to_all_subscribers() {
    let manager = SubscriptionManager::spawn(test_store());

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
    let manager = SubscriptionManager::spawn(test_store());

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
    let manager = SubscriptionManager::spawn(test_store());
    let header = BlockHeader::default();
    // Should not panic or error.
    manager.new_head(header).unwrap();
}

#[tokio::test]
async fn notification_contains_block_hash() {
    let manager = SubscriptionManager::spawn(test_store());

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

#[tokio::test]
async fn subscribe_returns_none_at_global_cap() {
    let manager = SubscriptionManager::spawn(test_store());

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
    let manager = SubscriptionManager::spawn(test_store());

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

async fn add_block(
    store: &Store,
    parent: &BlockHeader,
    timestamp: u64,
    logs: &[(u64, u64)],
) -> BlockHeader {
    let transactions = (0..logs.len() as u64)
        .map(|nonce| {
            Transaction::LegacyTransaction(LegacyTransaction {
                nonce,
                ..Default::default()
            })
        })
        .collect();
    let receipts = logs
        .iter()
        .map(|&(address, topic)| {
            let log = Log {
                address: Address::from_low_u64_be(address),
                topics: vec![H256::from_low_u64_be(topic)],
                data: Default::default(),
            };
            Receipt::new(TxType::Legacy, true, 21_000, vec![log])
        })
        .collect();
    let block = Block::new(
        BlockHeader {
            parent_hash: parent.hash(),
            number: parent.number + 1,
            timestamp,
            ..Default::default()
        },
        BlockBody {
            transactions,
            ommers: Default::default(),
            withdrawals: Default::default(),
        },
    );
    let header = block.header.clone();
    store.add_block(block).await.unwrap();
    store.add_receipts(header.hash(), receipts).await.unwrap();
    header
}

async fn received_logs(
    manager: &impl SubscriptionManagerProtocol,
    rx: &mut mpsc::Receiver<String>,
) -> Vec<(u64, u64, bool)> {
    // Waits for the actor to handle the messages sent before it.
    manager.unsubscribe("0x00".to_string()).await.unwrap();
    let mut logs = Vec::new();
    while let Ok(entry) = rx.try_recv() {
        for msg in entry.split('\n') {
            let v: serde_json::Value = serde_json::from_str(msg).unwrap();
            let log = &v["params"]["result"];
            let number = |field: &str| {
                u64::from_str_radix(log[field].as_str().unwrap().trim_start_matches("0x"), 16)
                    .unwrap()
            };
            let address = log["address"].as_str().unwrap().parse::<Address>().unwrap();
            logs.push((
                number("blockNumber"),
                address.to_low_u64_be(),
                log["removed"].as_bool().unwrap(),
            ));
        }
    }
    logs
}

fn address_filter(addresses: &[u64]) -> LogSubscriptionFilter {
    LogSubscriptionFilter {
        addresses: addresses
            .iter()
            .map(|a| Address::from_low_u64_be(*a))
            .collect(),
        ..Default::default()
    }
}

#[tokio::test]
async fn logs_subscription_gets_the_matching_logs_of_each_new_head() {
    let store = test_store();
    let manager = SubscriptionManager::spawn(store.clone());
    let (tx, mut rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    manager
        .subscribe_logs(tx, address_filter(&[1]))
        .await
        .unwrap()
        .unwrap();

    let genesis = BlockHeader::default();
    let one = add_block(&store, &genesis, 0, &[(1, 10), (2, 10), (1, 11)]).await;
    manager.new_head(one).unwrap();

    assert_eq!(
        received_logs(&manager, &mut rx).await,
        vec![(1, 1, false), (1, 1, false)]
    );
}

#[tokio::test]
async fn logs_subscription_matches_topics() {
    let store = test_store();
    let manager = SubscriptionManager::spawn(store.clone());
    let (tx, mut rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let filter = LogSubscriptionFilter {
        addresses: Default::default(),
        topics: vec![TopicFilter::Topic(Some(H256::from_low_u64_be(11)))],
        ..Default::default()
    };
    manager.subscribe_logs(tx, filter).await.unwrap().unwrap();

    let one = add_block(&store, &BlockHeader::default(), 0, &[(1, 10), (2, 11)]).await;
    manager.new_head(one).unwrap();

    assert_eq!(received_logs(&manager, &mut rx).await, vec![(1, 2, false)]);
}

#[tokio::test]
async fn logs_subscription_covers_every_block_a_head_adds() {
    let store = test_store();
    let manager = SubscriptionManager::spawn(store.clone());
    let (tx, mut rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    manager
        .subscribe_logs(tx, LogSubscriptionFilter::default())
        .await
        .unwrap()
        .unwrap();

    let one = add_block(&store, &BlockHeader::default(), 0, &[(1, 1)]).await;
    let two = add_block(&store, &one, 0, &[(2, 1)]).await;
    let three = add_block(&store, &two, 0, &[(3, 1)]).await;
    manager.new_head(one).unwrap();
    manager.new_head(three.clone()).unwrap();
    manager.new_head(three).unwrap();

    assert_eq!(
        received_logs(&manager, &mut rx).await,
        vec![(1, 1, false), (2, 2, false), (3, 3, false)]
    );
}

#[tokio::test]
async fn logs_subscription_removes_the_logs_of_a_reorged_out_chain() {
    let store = test_store();
    let manager = SubscriptionManager::spawn(store.clone());
    let (tx, mut rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    manager
        .subscribe_logs(tx, LogSubscriptionFilter::default())
        .await
        .unwrap()
        .unwrap();

    let one = add_block(&store, &BlockHeader::default(), 0, &[(1, 1)]).await;
    let two_a = add_block(&store, &one, 0, &[(2, 1)]).await;
    let three_a = add_block(&store, &two_a, 0, &[(3, 1)]).await;
    let two_b = add_block(&store, &one, 1, &[(4, 1)]).await;
    manager.new_head(one).unwrap();
    manager.new_head(three_a).unwrap();
    received_logs(&manager, &mut rx).await;

    manager.new_head(two_b).unwrap();

    assert_eq!(
        received_logs(&manager, &mut rx).await,
        vec![(2, 2, true), (3, 3, true), (2, 4, false)]
    );
}

#[tokio::test]
async fn logs_subscription_stops_after_unsubscribe() {
    let store = test_store();
    let manager = SubscriptionManager::spawn(store.clone());
    let (tx, mut rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let id = manager
        .subscribe_logs(tx, LogSubscriptionFilter::default())
        .await
        .unwrap()
        .unwrap();
    let one = add_block(&store, &BlockHeader::default(), 0, &[(1, 1)]).await;
    let two = add_block(&store, &one, 0, &[(2, 1)]).await;
    manager.new_head(one).unwrap();
    assert_eq!(received_logs(&manager, &mut rx).await, vec![(1, 1, false)]);

    assert!(manager.unsubscribe(id).await.unwrap());
    manager.new_head(two).unwrap();

    assert!(received_logs(&manager, &mut rx).await.is_empty());
}

#[tokio::test]
async fn logs_subscription_removes_logs_when_the_head_moves_back() {
    let store = test_store();
    let manager = SubscriptionManager::spawn(store.clone());
    let (tx, mut rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    manager
        .subscribe_logs(tx, LogSubscriptionFilter::default())
        .await
        .unwrap()
        .unwrap();
    let one = add_block(&store, &BlockHeader::default(), 0, &[(1, 1)]).await;
    let two = add_block(&store, &one, 0, &[(2, 1)]).await;
    let three = add_block(&store, &two, 0, &[(3, 1)]).await;
    manager.new_head(three).unwrap();
    received_logs(&manager, &mut rx).await;

    manager.new_head(one).unwrap();

    assert_eq!(
        received_logs(&manager, &mut rx).await,
        vec![(2, 2, true), (3, 3, true)]
    );
}

#[tokio::test]
async fn logs_subscription_sends_nothing_when_the_head_moves_back_past_the_limit() {
    let store = test_store();
    let manager = SubscriptionManager::spawn(store.clone());
    let (tx, mut rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    manager
        .subscribe_logs(tx, LogSubscriptionFilter::default())
        .await
        .unwrap()
        .unwrap();
    let first = add_block(&store, &BlockHeader::default(), 0, &[(1, 1)]).await;
    let mut head = first.clone();
    for _ in 0..=MAX_LOGS_ROUTE_BLOCKS {
        head = add_block(&store, &head, 0, &[(2, 1)]).await;
    }
    manager.new_head(head).unwrap();
    received_logs(&manager, &mut rx).await;

    manager.new_head(first).unwrap();

    assert!(received_logs(&manager, &mut rx).await.is_empty());
}

#[tokio::test]
async fn logs_subscription_gets_every_log_of_a_block_larger_than_its_channel() {
    let store = test_store();
    let manager = SubscriptionManager::spawn(store.clone());
    let (tx, mut rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    manager
        .subscribe_logs(tx, LogSubscriptionFilter::default())
        .await
        .unwrap()
        .unwrap();
    let logs = vec![(1, 1); SUBSCRIBER_CHANNEL_CAPACITY * 2];
    let one = add_block(&store, &BlockHeader::default(), 0, &logs).await;
    manager.new_head(one).unwrap();

    assert_eq!(
        received_logs(&manager, &mut rx).await.len(),
        SUBSCRIBER_CHANNEL_CAPACITY * 2
    );
}

#[tokio::test]
async fn logs_subscription_gets_only_the_head_after_a_long_jump() {
    let store = test_store();
    let manager = SubscriptionManager::spawn(store.clone());
    let (tx, mut rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    manager
        .subscribe_logs(tx, LogSubscriptionFilter::default())
        .await
        .unwrap()
        .unwrap();

    let first = add_block(&store, &BlockHeader::default(), 0, &[]).await;
    let mut head = first.clone();
    for _ in 0..=MAX_LOGS_ROUTE_BLOCKS {
        head = add_block(&store, &head, 0, &[(1, 1)]).await;
    }
    manager.new_head(first).unwrap();
    manager.new_head(head.clone()).unwrap();

    assert_eq!(
        received_logs(&manager, &mut rx).await,
        vec![(head.number, 1, false)]
    );
}

#[tokio::test]
async fn logs_subscription_applies_its_block_bounds() {
    let store = test_store();
    let manager = SubscriptionManager::spawn(store.clone());
    let (tx, mut rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let filter = LogSubscriptionFilter {
        from_block: Some(2),
        to_block: Some(3),
        ..Default::default()
    };
    manager.subscribe_logs(tx, filter).await.unwrap().unwrap();

    let one = add_block(&store, &BlockHeader::default(), 0, &[(1, 1)]).await;
    let two = add_block(&store, &one, 0, &[(2, 1)]).await;
    let three = add_block(&store, &two, 0, &[(3, 1)]).await;
    let four = add_block(&store, &three, 0, &[(4, 1)]).await;
    manager.new_head(one).unwrap();
    manager.new_head(four).unwrap();
    manager.new_head(two).unwrap();

    assert_eq!(
        received_logs(&manager, &mut rx).await,
        vec![(2, 2, false), (3, 3, false), (3, 3, true)]
    );
}

#[tokio::test]
async fn new_head_removes_closed_logs_subscribers_without_matching_logs() {
    let store = test_store();
    let manager = SubscriptionManager::spawn(store.clone());
    let (tx, rx) = mpsc::channel(SUBSCRIBER_CHANNEL_CAPACITY);
    let id = manager
        .subscribe_logs(tx, address_filter(&[2]))
        .await
        .unwrap()
        .unwrap();
    drop(rx);

    let one = add_block(&store, &BlockHeader::default(), 0, &[(1, 1)]).await;
    manager.new_head(one).unwrap();

    assert!(!manager.unsubscribe(id).await.unwrap());
}
