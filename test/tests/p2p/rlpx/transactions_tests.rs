use ethrex_common::{H256, types::P2PTransaction};
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
    let transaction1 = P2PTransaction::LegacyTransaction(Default::default());
    let pooled_transactions = vec![transaction1.clone()];
    let pooled_transactions = PooledTransactions::new(1, pooled_transactions);

    let mut buf = Vec::new();
    pooled_transactions.encode(&mut buf).unwrap();
    let decoded = PooledTransactions::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert_eq!(decoded.pooled_transactions, vec![transaction1]);
}
