//! `TransactionQueue::pop` drops the head plus everything ordered behind it.
//! That reasoning is a property of the linear account-nonce domain, and neither
//! [EIP-8250] keyed frame transactions nor [EIP-8312] UTXO spends live there.

use ethrex_blockchain::payload::TransactionQueue;
use ethrex_common::types::{
    APPROVE_EXECUTION_AND_PAYMENT, EIP1559Transaction, FRAME_SIG_SCHEME_SECP256K1, Frame,
    FrameMode, FrameSignature, FrameTransaction, MempoolTransaction, Transaction, TxKind,
    utxo_vault,
};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use rustc_hash::FxHashMap;

fn addr(byte: u8) -> Address {
    Address::repeat_byte(byte)
}

fn eip1559_tx(nonce: u64) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1_000,
        gas_limit: 21_000,
        to: TxKind::Call(addr(0xff)),
        signature_r: U256::from(1),
        signature_s: U256::from(1),
        ..Default::default()
    })
}

fn frame_tx(sender: Address, nonce_keys: Vec<U256>, nonce_seq: u64) -> Transaction {
    Transaction::FrameTransaction(FrameTransaction {
        chain_id: 1,
        nonce_keys,
        nonce_seq,
        sender,
        frames: vec![Frame {
            mode: FrameMode::Verify as u8,
            flags: APPROVE_EXECUTION_AND_PAYMENT,
            target: Some(sender),
            gas_limit: 21_000,
            value: U256::zero(),
            data: Default::default(),
        }],
        signatures: vec![FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(sender),
            msg: Default::default(),
            signature: Default::default(),
        }],
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1_000,
        ..Default::default()
    })
}

fn queue(sender: Address, txs: Vec<Transaction>) -> TransactionQueue {
    let mut by_sender: FxHashMap<Address, Vec<MempoolTransaction>> = FxHashMap::default();
    by_sender.insert(
        sender,
        txs.into_iter()
            .map(|tx| MempoolTransaction::new(tx, sender))
            .collect(),
    );
    TransactionQueue::new(by_sender, Some(1)).expect("queue")
}

fn drain(queue: &mut TransactionQueue) -> Vec<H256> {
    let mut hashes = Vec::new();
    while let Some(head) = queue.peek() {
        hashes.push(head.tx.transaction().hash(&NativeCrypto));
        queue.shift().expect("shift");
    }
    hashes
}

#[test]
fn popping_a_linear_tx_drops_the_rest_of_the_sender_queue() {
    let sender = addr(0x01);
    let mut queue = queue(sender, vec![eip1559_tx(0), eip1559_tx(1), eip1559_tx(2)]);

    queue.pop().expect("pop");

    assert!(
        queue.is_empty(),
        "later nonces from the same sender cannot be included without the one that was dropped"
    );
}

#[test]
fn popping_a_keyed_frame_tx_keeps_its_independent_siblings() {
    let sender = addr(0x01);
    // Three keyed frame txs on disjoint key sets. The mempool admits at most one
    // pending transaction per key set, so none of them is ordered behind another.
    let txs = vec![
        frame_tx(sender, vec![U256::from(1u64)], 0),
        frame_tx(sender, vec![U256::from(2u64)], 0),
        frame_tx(sender, vec![U256::from(3u64)], 0),
    ];
    let survivors: Vec<H256> = txs[1..].iter().map(|tx| tx.hash(&NativeCrypto)).collect();
    let mut queue = queue(sender, txs);

    queue.pop().expect("pop");

    assert_eq!(drain(&mut queue), survivors);
}

#[test]
fn popping_a_utxo_spend_keeps_every_other_users_spend() {
    // EIP-8312: every UTXO spend shares the vault sender, so treating the queue
    // as one sender's nonce chain would let a single unusable spend evict the
    // whole network's privacy transactions from the block.
    let vault = utxo_vault();
    let txs = vec![
        frame_tx(vault, vec![U256::from(1u64)], 0),
        frame_tx(vault, vec![U256::from(2u64)], 0),
        frame_tx(vault, vec![U256::from(3u64)], 0),
    ];
    let survivors: Vec<H256> = txs[1..].iter().map(|tx| tx.hash(&NativeCrypto)).collect();
    let mut queue = queue(vault, txs);

    queue.pop().expect("pop");

    assert_eq!(drain(&mut queue), survivors);
}

#[test]
fn popping_a_key_zero_frame_tx_drops_the_rest_of_the_sender_queue() {
    let sender = addr(0x01);
    // `nonce_keys == [0]` is the linear domain: `nonce_seq` IS the account nonce,
    // so these are a nonce chain like any other.
    let mut queue = queue(
        sender,
        vec![
            frame_tx(sender, vec![U256::zero()], 0),
            frame_tx(sender, vec![U256::zero()], 1),
        ],
    );

    queue.pop().expect("pop");

    assert!(queue.is_empty());
}
