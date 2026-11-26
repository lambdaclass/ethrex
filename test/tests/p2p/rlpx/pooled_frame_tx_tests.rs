//! EIP-8141 frame transactions between peers.
//!
//! A frame transaction is announced by hash in `NewPooledTransactionHashes` and
//! fetched with `GetPooledTransactions`, like any pooled transaction. What a peer
//! returns is its pooled representation, `rlp([tx_body, wrapper_version,
//! witnesses, blob_sidecar])`, so the witnesses of its claims travel with it. No
//! message and no protocol version is specific to it.

use std::net::{IpAddr, Ipv4Addr};

use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::{
    H256, H512,
    types::{Fork, MAX_TX_SIZE, P2PTransaction, PooledFrameTransaction, Transaction, TxType},
};
use ethrex_crypto::NativeCrypto;
use ethrex_p2p::{
    rlpx::{
        eth::{
            eth72::transactions::{NewPooledTransactionHashes72, PooledTransactions72},
            transactions::{GetPooledTransactions, NewPooledTransactionHashes, PooledTransactions},
        },
        message::RLPxMessage,
    },
    types::Node,
};
use ethrex_rlp::encode::RLPEncode;

use crate::blockchain::eip8141_claims_tests::{
    Key, blockchain, claim_transaction, key, pooled, setup_store,
};

fn encode<M: RLPxMessage>(msg: &M) -> Vec<u8> {
    let mut buf = vec![];
    msg.encode(&mut buf).expect("encode");
    buf
}

fn peer() -> Node {
    Node::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 30303, 30303, H512::zero())
}

/// A node at Hegota whose state has the key's account, and the key's
/// transaction in its pooled representation, signed.
async fn node_and_transaction(key: &Key) -> (Blockchain, PooledFrameTransaction) {
    let (store, chain_id) = setup_store(&[key]).await;
    let pooled_tx = pooled(claim_transaction(chain_id, key, Vec::new()), &[key]);
    (blockchain(store), pooled_tx)
}

fn hash_of(pooled_tx: &PooledFrameTransaction) -> H256 {
    Transaction::FrameTransaction(pooled_tx.tx.clone()).hash(&NativeCrypto)
}

/// The length of the transaction as it travels: the type byte and the pooled
/// representation.
fn pooled_len(pooled_tx: &PooledFrameTransaction) -> usize {
    1 + pooled_tx.length()
}

#[tokio::test]
async fn a_frame_transaction_travels_between_two_nodes_with_its_witnesses() {
    let key = key(0x81);
    let (sender_node, pooled_tx) = node_and_transaction(&key).await;
    let (receiver_node, _) = node_and_transaction(&key).await;
    let hash = sender_node
        .add_frame_transaction_to_pool(pooled_tx.clone())
        .await
        .expect("the sender admits its transaction");
    assert_eq!(hash, hash_of(&pooled_tx));

    // The sender announces the transaction by hash, with its type and the size
    // of its pooled representation.
    let announced = sender_node.mempool.get_txs_for_broadcast().unwrap();
    assert_eq!(announced.len(), 1);
    let transactions: Vec<Transaction> = announced
        .iter()
        .map(|tx| tx.transaction().clone())
        .collect();
    let announcement =
        NewPooledTransactionHashes::new(transactions, &sender_node).expect("announce");
    assert_eq!(announcement.transaction_hashes, vec![hash]);
    assert_eq!(
        announcement.transaction_types.as_ref(),
        &[TxType::Frame as u8]
    );
    assert_eq!(announcement.transaction_sizes, vec![pooled_len(&pooled_tx)]);
    let announcement = NewPooledTransactionHashes::decode(&encode(&announcement))
        .expect("the announcement decodes");

    // The receiver does not have it, asks for it, and gets the pooled
    // representation: the transaction and the witness of its claim.
    let request = GetPooledTransactions::decode(&encode(&GetPooledTransactions::new(
        7,
        announcement.transaction_hashes.clone(),
    )))
    .expect("the request decodes");
    let response = request.handle(&sender_node).expect("serve the request");
    let delivered = PooledTransactions::decode(&encode(&response)).expect("the response decodes");
    assert_eq!(delivered.id, 7);
    assert_eq!(delivered.pooled_transactions.len(), 1);
    let P2PTransaction::FrameTransaction(delivered_tx) = &delivered.pooled_transactions[0] else {
        panic!("expected a frame transaction");
    };
    assert_eq!(delivered_tx.witnesses, pooled_tx.witnesses);
    assert_eq!(delivered_tx.encode_to_vec(), pooled_tx.encode_to_vec());
    delivered
        .validate_requested(&announcement, Fork::Hegota)
        .expect("the delivery matches the announcement");

    // The receiver admits it, which verifies the witness, and keeps both.
    delivered
        .handle(&peer(), &receiver_node, false)
        .await
        .expect("handle the response");
    assert!(receiver_node.mempool.contains_tx(hash).unwrap());
    assert_eq!(
        receiver_node.mempool.get_claim_witnesses(hash).unwrap(),
        Some(pooled_tx.witnesses.clone())
    );
    // And can serve it to the next peer in the same form.
    assert_eq!(
        receiver_node
            .get_p2p_transaction_by_hash(&hash)
            .unwrap()
            .encode_canonical_to_vec(),
        P2PTransaction::FrameTransaction(pooled_tx).encode_canonical_to_vec()
    );
}

