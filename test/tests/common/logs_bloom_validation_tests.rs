//! Regression tests for `validate_receipts_root_and_logs_bloom` (the `header-logs-bloom-skip`
//! finding). The receipts root commits to each receipt's per-receipt bloom but *not* to the
//! header's aggregate `logs_bloom`, so the aggregate is validated separately. These tests use
//! receipts that actually carry logs (so the bloom is non-trivial) and pin three things:
//!
//! 1. a matching header passes,
//! 2. a wrong `logs_bloom` is rejected with `LogsBloomMismatch`,
//! 3. a wrong `receipts_root` is still rejected with `ReceiptsRootMismatch` — i.e. folding the
//!    two checks into one pass did not weaken the receipts-root check.
//!
//! It also asserts the single-pass `compute_receipts_root_and_logs_bloom` yields the same root
//! as the standalone `compute_receipts_root`, guarding the dedup refactor.
use ethrex_common::errors::InvalidBlockError;
use ethrex_common::types::{
    BlockHeader, Log, Receipt, TxType, compute_receipts_root, compute_receipts_root_and_logs_bloom,
};
use ethrex_common::{Address, H256, validate_receipts_root_and_logs_bloom};
use ethrex_crypto::NativeCrypto;

fn receipts_with_logs() -> Vec<Receipt> {
    let log = Log {
        address: Address::repeat_byte(0xab),
        topics: vec![H256::repeat_byte(0x11), H256::repeat_byte(0x22)],
        data: vec![1, 2, 3].into(),
    };
    vec![
        Receipt::new(TxType::Legacy, true, 21_000, vec![log.clone()]),
        Receipt::new(TxType::EIP1559, true, 42_000, vec![log]),
    ]
}

fn header_for(receipts: &[Receipt]) -> BlockHeader {
    let (receipts_root, logs_bloom) = compute_receipts_root_and_logs_bloom(receipts, &NativeCrypto);
    BlockHeader {
        receipts_root,
        logs_bloom,
        ..Default::default()
    }
}

#[test]
fn single_pass_root_matches_standalone() {
    // The dedup must not change the receipts root that the standalone path produces.
    let receipts = receipts_with_logs();
    let (root, _) = compute_receipts_root_and_logs_bloom(&receipts, &NativeCrypto);
    assert_eq!(root, compute_receipts_root(&receipts, &NativeCrypto));
}

#[test]
fn accepts_matching_root_and_bloom() {
    let receipts = receipts_with_logs();
    let header = header_for(&receipts);
    validate_receipts_root_and_logs_bloom(&header, &receipts, &NativeCrypto)
        .expect("matching root and bloom must validate");
}

#[test]
fn rejects_wrong_logs_bloom() {
    let receipts = receipts_with_logs();
    let mut header = header_for(&receipts);
    // Flip a single bit in the aggregate bloom — receipts root stays correct.
    header.logs_bloom.0[0] ^= 0x01;
    let err = validate_receipts_root_and_logs_bloom(&header, &receipts, &NativeCrypto)
        .expect_err("a mismatched logs_bloom must be rejected");
    assert!(
        matches!(err, InvalidBlockError::LogsBloomMismatch),
        "expected LogsBloomMismatch, got {err:?}"
    );
}

#[test]
fn rejects_wrong_receipts_root() {
    let receipts = receipts_with_logs();
    let mut header = header_for(&receipts);
    header.receipts_root = H256::repeat_byte(0xff);
    let err = validate_receipts_root_and_logs_bloom(&header, &receipts, &NativeCrypto)
        .expect_err("a mismatched receipts_root must be rejected");
    assert!(
        matches!(err, InvalidBlockError::ReceiptsRootMismatch),
        "expected ReceiptsRootMismatch, got {err:?}"
    );
}
