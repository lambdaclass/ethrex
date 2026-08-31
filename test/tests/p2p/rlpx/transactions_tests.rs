use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::{
    Address, H256,
    types::{
        BYTES_PER_BLOB, BlobsBundle, EIP1559Transaction, EIP4844Transaction, MempoolTransaction,
        P2PTransaction, Transaction,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_p2p::rlpx::{
    eth::transactions::{GetPooledTransactions, NewPooledTransactionHashes, PooledTransactions},
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

/// Adds a 1-blob EIP-4844 tx to the mempool with the given bundle (or none, mimicking the
/// state after the bundle was dropped by eviction or payload building) and returns the tx.
fn add_blob_tx_with_bundle(
    bc: &Blockchain,
    nonce: u64,
    bundle: Option<BlobsBundle>,
) -> Transaction {
    let tx = Transaction::EIP4844Transaction(EIP4844Transaction {
        nonce,
        gas: 21_000,
        to: Address::from_low_u64_be(1),
        ..Default::default()
    });
    let sender = Address::from_low_u64_be(2);
    let mtx = MempoolTransaction::new(tx.clone(), sender);
    let hash = mtx.hash(&NativeCrypto);
    if let Some(bundle) = bundle {
        bc.mempool
            .add_blobs_bundle(hash, bundle)
            .expect("add bundle");
    }
    bc.mempool
        .add_transaction(hash, sender, mtx, None, None)
        .expect("add to mempool");
    tx
}

fn full_bundle() -> BlobsBundle {
    BlobsBundle {
        blobs: vec![[0u8; BYTES_PER_BLOB]],
        commitments: vec![[0u8; 48]],
        proofs: vec![[0u8; 48]],
        version: 0,
    }
}

/// An eth/72 elided bundle: commitments and proofs present, blobs absent (cells are stored
/// and fetched separately).
fn elided_bundle() -> BlobsBundle {
    BlobsBundle {
        blobs: vec![],
        commitments: vec![[0u8; 48]],
        proofs: vec![[0u8; 48]; 128],
        version: 1,
    }
}

/// A blob tx whose bundle left the pool (pulled into a payload or evicted after the
/// broadcaster snapshot) can't be served, so it must not be announced — substituting an
/// empty bundle would announce a size no delivery can match, and peers checking announced
/// vs delivered size (geth's fetcher, our own `validate_requested`) disconnect over it.
/// The parallel type/size/hash arrays must stay in lockstep across the skip.
#[test]
fn pre72_announcement_skips_a_blob_tx_whose_bundle_is_gone() {
    let bc = test_blockchain();
    let normal_hash = add_mempool_tx(&bc, 0, 0);
    let normal_tx = bc
        .mempool
        .get_transaction_by_hash(normal_hash)
        .expect("mempool read")
        .expect("tx present");
    let gone_blob_tx = add_blob_tx_with_bundle(&bc, 0, None);

    let announcement =
        NewPooledTransactionHashes::new(vec![gone_blob_tx, normal_tx], &bc).expect("announce");

    assert_eq!(
        announcement.transaction_hashes,
        vec![normal_hash],
        "the bundle-less blob tx must not be announced"
    );
    assert_eq!(announcement.transaction_types.as_ref(), &[2u8]);
    assert_eq!(announcement.transaction_sizes.len(), 1);
}

/// A blob tx held elided (eth/72 ingest: commitments + proofs, no blobs) is dropped from
/// pre-72 `PooledTransactions` responses by the serve path, so a pre-72 announcement of it
/// advertises a hash we never deliver — with a size no full-blob delivery can match, which
/// geth charges to every announcer of the hash once someone else delivers the full tx.
/// A fully-held blob tx in the same batch must still be announced, at its full wrapped size.
#[test]
fn pre72_announcement_skips_an_elided_blob_tx_but_keeps_a_full_one() {
    let bc = test_blockchain();
    let elided_tx = add_blob_tx_with_bundle(&bc, 0, Some(elided_bundle()));
    let full_tx = add_blob_tx_with_bundle(&bc, 1, Some(full_bundle()));
    let full_hash = full_tx.hash(&NativeCrypto);

    let announcement =
        NewPooledTransactionHashes::new(vec![elided_tx, full_tx], &bc).expect("announce");

    assert_eq!(
        announcement.transaction_hashes,
        vec![full_hash],
        "the elided blob tx must not be announced on a pre-72 link"
    );
    assert_eq!(announcement.transaction_types.as_ref(), &[3u8]);
    assert!(
        announcement.transaction_sizes[0] > BYTES_PER_BLOB,
        "the announced size must be the full wrapped encoding (blobs included), got {}",
        announcement.transaction_sizes[0]
    );
}