#[tokio::test]
async fn a_frame_transaction_travels_over_eth72_like_over_earlier_versions() {
    let key = key(0x82);
    let (sender_node, pooled_tx) = node_and_transaction(&key).await;
    let (receiver_node, _) = node_and_transaction(&key).await;
    let hash = sender_node
        .add_frame_transaction_to_pool(pooled_tx.clone())
        .await
        .expect("the sender admits its transaction");

    let announcement = NewPooledTransactionHashes72::new(
        vec![Transaction::FrameTransaction(pooled_tx.tx.clone())],
        &sender_node,
    )
    .expect("announce");
    assert_eq!(announcement.transaction_hashes, vec![hash]);
    assert_eq!(announcement.transaction_sizes, vec![pooled_len(&pooled_tx)]);
    assert_eq!(
        announcement.cell_mask, None,
        "the cell mask covers type-3 transactions only"
    );

    let served = GetPooledTransactions::new(1, vec![hash])
        .handle(&sender_node)
        .expect("serve the request");
    let delivered = PooledTransactions72::decode(&encode(&PooledTransactions72::new(
        1,
        served.pooled_transactions,
    )))
    .expect("the response decodes");
    delivered
        .validate_requested(&announcement, Fork::Hegota)
        .expect("the delivery matches the announcement");
    delivered
        .handle(&peer(), &receiver_node, false)
        .await
        .expect("handle the response");
    assert_eq!(
        receiver_node.mempool.get_claim_witnesses(hash).unwrap(),
        Some(pooled_tx.witnesses)
    );
}

#[tokio::test]
async fn a_node_does_not_admit_a_frame_transaction_whose_witness_is_invalid() {
    let key = key(0x83);
    let (receiver_node, pooled_tx) = node_and_transaction(&key).await;
    let hash = hash_of(&pooled_tx);

    // The witness of another transaction, of the right length.
    let mut forged = pooled_tx.clone();
    let mut bytes = forged.witnesses[0].to_vec();
    bytes[0] ^= 1;
    forged.witnesses[0] = Bytes::from(bytes);
    // No witness at all.
    let mut without = pooled_tx.clone();
    without.witnesses.clear();

    for invalid in [forged, without] {
        let delivered = PooledTransactions::decode(&encode(&PooledTransactions::new(
            1,
            vec![P2PTransaction::FrameTransaction(invalid)],
        )))
        .expect("the response decodes");
        delivered
            .handle(&peer(), &receiver_node, false)
            .await
            .expect("an inadmissible transaction is dropped");
        assert!(!receiver_node.mempool.contains_tx(hash).unwrap());
    }

    // The valid one is admitted afterwards: a bad copy does not shadow it.
    PooledTransactions::new(1, vec![P2PTransaction::FrameTransaction(pooled_tx)])
        .handle(&peer(), &receiver_node, false)
        .await
        .expect("handle the response");
    assert!(receiver_node.mempool.contains_tx(hash).unwrap());
}

