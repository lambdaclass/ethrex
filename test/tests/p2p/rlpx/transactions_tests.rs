use ethrex_common::{
    H256,
    types::{BlobsBundle, EIP4844Transaction, PooledTransaction, Transaction},
};
use ethrex_p2p::rlpx::{
    eth::transactions::{GetPooledTransactions, PooledTransactions},
    message::RLPxMessage,
};

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
    let transaction1 =
        PooledTransaction::from_transaction(Transaction::LegacyTransaction(Default::default()))
            .unwrap();
    let pooled_transactions = vec![transaction1.clone()];
    let pooled_transactions = PooledTransactions::new(1, pooled_transactions);

    let mut buf = Vec::new();
    pooled_transactions.encode(&mut buf).unwrap();
    let decoded = PooledTransactions::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert_eq!(decoded.pooled_transactions, vec![transaction1]);
}

// A blob transaction is gossiped in wrapped (network) form carrying its blobs
// bundle. It must survive a full `PooledTransactions` RLP round-trip with the
// sidecar intact, and `as_blob()` must expose both the tx and the bundle.
#[test]
fn pooled_transactions_blob_roundtrip_preserves_sidecar() {
    let blob_tx = PooledTransaction::new_blob(EIP4844Transaction::default(), BlobsBundle::empty());
    // A non-blob tx in the same message must round-trip with no sidecar.
    let legacy_tx =
        PooledTransaction::from_transaction(Transaction::LegacyTransaction(Default::default()))
            .unwrap();
    let original = vec![blob_tx.clone(), legacy_tx.clone()];
    let pooled_transactions = PooledTransactions::new(7, original.clone());

    let mut buf = Vec::new();
    pooled_transactions.encode(&mut buf).unwrap();
    let decoded = PooledTransactions::decode(&buf).unwrap();

    assert_eq!(decoded.id, 7);
    assert_eq!(decoded.pooled_transactions, original);
    // The blob variant still carries its bundle after the round-trip.
    assert!(decoded.pooled_transactions[0].as_blob().is_some());
    // The legacy variant carries none.
    assert!(decoded.pooled_transactions[1].as_blob().is_none());
}
