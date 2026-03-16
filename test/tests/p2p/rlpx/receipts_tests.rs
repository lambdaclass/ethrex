use bytes::Bytes;
use ethrex_common::{
    Address, H256,
    types::{BlockHash, Log, Receipt, TxType},
};
use ethrex_p2p::rlpx::{
    eth::receipts::{
        GetReceipts68, GetReceipts69, GetReceipts70, Receipts68, Receipts69, Receipts70,
    },
    message::RLPxMessage,
};

fn make_receipt(gas: u64, num_logs: usize) -> Receipt {
    let logs: Vec<Log> = (0..num_logs)
        .map(|i| Log {
            address: Address::from_low_u64_be(i as u64),
            topics: vec![H256::from_low_u64_be(i as u64)],
            data: Bytes::from(vec![0xab; 32]),
        })
        .collect();
    Receipt::new(TxType::EIP1559, true, gas, logs)
}

// ── GetReceipts68 / GetReceipts69 (legacy, eth/68-69) ──

#[test]
fn get_receipts68_empty_message() {
    let blocks_hash = vec![];
    let get_receipts = GetReceipts68::new(1, blocks_hash.clone());

    let mut buf = Vec::new();
    get_receipts.encode(&mut buf).unwrap();

    let decoded = GetReceipts68::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert_eq!(decoded.block_hashes, blocks_hash);
}

#[test]
fn get_receipts69_not_empty_message() {
    let blocks_hash = vec![
        BlockHash::from([0; 32]),
        BlockHash::from([1; 32]),
        BlockHash::from([2; 32]),
    ];
    let get_receipts = GetReceipts69::new(1, blocks_hash.clone());

    let mut buf = Vec::new();
    get_receipts.encode(&mut buf).unwrap();

    let decoded = GetReceipts69::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert_eq!(decoded.block_hashes, blocks_hash);
}

// ── Receipts68 ──

#[test]
fn receipts68_empty_message() {
    let receipts = vec![];
    let receipts = Receipts68::new(1, receipts);

    let mut buf = Vec::new();
    receipts.encode(&mut buf).unwrap();

    let decoded = Receipts68::decode(&buf).unwrap();

    assert_eq!(decoded.get_id(), 1);
    assert_eq!(decoded.get_receipts(), Vec::<Vec<Receipt>>::new());
}

// ── Receipts69 ──

#[test]
fn receipts69_empty_message() {
    let msg = Receipts69::new(1, vec![]);
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();
    let decoded = Receipts69::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert!(decoded.receipts.is_empty());
}

#[test]
fn receipts69_with_receipts() {
    let receipts = vec![vec![make_receipt(21000, 1), make_receipt(42000, 2)]];
    let msg = Receipts69::new(5, receipts.clone());
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();
    let decoded = Receipts69::decode(&buf).unwrap();
    assert_eq!(decoded.id, 5);
    assert_eq!(decoded.receipts, receipts);
}

// ── GetReceipts70 (eth/70) ──

#[test]
fn get_receipts70_empty() {
    let msg = GetReceipts70::new(1, 0, vec![]);
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();
    let decoded = GetReceipts70::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert_eq!(decoded.first_block_receipt_index, 0);
    assert!(decoded.block_hashes.is_empty());
}

#[test]
fn get_receipts70_with_index_and_hashes() {
    let hashes = vec![BlockHash::from([1; 32]), BlockHash::from([2; 32])];
    let msg = GetReceipts70::new(42, 5, hashes.clone());
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();
    let decoded = GetReceipts70::decode(&buf).unwrap();
    assert_eq!(decoded.id, 42);
    assert_eq!(decoded.first_block_receipt_index, 5);
    assert_eq!(decoded.block_hashes, hashes);
}

// ── Receipts70 (eth/70) ──

#[test]
fn receipts70_empty_complete() {
    let msg = Receipts70::new(1, false, vec![]);
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();
    let decoded = Receipts70::decode(&buf).unwrap();
    assert_eq!(decoded.id, 1);
    assert!(!decoded.last_block_incomplete);
    assert!(decoded.receipts.is_empty());
}

#[test]
fn receipts70_with_receipts_complete() {
    let receipts = vec![
        vec![make_receipt(21000, 1), make_receipt(42000, 2)],
        vec![make_receipt(100000, 0)],
    ];
    let msg = Receipts70::new(10, false, receipts.clone());
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();
    let decoded = Receipts70::decode(&buf).unwrap();
    assert_eq!(decoded.id, 10);
    assert!(!decoded.last_block_incomplete);
    assert_eq!(decoded.receipts, receipts);
}

#[test]
fn receipts70_with_receipts_incomplete() {
    let receipts = vec![vec![make_receipt(21000, 3)]];
    let msg = Receipts70::new(7, true, receipts.clone());
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();
    let decoded = Receipts70::decode(&buf).unwrap();
    assert_eq!(decoded.id, 7);
    assert!(decoded.last_block_incomplete);
    assert_eq!(decoded.receipts, receipts);
}

// ── Wire format compatibility ──

#[test]
fn get_receipts_and_get_receipts70_have_same_message_code() {
    assert_eq!(GetReceipts68::CODE, GetReceipts69::CODE);
    assert_eq!(GetReceipts69::CODE, GetReceipts70::CODE);
}

#[test]
fn receipts_versions_have_same_message_code() {
    assert_eq!(Receipts68::CODE, Receipts69::CODE);
    assert_eq!(Receipts69::CODE, Receipts70::CODE);
}

#[test]
fn get_receipts70_wire_format_differs_from_legacy() {
    let hashes = vec![BlockHash::from([1; 32])];
    let legacy = GetReceipts69::new(1, hashes.clone());
    let v70 = GetReceipts70::new(1, 0, hashes);

    let mut buf_legacy = Vec::new();
    legacy.encode(&mut buf_legacy).unwrap();
    let mut buf_v70 = Vec::new();
    v70.encode(&mut buf_v70).unwrap();

    assert_ne!(buf_legacy, buf_v70);
}

#[test]
fn receipts70_wire_format_differs_from_69() {
    let receipts = vec![vec![make_receipt(21000, 0)]];
    let v69 = Receipts69::new(1, receipts.clone());
    let v70 = Receipts70::new(1, false, receipts);

    let mut buf_69 = Vec::new();
    v69.encode(&mut buf_69).unwrap();
    let mut buf_70 = Vec::new();
    v70.encode(&mut buf_70).unwrap();

    assert_ne!(buf_69, buf_70);
}

// ── Cross-decode rejection ──

#[test]
fn get_receipts70_cannot_decode_as_legacy() {
    let msg = GetReceipts70::new(1, 5, vec![BlockHash::from([1; 32])]);
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();

    let result = GetReceipts68::decode(&buf);
    assert!(result.is_err());
}

#[test]
fn receipts70_cannot_decode_as_69() {
    let receipts = vec![vec![make_receipt(21000, 1)]];
    let msg = Receipts70::new(1, false, receipts);
    let mut buf = Vec::new();
    msg.encode(&mut buf).unwrap();

    let result = Receipts69::decode(&buf);
    assert!(result.is_err());
}