// A peer cannot make a node read more witness bytes than the claims of a
// transaction allow: a response that carries such a pooled representation does
// not decode, on either version of the message. One that reaches the handler
// some other way is dropped there, before admission.
#[tokio::test]
async fn an_oversized_pooled_representation_from_a_peer_is_not_admitted() {
    let key = key(0x85);
    let (receiver_node, pooled_tx) = node_and_transaction(&key).await;
    let hash = hash_of(&pooled_tx);

    // A witness the transaction has no claim for, large enough to take the
    // pooled representation over its size cap.
    let mut oversized = pooled_tx.clone();
    oversized
        .witnesses
        .push(Bytes::from(vec![0u8; MAX_TX_SIZE]));
    assert!(oversized.wire_len() > oversized.max_wire_len());
    let response = vec![P2PTransaction::FrameTransaction(oversized)];

    assert!(
        PooledTransactions::decode(&encode(&PooledTransactions::new(1, response.clone()))).is_err()
    );
    assert!(
        PooledTransactions72::decode(&encode(&PooledTransactions72::new(1, response.clone())))
            .is_err()
    );

    PooledTransactions::new(1, response.clone())
        .handle(&peer(), &receiver_node, false)
        .await
        .expect("an oversized transaction is dropped");
    assert!(!receiver_node.mempool.contains_tx(hash).unwrap());
    PooledTransactions72::new(1, response)
        .handle(&peer(), &receiver_node, false)
        .await
        .expect("an oversized transaction is dropped");
    assert!(!receiver_node.mempool.contains_tx(hash).unwrap());

    // The same transaction with the witnesses its claims call for is admitted.
    PooledTransactions::new(1, vec![P2PTransaction::FrameTransaction(pooled_tx)])
        .handle(&peer(), &receiver_node, false)
        .await
        .expect("handle the response");
    assert!(receiver_node.mempool.contains_tx(hash).unwrap());
}

// Between peers a frame transaction has one form. The bare transaction body,
// the form it has inside a block, does not decode as a pooled transaction, and
// neither does a pooled representation of another wrapper version.
#[tokio::test]
async fn only_the_pooled_representation_decodes_as_a_pooled_frame_transaction() {
    let key = key(0x84);
    let (_, pooled_tx) = node_and_transaction(&key).await;

    let pooled_item = |canonical: Vec<u8>| {
        let mut item = Vec::new();
        <[u8] as RLPEncode>::encode(&canonical, &mut item);
        item
    };

    let mut pooled_form = vec![TxType::Frame as u8];
    pooled_tx.encode(&mut pooled_form);
    let item = pooled_item(pooled_form);
    let (decoded, rest) =
        <P2PTransaction as ethrex_rlp::decode::RLPDecode>::decode_unfinished(&item)
            .expect("the pooled representation decodes");
    assert!(rest.is_empty());
    assert_eq!(decoded.tx_type(), TxType::Frame);

    let bare = Transaction::FrameTransaction(pooled_tx.tx.clone()).encode_canonical_to_vec();
    assert!(
        <P2PTransaction as ethrex_rlp::decode::RLPDecode>::decode_unfinished(&pooled_item(bare))
            .is_err()
    );

    let mut other_version = vec![TxType::Frame as u8];
    ethrex_rlp::structs::Encoder::new(&mut other_version)
        .encode_field(&pooled_tx.tx)
        .encode_field(&1u8)
        .encode_field(&pooled_tx.witnesses)
        .encode_raw(&[0xc0])
        .finish();
    assert!(
        <P2PTransaction as ethrex_rlp::decode::RLPDecode>::decode_unfinished(&pooled_item(
            other_version
        ))
        .is_err()
    );
}
