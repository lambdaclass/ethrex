//! A frame receipt must survive the block-data flush to disk.
//!
//! `RECEIPTS_V2` holds the internal storage codec (`Receipt::encode_storage` /
//! `Receipt::decode_storage`), which for an EIP-8141 frame receipt carries
//! `succeeded` and the aggregated top-level logs that the consensus layout
//! deliberately omits. Writing the consensus layout into that table instead
//! leaves the row undecodable, and only for frame receipts: the two codecs
//! coincide for every other transaction type.
//!
//! Reads are served from the in-memory buffer until the block flushes, so these
//! assertions only bite after `flush_block_data`.

use ethrex_common::{
    Address, H256,
    types::{
        Block, BlockBody, BlockHeader, FRAME_RECEIPT_STATUS_SKIPPED, FRAME_RECEIPT_STATUS_SUCCESS,
        FrameReceipt, Log, Receipt, TxType,
    },
};
use ethrex_storage::{EngineType, Store};

fn log_at(address: Address) -> Log {
    Log {
        address,
        topics: vec![H256::from_low_u64_be(0xbeef)],
        data: vec![0x01, 0x02, 0x03].into(),
    }
}

/// A frame receipt whose every field distinguishes the storage codec from the
/// consensus one: `succeeded` and the top-level `logs` exist only in storage.
fn frame_receipt() -> Receipt {
    let payer = Address::from_low_u64_be(0x8141);
    Receipt {
        tx_type: TxType::Frame,
        succeeded: true,
        cumulative_gas_used: 0x52fe,
        logs: vec![log_at(Address::from_low_u64_be(0xfffe))],
        payer: Some(payer),
        frame_receipts: Some(vec![
            FrameReceipt {
                status: FRAME_RECEIPT_STATUS_SUCCESS,
                gas_used: 0x1234,
                state_gas_used: 0,
                logs: vec![log_at(Address::from_low_u64_be(0xaaaa))],
            },
            FrameReceipt {
                status: FRAME_RECEIPT_STATUS_SUCCESS,
                gas_used: 0,
                state_gas_used: 0,
                logs: vec![],
            },
        ]),
    }
}

fn block_at(number: u64) -> Block {
    Block::new(
        BlockHeader {
            number,
            ..Default::default()
        },
        BlockBody::default(),
    )
}

#[tokio::test]
async fn frame_receipt_survives_the_flush_to_disk() {
    let store = Store::new("", EngineType::InMemory).expect("store");
    let block = block_at(89);
    let hash = block.hash();
    let receipt = frame_receipt();

    store.buffer_block_with_receipts_for_test(&block, vec![receipt.clone()]);
    store.flush_block_data_for_test().expect("flush");

    // Reading this back is what fails when the write side uses the consensus
    // codec: `cumulative_gas_used` lands where `succeeded` is expected and the
    // decode aborts with a malformed-boolean error rather than returning a row.
    let stored = store
        .get_receipt_by_hash_for_test(hash, 0)
        .await
        .expect("decode the flushed frame receipt")
        .expect("receipt present after flush");

    assert_eq!(stored, receipt, "frame receipt changed across the flush");
}

/// The fields the consensus layout drops are exactly the ones worth asserting
/// individually, so a partial regression cannot hide behind the equality check
/// above.
#[tokio::test]
async fn flushed_frame_receipt_keeps_the_storage_only_fields() {
    let store = Store::new("", EngineType::InMemory).expect("store");
    let block = block_at(7);
    let hash = block.hash();

    // `succeeded: false` with all-SUCCESS frames cannot be re-derived from the
    // frame statuses, so it can only survive if the row really carried it.
    let mut receipt = frame_receipt();
    receipt.succeeded = false;

    store.buffer_block_with_receipts_for_test(&block, vec![receipt.clone()]);
    store.flush_block_data_for_test().expect("flush");

    let stored = store
        .get_receipt_by_hash_for_test(hash, 0)
        .await
        .expect("decode")
        .expect("present");

    assert!(!stored.succeeded, "succeeded was re-derived, not persisted");
    assert_eq!(stored.logs, receipt.logs, "aggregated logs were dropped");
    assert_eq!(stored.payer, receipt.payer, "payer was dropped");
    assert_eq!(
        stored.frame_receipts.as_ref().map(Vec::len),
        Some(2),
        "per-frame receipts were dropped"
    );
}

/// A skipped frame must round-trip as skipped rather than collapsing into a
/// plain failure.
#[tokio::test]
async fn flushed_frame_receipt_preserves_a_skipped_frame() {
    let store = Store::new("", EngineType::InMemory).expect("store");
    let block = block_at(12);
    let hash = block.hash();

    let mut receipt = frame_receipt();
    receipt.succeeded = false;
    receipt.frame_receipts = Some(vec![FrameReceipt {
        status: FRAME_RECEIPT_STATUS_SKIPPED,
        gas_used: 0,
        state_gas_used: 0,
        logs: vec![],
    }]);

    store.buffer_block_with_receipts_for_test(&block, vec![receipt.clone()]);
    store.flush_block_data_for_test().expect("flush");

    let stored = store
        .get_receipt_by_hash_for_test(hash, 0)
        .await
        .expect("decode")
        .expect("present");

    assert_eq!(
        stored.frame_receipts.expect("frames")[0].status,
        FRAME_RECEIPT_STATUS_SKIPPED
    );
}

/// The non-frame path shares the two codecs, so it must keep working unchanged.
#[tokio::test]
async fn non_frame_receipt_survives_the_flush_to_disk() {
    let store = Store::new("", EngineType::InMemory).expect("store");
    let block = block_at(3);
    let hash = block.hash();
    let receipt = Receipt {
        tx_type: TxType::EIP1559,
        succeeded: true,
        cumulative_gas_used: 21_000,
        logs: vec![log_at(Address::from_low_u64_be(0x1559))],
        payer: None,
        frame_receipts: None,
    };

    store.buffer_block_with_receipts_for_test(&block, vec![receipt.clone()]);
    store.flush_block_data_for_test().expect("flush");

    let stored = store
        .get_receipt_by_hash_for_test(hash, 0)
        .await
        .expect("decode")
        .expect("present");

    assert_eq!(stored, receipt);
}
