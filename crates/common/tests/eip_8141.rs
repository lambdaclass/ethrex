//! Unit tests for EIP-8141 Frame Transaction types.
//!
//! These tests cover RLP encode/decode round-trips, signature hash computation,
//! FrameMode encoding, TxType variant, and Transaction method behavior.
//!
//! Run with: `cargo test -p ethrex-common --features eip-8141`

#![cfg(feature = "eip-8141")]

use bytes::Bytes;
use ethereum_types::U256;
use ethrex_common::{
    types::transaction::{
        EIP8141Transaction, Frame, FrameMode, Transaction, TxType,
    },
    Address, H256,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

/// Helper to create a simple EIP-8141 transaction with the given frames.
fn make_frame_tx(frames: Vec<Frame>) -> EIP8141Transaction {
    EIP8141Transaction {
        chain_id: 1,
        nonce: 42,
        sender: Address::from_low_u64_be(0xABCD),
        frames,
        max_priority_fee_per_gas: U256::from(1_000_000_000u64),
        max_fee_per_gas: U256::from(50_000_000_000u64),
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: vec![],
    }
}

/// Helper to create a VERIFY frame with given data.
fn verify_frame(target: Address, data: Bytes) -> Frame {
    Frame {
        mode: FrameMode::Verify,
        target: Some(target),
        gas_limit: 100_000,
        data,
    }
}

/// Helper to create a SENDER frame.
fn sender_frame(target: Address, data: Bytes) -> Frame {
    Frame {
        mode: FrameMode::Sender,
        target: Some(target),
        gas_limit: 200_000,
        data,
    }
}

/// Helper to create a DEFAULT frame.
fn default_frame(target: Address, data: Bytes) -> Frame {
    Frame {
        mode: FrameMode::Default,
        target: Some(target),
        gas_limit: 50_000,
        data,
    }
}

// ========================================================================
// T7.1: RLP round-trip for EIP8141Transaction
// ========================================================================

#[test]
fn eip8141_tx_rlp_round_trip() {
    let validator = Address::from_low_u64_be(0x1111);
    let recipient = Address::from_low_u64_be(0x2222);

    let tx = make_frame_tx(vec![
        verify_frame(validator, Bytes::from_static(b"verify_data")),
        sender_frame(recipient, Bytes::from_static(b"transfer")),
    ]);

    let mut buf = Vec::new();
    tx.encode(&mut buf);

    let decoded = EIP8141Transaction::decode(&buf).expect("RLP decode failed");
    assert_eq!(decoded.chain_id, tx.chain_id);
    assert_eq!(decoded.nonce, tx.nonce);
    assert_eq!(decoded.sender, tx.sender);
    assert_eq!(decoded.frames.len(), tx.frames.len());
    assert_eq!(decoded.max_priority_fee_per_gas, tx.max_priority_fee_per_gas);
    assert_eq!(decoded.max_fee_per_gas, tx.max_fee_per_gas);
    assert_eq!(decoded.max_fee_per_blob_gas, tx.max_fee_per_blob_gas);
    assert_eq!(decoded.blob_versioned_hashes, tx.blob_versioned_hashes);

    // Check individual frames
    for (original, decoded_frame) in tx.frames.iter().zip(decoded.frames.iter()) {
        assert_eq!(original.mode, decoded_frame.mode);
        assert_eq!(original.target, decoded_frame.target);
        assert_eq!(original.gas_limit, decoded_frame.gas_limit);
        assert_eq!(original.data, decoded_frame.data);
    }
}

#[test]
fn eip8141_tx_rlp_round_trip_empty_frames() {
    let tx = make_frame_tx(vec![]);
    let mut buf = Vec::new();
    tx.encode(&mut buf);
    let decoded = EIP8141Transaction::decode(&buf).expect("RLP decode failed");
    assert_eq!(decoded.frames.len(), 0);
}

#[test]
fn eip8141_tx_rlp_round_trip_with_blob_hashes() {
    let mut tx = make_frame_tx(vec![default_frame(
        Address::from_low_u64_be(0x3333),
        Bytes::new(),
    )]);
    tx.blob_versioned_hashes = vec![H256::from_low_u64_be(1), H256::from_low_u64_be(2)];
    tx.max_fee_per_blob_gas = U256::from(100);

    let mut buf = Vec::new();
    tx.encode(&mut buf);
    let decoded = EIP8141Transaction::decode(&buf).expect("RLP decode failed");
    assert_eq!(decoded.blob_versioned_hashes.len(), 2);
    assert_eq!(decoded.max_fee_per_blob_gas, U256::from(100));
}

#[test]
fn eip8141_tx_rlp_round_trip_none_target() {
    let frame = Frame {
        mode: FrameMode::Default,
        target: None,
        gas_limit: 10_000,
        data: Bytes::from_static(b"create"),
    };
    let tx = make_frame_tx(vec![frame]);

    let mut buf = Vec::new();
    tx.encode(&mut buf);
    let decoded = EIP8141Transaction::decode(&buf).expect("RLP decode failed");
    assert!(decoded.frames[0].target.is_none());
}

// ========================================================================
// T7.2: compute_sig_hash zeroes VERIFY frame data
// ========================================================================

#[test]
fn compute_sig_hash_zeroes_verify_data() {
    let validator = Address::from_low_u64_be(0x1111);
    let recipient = Address::from_low_u64_be(0x2222);

    let tx_with_verify_data = make_frame_tx(vec![
        verify_frame(validator, Bytes::from_static(b"secret_signature_data")),
        sender_frame(recipient, Bytes::from_static(b"transfer")),
    ]);

    let tx_with_empty_verify = make_frame_tx(vec![
        verify_frame(validator, Bytes::new()),
        sender_frame(recipient, Bytes::from_static(b"transfer")),
    ]);

    // Both should produce the same sig_hash because VERIFY data is zeroed
    let hash1 = tx_with_verify_data.compute_sig_hash();
    let hash2 = tx_with_empty_verify.compute_sig_hash();
    assert_eq!(hash1, hash2, "sig_hash should be identical regardless of VERIFY frame data");
}

#[test]
fn compute_sig_hash_preserves_non_verify_data() {
    let validator = Address::from_low_u64_be(0x1111);
    let recipient = Address::from_low_u64_be(0x2222);

    let tx1 = make_frame_tx(vec![
        verify_frame(validator, Bytes::from_static(b"sig")),
        sender_frame(recipient, Bytes::from_static(b"transfer_A")),
    ]);

    let tx2 = make_frame_tx(vec![
        verify_frame(validator, Bytes::from_static(b"sig")),
        sender_frame(recipient, Bytes::from_static(b"transfer_B")),
    ]);

    // Different SENDER data should produce different sig_hash
    let hash1 = tx1.compute_sig_hash();
    let hash2 = tx2.compute_sig_hash();
    assert_ne!(hash1, hash2, "sig_hash should differ when non-VERIFY frame data differs");
}

#[test]
fn compute_sig_hash_deterministic() {
    let tx = make_frame_tx(vec![
        verify_frame(Address::from_low_u64_be(0x1111), Bytes::from_static(b"data")),
        default_frame(Address::from_low_u64_be(0x2222), Bytes::new()),
    ]);

    let hash1 = tx.compute_sig_hash();
    let hash2 = tx.compute_sig_hash();
    assert_eq!(hash1, hash2, "sig_hash must be deterministic");
}

// ========================================================================
// T7.3: FrameMode RLP encode/decode
// ========================================================================

#[test]
fn frame_mode_rlp_round_trip() {
    for mode in [FrameMode::Default, FrameMode::Verify, FrameMode::Sender] {
        let mut buf = Vec::new();
        mode.encode(&mut buf);
        let decoded = FrameMode::decode(&buf).expect("FrameMode decode failed");
        assert_eq!(mode, decoded);
    }
}

#[test]
fn frame_rlp_round_trip() {
    let frame = Frame {
        mode: FrameMode::Verify,
        target: Some(Address::from_low_u64_be(0xCAFE)),
        gas_limit: 999_999,
        data: Bytes::from_static(b"frame_payload"),
    };

    let mut buf = Vec::new();
    frame.encode(&mut buf);
    let decoded = Frame::decode(&buf).expect("Frame decode failed");
    assert_eq!(frame.mode, decoded.mode);
    assert_eq!(frame.target, decoded.target);
    assert_eq!(frame.gas_limit, decoded.gas_limit);
    assert_eq!(frame.data, decoded.data);
}

// ========================================================================
// T7.4: TxType::EIP8141 -> u8 (0x06)
// ========================================================================

#[test]
fn tx_type_eip8141_to_u8() {
    let ty = TxType::EIP8141;
    let val: u8 = ty.into();
    assert_eq!(val, 0x06);
}

#[test]
fn tx_type_eip8141_display() {
    let ty = TxType::EIP8141;
    let s = format!("{ty}");
    // Should contain some identifier for EIP-8141
    assert!(
        s.contains("8141") || s.contains("Frame") || s.contains("0x06") || s.contains("06"),
        "TxType::EIP8141 display should mention '8141', 'Frame', or '0x06', got: {s}"
    );
}

// ========================================================================
// T7.5: Transaction variant methods (tx_type, gas_limit, nonce, etc.)
// ========================================================================

#[test]
fn transaction_variant_tx_type() {
    let tx = Transaction::EIP8141Transaction(make_frame_tx(vec![
        default_frame(Address::from_low_u64_be(1), Bytes::new()),
    ]));
    assert_eq!(tx.tx_type(), TxType::EIP8141);
}

#[test]
fn transaction_variant_nonce() {
    let inner = make_frame_tx(vec![]);
    let tx = Transaction::EIP8141Transaction(inner.clone());
    assert_eq!(tx.nonce(), inner.nonce);
}

#[test]
fn transaction_variant_gas_limit() {
    let tx = Transaction::EIP8141Transaction(make_frame_tx(vec![
        default_frame(Address::from_low_u64_be(1), Bytes::new()),
        sender_frame(Address::from_low_u64_be(2), Bytes::new()),
    ]));
    // gas_limit should be the sum of all frame gas limits
    let expected = 50_000 + 200_000;
    assert_eq!(tx.gas_limit(), expected);
}

#[test]
fn transaction_variant_max_fee_per_gas() {
    let inner = make_frame_tx(vec![]);
    let tx = Transaction::EIP8141Transaction(inner.clone());
    assert_eq!(tx.max_fee_per_gas(), Some(inner.max_fee_per_gas.as_u64()));
}

#[test]
fn transaction_variant_max_priority_fee() {
    let inner = make_frame_tx(vec![]);
    let tx = Transaction::EIP8141Transaction(inner.clone());
    assert_eq!(tx.max_priority_fee(), Some(inner.max_priority_fee_per_gas.as_u64()));
}

#[test]
fn transaction_variant_access_list_empty() {
    let tx = Transaction::EIP8141Transaction(make_frame_tx(vec![]));
    // Frame transactions don't have access lists
    assert!(tx.access_list().is_empty());
}

#[test]
fn transaction_variant_blob_versioned_hashes() {
    let mut inner = make_frame_tx(vec![]);
    inner.blob_versioned_hashes = vec![H256::from_low_u64_be(99)];
    let tx = Transaction::EIP8141Transaction(inner);
    assert_eq!(tx.blob_versioned_hashes().len(), 1);
}
