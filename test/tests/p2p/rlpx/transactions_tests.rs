use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::{
    Address, H256,
    types::{EIP1559Transaction, MempoolTransaction, P2PTransaction, Transaction},
};
use ethrex_crypto::NativeCrypto;
use ethrex_p2p::rlpx::{
    eth::transactions::{GetPooledTransactions, PooledTransactions},
    message::RLPxMessage,
};
use ethrex_storage::{EngineType, Store};

#[test]
fn get_pooled_transactions_empty_message() {
    let transaction_hashes = vec![];
    let get_pooled_transactions = GetPooledTransactions::new(1, transaction_hashes.clone());

    let mut buf = Vec::new();
    get_pooled_transactions.encode(&mut buf).unwrap();

    let decoded = GetPooledTransactions::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert_eq!(decoded.transaction_hashes, transaction_hashes);
}

#[test]
fn get_pooled_transactions_not_empty_message() {
    let transaction_hashes = vec![
        H256::from_low_u64_be(1),
        H256::from_low_u64_be(2),
        H256::from_low_u64_be(3),
    ];
    let get_pooled_transactions = GetPooledTransactions::new(1, transaction_hashes.clone());

    let mut buf = Vec::new();
    get_pooled_transactions.encode(&mut buf).unwrap();

    let decoded = GetPooledTransactions::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert_eq!(decoded.transaction_hashes, transaction_hashes);
}

#[test]
fn pooled_transactions_of_one_type() {
    let transaction1 = P2PTransaction::LegacyTransaction(Default::default());
    let pooled_transactions = vec![transaction1.clone()];
    let pooled_transactions = PooledTransactions::new(1, pooled_transactions);

    let mut buf = Vec::new();
    pooled_transactions.encode(&mut buf).unwrap();
    let decoded = PooledTransactions::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert_eq!(decoded.pooled_transactions, vec![transaction1]);
}

fn test_blockchain() -> Blockchain {
    let store = Store::new("", EngineType::InMemory).expect("in-memory store");
    Blockchain::default_with_store(store)
}

/// Adds an EIP-1559 tx (with `data_len` bytes of calldata) to the mempool and returns its hash.
fn add_mempool_tx(bc: &Blockchain, nonce: u64, data_len: usize) -> H256 {
    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        nonce,
        data: Bytes::from(vec![0u8; data_len]),
        ..Default::default()
    });
    let sender = Address::from_low_u64_be(1);
    let mtx = MempoolTransaction::new(tx, sender);
    let hash = mtx.hash(&NativeCrypto);
    bc.mempool
        .add_transaction(hash, sender, mtx, None, None)
        .expect("add to mempool");
    hash
}

/// `GetPooledTransactions::handle` must serve each requested hash at most once, so a request
/// padded with duplicates can't amplify the response or force repeated lookups.
#[test]
fn get_pooled_transactions_handle_dedups_requested_hashes() {
    let bc = test_blockchain();
    let h1 = add_mempool_tx(&bc, 0, 0);
    let h2 = add_mempool_tx(&bc, 1, 0);

    let req = GetPooledTransactions::new(7, vec![h1, h1, h2, h1, h2]);
    let resp = req.handle(&bc).expect("handle");

    assert_eq!(resp.id, 7);
    assert_eq!(
        resp.pooled_transactions.len(),
        2,
        "each requested hash must be served at most once"
    );
}

/// `GetPooledTransactions::handle` must stop once the response would exceed the serving budget
/// (geth `softResponseLimit`), so it never emits more than a peer's inbound cap accepts.
#[test]
fn get_pooled_transactions_handle_caps_response_bytes() {
    let bc = test_blockchain();
    // Five ~700 KiB txs (~3.5 MiB total) — well over the 2 MiB serving budget.
    let hashes: Vec<H256> = (0..5).map(|n| add_mempool_tx(&bc, n, 700 * 1024)).collect();

    let req = GetPooledTransactions::new(1, hashes.clone());
    let resp = req.handle(&bc).expect("handle");

    assert!(
        !resp.pooled_transactions.is_empty(),
        "at least one tx must be served"
    );
    assert!(
        resp.pooled_transactions.len() < hashes.len(),
        "the byte budget must stop the response short of the full {}-tx request, got {}",
        hashes.len(),
        resp.pooled_transactions.len()
    );
}
