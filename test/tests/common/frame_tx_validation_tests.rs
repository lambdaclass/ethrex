//! EIP-8141 frame-transaction validation tests (migrated from inline modules).
//!
//! Covers:
//!   - Blob-gas accounting for frame transactions (migrated from `crates/common/validation.rs`).
//!   - Validation-prefix recognition and structural validation (Phase 1, task 1.7,
//!     migrated from `crates/common/types/transaction.rs`).

use bytes::Bytes;
use ethrex_common::constants::GAS_PER_BLOB;
use ethrex_common::types::BATCH_SIZE;
use ethrex_common::types::{
    APPROVE_EXECUTION, APPROVE_EXECUTION_AND_PAYMENT, APPROVE_PAYMENT, BATCH_PATH_LEN, Block,
    BlockBody, BlockHeader, ChainConfig, EIP4844Transaction, FRAME_SIG_SCHEME_ARBITRARY,
    FRAME_SIG_SCHEME_SECP256K1, FRAME_TX_MAX_VERIFY_GAS, Frame, FrameMode, FrameSignature,
    FrameTransaction, FrameValidationError, MAX_SIBLINGS, P2PTransaction, PrefixShape, RING_SIZE,
    SLOT_NEXT_INDEX, SLOT_RING_BASE, Spend, SpendInput, SpendOutput, Transaction,
    WrappedFrameTransaction, batch_slot, batch_slot_for_block, fold, frame_tx_expiry_verifier,
    hash_pair, is_spent, merkle_proof, merkle_root, opening_leaf, ring_slot, seals_batch,
    slot_batch_base, slot_spent_base, spent_bit_location, utxo_vault,
};
use ethrex_common::types::{BlobsBundle, Fork, MAX_BLOBS_PER_TX, TxType};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;

/// EIP-4844 `VERSIONED_HASH_VERSION_KZG`. The constant itself lives in a private
/// module of `ethrex-common`, so it is restated here.
const VERSIONED_HASH_VERSION_KZG: u8 = 0x01;
use ethrex_common::validation::verify_blob_gas_usage;
use ethrex_common::{Address, H256, U256};

// ---------------------------------------------------------------------------
// Helpers shared by blob-gas and prefix tests
// ---------------------------------------------------------------------------

/// Minimal cancun-active ChainConfig: only cancun_time set (= 0), default
/// blob schedule (max = 6 blobs per block).
fn cancun_config() -> ChainConfig {
    ChainConfig {
        cancun_time: Some(0),
        ..Default::default()
    }
}

/// A minimal FrameTransaction with the given number of blob versioned hashes.
fn frame_tx_with_blobs(n_blobs: usize) -> FrameTransaction {
    FrameTransaction {
        chain_id: 0,
        nonce_keys: vec![U256::zero()],
        nonce_seq: 0,
        sender: Default::default(),
        frames: vec![Frame {
            mode: FrameMode::Default as u8,
            flags: 0x00,
            target: None,
            gas_limit: 0,
            state_limit: 0,
            value: Default::default(),
            data: Bytes::new(),
        }],
        signatures: vec![],
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        max_fee_per_blob_gas: Default::default(),
        blob_versioned_hashes: (0..n_blobs).map(|_| H256::zero()).collect(),
        ..Default::default()
    }
}

/// Build a minimal Block with the given transactions and blob_gas_used header
/// value. timestamp = 1 so cancun_time = 0 is active.
fn make_block(transactions: Vec<Transaction>, blob_gas_used: u64) -> Block {
    Block {
        header: BlockHeader {
            timestamp: 1,
            gas_limit: 30_000_000,
            blob_gas_used: Some(blob_gas_used),
            excess_blob_gas: Some(0),
            ..Default::default()
        },
        body: BlockBody {
            transactions,
            ommers: vec![],
            withdrawals: Some(vec![]),
        },
    }
}

// ---------------------------------------------------------------------------
// EIP-8141 frame tx blob gas accounting
// ---------------------------------------------------------------------------

#[test]
fn frame_tx_blob_gas_counts_correctly() {
    let config = cancun_config();
    let tx = Transaction::FrameTransaction(frame_tx_with_blobs(2));
    let block = make_block(vec![tx], 2 * GAS_PER_BLOB as u64);
    assert!(verify_blob_gas_usage(&block, &config).is_ok());
}

#[test]
fn frame_tx_blob_gas_mismatch_fails() {
    use ethrex_common::errors::InvalidBlockError;
    let config = cancun_config();
    let tx = Transaction::FrameTransaction(frame_tx_with_blobs(2));
    // Header claims 0 but actual is 2 * GAS_PER_BLOB
    let block = make_block(vec![tx], 0);
    assert!(matches!(
        verify_blob_gas_usage(&block, &config),
        Err(InvalidBlockError::BlobGasUsedMismatch)
    ));
}

#[test]
fn mixed_eip4844_and_frame_tx_blobs_counted_together() {
    let config = cancun_config();
    let eip4844_tx = Transaction::EIP4844Transaction(EIP4844Transaction {
        blob_versioned_hashes: vec![H256::zero()],
        ..Default::default()
    });
    let frame_tx = Transaction::FrameTransaction(frame_tx_with_blobs(2));
    let expected_gas = 3 * GAS_PER_BLOB as u64; // 1 EIP-4844 + 2 frame
    let block = make_block(vec![eip4844_tx, frame_tx], expected_gas);
    assert!(verify_blob_gas_usage(&block, &config).is_ok());
}

// ---------------------------------------------------------------------------
// EIP-8141 validation-prefix recognition and structural validation (task 1.7)
// ---------------------------------------------------------------------------

fn sender_addr() -> Address {
    Address::from_low_u64_be(0xABCD)
}

fn expiry_verifier_frame() -> Frame {
    Frame {
        mode: FrameMode::Verify as u8,
        flags: 0x00,
        target: Some(frame_tx_expiry_verifier()),
        gas_limit: 1_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::from(vec![0u8; 8]),
    }
}

fn self_verify_frame() -> Frame {
    Frame {
        mode: FrameMode::Verify as u8,
        flags: APPROVE_EXECUTION_AND_PAYMENT,
        target: Some(sender_addr()),
        gas_limit: 10_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

fn only_verify_frame() -> Frame {
    Frame {
        mode: FrameMode::Verify as u8,
        flags: APPROVE_EXECUTION,
        target: Some(sender_addr()),
        gas_limit: 10_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

fn pay_frame() -> Frame {
    Frame {
        mode: FrameMode::Verify as u8,
        flags: APPROVE_PAYMENT,
        target: Some(sender_addr()),
        gas_limit: 10_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

fn deploy_frame() -> Frame {
    Frame {
        mode: FrameMode::Default as u8,
        flags: 0x00,
        target: None,
        gas_limit: 50_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::from_static(b"deploy_bytecode"),
    }
}

fn base_frame_tx_with_frames(frames: Vec<Frame>) -> FrameTransaction {
    FrameTransaction {
        sender: sender_addr(),
        frames,
        chain_id: 1,
        nonce_keys: vec![U256::zero()],
        nonce_seq: 42,
        max_priority_fee_per_gas: 1_000_000_000,
        max_fee_per_gas: 30_000_000_000,
        ..Default::default()
    }
}

// --- Passing shape tests ---

#[test]
fn prefix_shape_self_verify() {
    let tx = base_frame_tx_with_frames(vec![self_verify_frame()]);
    let prefix = tx.validation_prefix().expect("should recognize SelfVerify");
    assert_eq!(prefix.shape, PrefixShape::SelfVerify);
    assert_eq!(prefix.frame_indices, vec![0]);
    assert_eq!(prefix.deploy_index, None);
    assert_eq!(prefix.pay_index, Some(0));
    tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
        .expect("SelfVerify structure should be valid");
}

#[test]
fn prefix_shape_deploy_self_verify() {
    let tx = base_frame_tx_with_frames(vec![deploy_frame(), self_verify_frame()]);
    let prefix = tx
        .validation_prefix()
        .expect("should recognize DeploySelfVerify");
    assert_eq!(prefix.shape, PrefixShape::DeploySelfVerify);
    assert_eq!(prefix.frame_indices, vec![0, 1]);
    assert_eq!(prefix.deploy_index, Some(0));
    assert_eq!(prefix.pay_index, Some(1));
    tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
        .expect("DeploySelfVerify structure should be valid");
}

#[test]
fn prefix_shape_only_verify_pay() {
    let tx = base_frame_tx_with_frames(vec![only_verify_frame(), pay_frame()]);
    let prefix = tx
        .validation_prefix()
        .expect("should recognize OnlyVerifyPay");
    assert_eq!(prefix.shape, PrefixShape::OnlyVerifyPay);
    assert_eq!(prefix.frame_indices, vec![0, 1]);
    assert_eq!(prefix.deploy_index, None);
    assert_eq!(prefix.pay_index, Some(1));
    tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
        .expect("OnlyVerifyPay structure should be valid");
}

#[test]
fn prefix_shape_deploy_only_verify_pay() {
    let tx = base_frame_tx_with_frames(vec![deploy_frame(), only_verify_frame(), pay_frame()]);
    let prefix = tx
        .validation_prefix()
        .expect("should recognize DeployOnlyVerifyPay");
    assert_eq!(prefix.shape, PrefixShape::DeployOnlyVerifyPay);
    assert_eq!(prefix.frame_indices, vec![0, 1, 2]);
    assert_eq!(prefix.deploy_index, Some(0));
    assert_eq!(prefix.pay_index, Some(2));
    tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
        .expect("DeployOnlyVerifyPay structure should be valid");
}

#[test]
fn prefix_shape_self_verify_with_interleaved_expiry_verifier() {
    // Expiry-verifier frames are transparent: they are skipped during
    // shape matching. The prefix should still be recognized as SelfVerify.
    let tx = base_frame_tx_with_frames(vec![expiry_verifier_frame(), self_verify_frame()]);
    let prefix = tx
        .validation_prefix()
        .expect("should recognize SelfVerify with leading expiry-verifier");
    assert_eq!(prefix.shape, PrefixShape::SelfVerify);
    // frame_indices omits the expiry-verifier (index 0); self_verify is at index 1.
    assert_eq!(prefix.frame_indices, vec![1]);
    tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
        .expect("SelfVerify with expiry-verifier should be structurally valid");
}

#[test]
fn prefix_shape_deploy_self_verify_with_expiry_verifier_between() {
    // An expiry verifier between deploy and self-verify is transparent to shape
    // matching, but the frame itself is misplaced: it may only be the first frame.
    let tx = base_frame_tx_with_frames(vec![
        deploy_frame(),
        expiry_verifier_frame(),
        self_verify_frame(),
    ]);
    let prefix = tx
        .validation_prefix()
        .expect("should recognize DeploySelfVerify with interleaved expiry-verifier");
    assert_eq!(prefix.shape, PrefixShape::DeploySelfVerify);
    assert_eq!(prefix.frame_indices, vec![0, 2]);
    assert_eq!(prefix.deploy_index, Some(0));
    assert_eq!(prefix.pay_index, Some(2));
    assert_eq!(
        tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
            .unwrap_err(),
        FrameValidationError::ExpiryFrameNotFirst { frame_index: 1 }
    );
}

#[test]
fn prefix_shape_deploy_self_verify_with_leading_expiry_verifier() {
    // Expiry-verifier frame at raw index 0 means the deploy frame has raw
    // index 1. `validate_prefix_structure` must not reject this with
    // `DeployNotFirst` — the deploy IS first among non-expiry frames.
    let tx = base_frame_tx_with_frames(vec![
        expiry_verifier_frame(), // raw index 0 — skipped by shape matching
        deploy_frame(),          // raw index 1 — first non-expiry frame
        self_verify_frame(),     // raw index 2
    ]);
    let prefix = tx
        .validation_prefix()
        .expect("should recognize DeploySelfVerify with leading expiry-verifier");
    assert_eq!(prefix.shape, PrefixShape::DeploySelfVerify);
    assert_eq!(prefix.frame_indices, vec![1, 2]);
    assert_eq!(prefix.deploy_index, Some(1));
    assert_eq!(prefix.pay_index, Some(2));
    tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
        .expect("DeploySelfVerify with raw-index-1 deploy should be structurally valid");
}

// --- Rejection tests ---

#[test]
fn prefix_rejection_unrecognized_shape() {
    // A single DEFAULT frame with no VERIFY frames cannot match any shape.
    let tx = base_frame_tx_with_frames(vec![Frame {
        mode: FrameMode::Default as u8,
        flags: 0x00,
        target: None,
        gas_limit: 10_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }]);
    assert_eq!(
        tx.validation_prefix().unwrap_err(),
        FrameValidationError::UnrecognizedPrefix
    );
}

#[test]
fn prefix_rejection_deploy_not_first() {
    // A VERIFY frame followed by a DEFAULT frame: the DEFAULT is not at index 0
    // of non-expiry frames, so this doesn't match any shape that has a deploy.
    // It also doesn't match SelfVerify (wrong scope) or OnlyVerifyPay (wrong scope).
    // This is unrecognized.
    let tx = base_frame_tx_with_frames(vec![
        Frame {
            mode: FrameMode::Verify as u8,
            flags: APPROVE_EXECUTION_AND_PAYMENT,
            target: Some(sender_addr()),
            gas_limit: 5_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
        deploy_frame(),
    ]);
    // Shape matching succeeds (SelfVerify — only the first frame matters for prefix).
    let prefix = tx
        .validation_prefix()
        .expect("SelfVerify recognized (deploy after prefix is ignored)");
    assert_eq!(prefix.shape, PrefixShape::SelfVerify);
    // Structure validation passes too (the deploy frame is not in the prefix).
    tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
        .expect("SelfVerify with trailing deploy is structurally valid");
}

#[test]
fn prefix_rejection_two_deploys_in_prefix() {
    // DeployOnlyVerifyPay with two DEFAULT frames before the pair — the second
    // DEFAULT would be at non-zero position, which doesn't match any shape.
    // Shape matching: position 0 is DEFAULT, position 1 is DEFAULT (not VERIFY) —
    // none of the four shapes matches.
    let tx = base_frame_tx_with_frames(vec![
        deploy_frame(),
        deploy_frame(),
        only_verify_frame(),
        pay_frame(),
    ]);
    // Position 0=DEFAULT, position 1=DEFAULT → DeployOnlyVerifyPay needs
    // pos 1 to be VERIFY(exec). Shape is unrecognized.
    assert_eq!(
        tx.validation_prefix().unwrap_err(),
        FrameValidationError::UnrecognizedPrefix
    );
}

#[test]
fn prefix_rejection_target_not_sender() {
    let other = Address::from_low_u64_be(0xDEAD);
    let mut frame = self_verify_frame();
    frame.target = Some(other);
    let tx = base_frame_tx_with_frames(vec![frame]);
    let prefix = tx.validation_prefix().expect("shape recognized");
    assert_eq!(
        tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
            .unwrap_err(),
        FrameValidationError::VerifyTargetNotSender { frame_index: 0 }
    );
}

#[test]
fn prefix_rejection_wrong_scope_self_verify() {
    // SelfVerify frame must have scope APPROVE_EXECUTION_AND_PAYMENT (0x3),
    // not APPROVE_EXECUTION (0x2).
    let mut frame = self_verify_frame();
    frame.flags = APPROVE_EXECUTION;
    let tx = base_frame_tx_with_frames(vec![frame, pay_frame()]);
    // With scope 0x2 at position 0 and APPROVE_PAYMENT at position 1, this
    // matches OnlyVerifyPay shape (pos 0 = VERIFY(exec), pos 1 = VERIFY(pay)).
    let prefix = tx.validation_prefix().expect("OnlyVerifyPay recognized");
    assert_eq!(prefix.shape, PrefixShape::OnlyVerifyPay);
    tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
        .expect("OnlyVerifyPay structure is valid");
    // Now single VERIFY with wrong scope for SelfVerify: only one VERIFY with
    // APPROVE_EXECUTION means no SelfVerify shape.
    let tx2 = base_frame_tx_with_frames(vec![Frame {
        mode: FrameMode::Verify as u8,
        flags: APPROVE_EXECUTION,
        target: Some(sender_addr()),
        gas_limit: 10_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }]);
    assert_eq!(
        tx2.validation_prefix().unwrap_err(),
        FrameValidationError::UnrecognizedPrefix,
        "VERIFY with APPROVE_EXECUTION alone is not a recognized shape"
    );
}

#[test]
fn prefix_rejection_wrong_scope_only_verify_pay() {
    // only_verify frame must have APPROVE_EXECUTION (0x2), not 0x3.
    let mut verify = only_verify_frame();
    verify.flags = APPROVE_EXECUTION_AND_PAYMENT;
    // Both frames have scope 0x3: doesn't match OnlyVerifyPay (pos 0 needs 0x2),
    // but matches SelfVerify (pos 0 has scope 0x3).
    let tx = base_frame_tx_with_frames(vec![verify, pay_frame()]);
    let prefix = tx.validation_prefix().expect("SelfVerify recognized");
    assert_eq!(prefix.shape, PrefixShape::SelfVerify);
    // The prefix covers only the first frame, which leaves the `pay` frame as a
    // VERIFY frame after the prefix — banned by structural rule 8.
    assert_eq!(
        tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
            .unwrap_err(),
        FrameValidationError::VerifyFrameAfterPrefix { frame_index: 1 }
    );
}

#[test]
fn prefix_rejection_atomic_batch_in_prefix() {
    let mut frame = self_verify_frame();
    frame.flags = APPROVE_EXECUTION_AND_PAYMENT | 0x04; // set atomic batch bit
    // Need a following frame so static validation doesn't reject atomic batch on last frame.
    let tx = base_frame_tx_with_frames(vec![frame, pay_frame()]);
    // Shape: pos 0 has scope 0x3 (bits 0-1 of 0x07 = 0x3) and VERIFY mode → SelfVerify.
    let prefix = tx.validation_prefix().expect("SelfVerify recognized");
    assert_eq!(prefix.shape, PrefixShape::SelfVerify);
    assert_eq!(
        tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
            .unwrap_err(),
        FrameValidationError::AtomicBatchInPrefix { frame_index: 0 }
    );
}

#[test]
fn prefix_rejection_gas_budget_exceeded() {
    // Give the self_verify frame a gas_limit that, combined with sig cost,
    // exceeds MAX_VERIFY_GAS (100_000). Sig cost for one SECP256K1 = 2800.
    let mut frame = self_verify_frame();
    frame.gas_limit = FRAME_TX_MAX_VERIFY_GAS; // 100_000 alone already == limit
    let mut tx = base_frame_tx_with_frames(vec![frame]);
    // Ensure exactly one SECP256K1 sig so sig cost = 2800.
    tx.signatures = vec![FrameSignature {
        scheme: FRAME_SIG_SCHEME_SECP256K1,
        signer: Some(sender_addr()),
        msg: Bytes::new(),
        signature: Bytes::from(vec![0u8; 65]),
    }];
    let prefix = tx.validation_prefix().expect("SelfVerify recognized");
    // 100_000 + 2_800 > 100_000 → budget exceeded.
    assert!(matches!(
        tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
            .unwrap_err(),
        FrameValidationError::VerifyGasBudgetExceeded { .. }
    ));
}

// ---------------------------------------------------------------------------
// Static constraints and gas accounting
// ---------------------------------------------------------------------------

/// A frame transaction that satisfies every static constraint, for tests that
/// then violate exactly one of them.
fn make_test_frame_tx() -> FrameTransaction {
    FrameTransaction {
        chain_id: 1,
        nonce_keys: vec![U256::zero()],
        nonce_seq: 42,
        sender: Address::from_low_u64_be(0xABCD),
        frames: vec![
            Frame {
                mode: FrameMode::Verify as u8,
                flags: 0x03, // APPROVE_EXECUTION_AND_PAYMENT
                target: Some(Address::from_low_u64_be(0xABCD)),
                gas_limit: 100_000,
                state_limit: 0,
                value: U256::zero(),
                data: Bytes::from_static(b"verify_data"),
            },
            Frame {
                mode: FrameMode::Sender as u8,
                flags: 0x00,
                target: Some(Address::from_low_u64_be(0x1234)),
                gas_limit: 200_000,
                state_limit: 0,
                value: U256::zero(),
                data: Bytes::from_static(b"call_data"),
            },
        ],
        signatures: vec![FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(Address::from_low_u64_be(0xABCD)),
            msg: Bytes::new(),
            signature: Bytes::from(vec![0u8; 65]),
        }],
        max_priority_fee_per_gas: 1_000_000_000,
        max_fee_per_gas: 30_000_000_000,
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: vec![],
        ..Default::default()
    }
}

#[test]
fn atomic_batch_flag_on_verify_frame_is_invalid() {
    let mut tx = make_test_frame_tx();
    tx.frames = vec![
        Frame {
            mode: FrameMode::Verify as u8,
            flags: 0x04 | 0x03, // atomic batch + scope bits
            target: None,
            gas_limit: 21_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
        Frame {
            mode: FrameMode::Sender as u8,
            flags: 0x00,
            target: Some(Address::from_low_u64_be(0xCAFE)),
            gas_limit: 21_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ];
    assert!(
        tx.validate_static_constraints(false)
            .unwrap_err()
            .contains("atomic batch flag on a VERIFY frame")
    );
}

#[test]
fn atomic_batch_followed_by_verify_frame_is_invalid() {
    // Batches never contain VERIFY frames, so the frame a batch member
    // batches with must be non-VERIFY too.
    let mut tx = make_test_frame_tx();
    tx.frames = vec![
        Frame {
            mode: FrameMode::Sender as u8,
            flags: 0x04, // atomic batch
            target: Some(Address::from_low_u64_be(0xB0B)),
            gas_limit: 21_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
        Frame {
            mode: FrameMode::Verify as u8,
            flags: 0x03,
            target: None,
            gas_limit: 21_000,
            state_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
    ];
    assert!(
        tx.validate_static_constraints(false)
            .unwrap_err()
            .contains("atomic batch flag followed by a VERIFY frame")
    );
}

#[test]
fn static_validation_rejects_approve_execution_with_third_party_target() {
    // Approval of execution is only allowed when the target is empty or tx.sender.
    let mut tx = make_test_frame_tx();
    tx.frames[0].target = Some(Address::from_low_u64_be(0xBEEF));
    assert!(
        tx.validate_static_constraints(false)
            .unwrap_err()
            .contains("APPROVE_EXECUTION requires an empty target or tx.sender"),
    );
    // An empty target resolves to tx.sender, so it is allowed.
    tx.frames[0].target = None;
    assert!(tx.validate_static_constraints(false).is_ok());
}

#[test]
fn static_validation_rejects_wrong_blob_hash_version() {
    let mut tx = make_test_frame_tx();
    tx.blob_versioned_hashes = vec![H256([0xABu8; 32])];
    tx.max_fee_per_blob_gas = U256::from(1u64);
    assert!(
        tx.validate_static_constraints(false)
            .unwrap_err()
            .contains("wrong version byte"),
    );
    let mut hash = [0xABu8; 32];
    hash[0] = VERSIONED_HASH_VERSION_KZG;
    tx.blob_versioned_hashes = vec![H256(hash)];
    assert!(tx.validate_static_constraints(false).is_ok());
}

#[test]
fn static_validation_rejects_blob_fee_without_blobs() {
    let mut tx = make_test_frame_tx();
    assert!(tx.blob_versioned_hashes.is_empty());
    tx.max_fee_per_blob_gas = U256::from(1u64);
    assert!(
        tx.validate_static_constraints(false)
            .unwrap_err()
            .contains("max_fee_per_blob_gas must be zero"),
    );
}

/// The EIP-7594 per-transaction blob limit applies to frame transactions
/// unchanged (EIP-8141 §Blob-carrying frame transactions). It must bind here
/// rather than only on the sidecar: block execution has no sidecar to check, so
/// a static-constraint miss would make ethrex accept a block that a conformant
/// client rejects.
#[test]
fn static_validation_rejects_more_blobs_than_the_per_transaction_limit() {
    let mut tx = make_test_frame_tx();
    tx.max_fee_per_blob_gas = U256::from(1u64);
    let mut hash = [0x11u8; 32];
    hash[0] = VERSIONED_HASH_VERSION_KZG;
    tx.blob_versioned_hashes = vec![H256(hash); MAX_BLOBS_PER_TX];
    assert!(
        tx.validate_static_constraints(false).is_ok(),
        "{MAX_BLOBS_PER_TX} blobs are within the per-transaction limit"
    );

    tx.blob_versioned_hashes.push(H256(hash));
    assert!(
        tx.validate_static_constraints(false)
            .unwrap_err()
            .contains(&format!("Blob count must not exceed {MAX_BLOBS_PER_TX}")),
    );
}

#[test]
fn data_cost_covers_frame_signature_and_nonce_data() {
    // 4 gas per zero byte, 16 per non-zero, over frame.data, each signature's
    // signer/msg/signature, and EIP-8250's rlp(nonce_keys) || rlp(nonce_seq) —
    // no other RLP framing, no other scalar fields.
    let mut tx = make_test_frame_tx();
    tx.frames[0].data = Bytes::from(vec![0u8; 3]); // 3 zero bytes -> 12
    tx.frames[1].data = Bytes::from(vec![0xAAu8; 2]); // 2 non-zero -> 32
    tx.signatures = vec![FrameSignature {
        scheme: FRAME_SIG_SCHEME_ARBITRARY,
        signer: None, // empty signer contributes nothing
        msg: Bytes::new(),
        signature: Bytes::from(vec![0xAAu8, 0x00]), // 16 + 4
    }];
    // nonce_keys == [0] and nonce_seq == 42 encode as c1 80 2a: 3 non-zero -> 48.
    assert_eq!(tx.nonce_calldata(), vec![0xc1, 0x80, 0x2a]);
    assert_eq!(tx.data_cost(), 12 + 32 + 16 + 4 + 48);
    // Floor tokens are unweighted: (7 data + 3 nonce) bytes * 4 tokens * 16 gas.
    assert_eq!(tx.calldata_tokens(), 10 * 4);
    assert_eq!(tx.calldata_floor_gas(), 10 * 64);
}

#[test]
fn max_gas_takes_the_calldata_floor_when_it_exceeds_the_standard_limit() {
    // EIP-8141 `max_gas = max(standard_gas_limit, calldata_floor_gas)`. A
    // transaction whose data floor exceeds what it declared for execution
    // reserves the floor; it is valid, not rejected.
    let mut tx = make_test_frame_tx();
    // 64 bytes of frame data plus the 3 nonce-calldata bytes are 67 floor bytes,
    // needing 4288 gas of floor, which frames carrying 100 gas each cannot cover.
    tx.signatures.clear();
    tx.frames[0].data = Bytes::from(vec![0xAAu8; 64]);
    tx.frames[1].data = Bytes::new();
    tx.frames[0].gas_limit = 100;
    tx.frames[1].gas_limit = 100;
    assert_eq!(tx.calldata_floor_gas(), 4288);
    assert!(tx.calldata_floor_total() > tx.standard_gas_limit());
    assert_eq!(tx.total_gas_limit(), tx.calldata_floor_total());
    assert!(tx.validate_static_constraints(false).is_ok());

    // With enough frame gas to outweigh the floor, `max_gas` is the standard limit.
    tx.frames[1].gas_limit = 100_000;
    assert!(tx.standard_gas_limit() > tx.calldata_floor_total());
    assert_eq!(tx.total_gas_limit(), tx.standard_gas_limit());
    assert!(tx.validate_static_constraints(false).is_ok());
}

// ---------------------------------------------------------------------------
// EIP-8312 frame-mode allocation and activation gate
//
// The frame-mode table is unconditional: {0 DEFAULT, 1 VERIFY, 2 SENDER,
// 3 unassigned, 4 reserved (EIP-8288 DEP_VERIFY), 5 UTXO (EIP-8312)}.
// Only mode 5's *admissibility* is gated, on the EIP-8312 activation predicate,
// so that a chain adopting EIP-8312 at a future timestamp re-executes all of its
// existing blocks identically.
// ---------------------------------------------------------------------------

/// A UTXO-mode frame. Its `data` is not a well-formed spend payload — these
/// tests exercise the mode gate, which is reached before any payload decoding.
fn utxo_frame() -> Frame {
    Frame {
        mode: FrameMode::Utxo as u8,
        flags: 0x00,
        target: None,
        gas_limit: 50_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

#[test]
fn frame_mode_wire_bytes_are_pinned() {
    // These byte values are consensus-visible and covered by the transaction's
    // signature, so pin them: mode 3 is unassigned, mode 4 stays reserved for
    // EIP-8288's deferred DEP_VERIFY, and EIP-8312 UTXO takes 5 — a documented
    // deviation from EIP-8312's own `UTXO_MODE = 3`.
    assert_eq!(FrameMode::Default as u8, 0);
    assert_eq!(FrameMode::Verify as u8, 1);
    assert_eq!(FrameMode::Sender as u8, 2);
    assert_eq!(FrameMode::Utxo as u8, 5);

    assert_eq!(FrameMode::from_u8(0), Some(FrameMode::Default));
    assert_eq!(FrameMode::from_u8(1), Some(FrameMode::Verify));
    assert_eq!(FrameMode::from_u8(2), Some(FrameMode::Sender));
    assert_eq!(FrameMode::from_u8(3), None, "mode 3 is unassigned");
    assert_eq!(FrameMode::from_u8(4), None, "mode 4 reserved for EIP-8288");
    assert_eq!(FrameMode::from_u8(5), Some(FrameMode::Utxo));
    for reserved in 6u8..=255 {
        assert_eq!(FrameMode::from_u8(reserved), None);
    }
}

#[test]
fn reserved_mode_never_falls_back_to_default() {
    // `execution_mode` must return None for a reserved byte rather than
    // resolving it to DEFAULT: a silent fallback would execute an unknown frame
    // kind as an ordinary EVM call.
    for reserved in [4u8, 6, 7, 200, 255] {
        let frame = Frame {
            mode: reserved,
            ..deploy_frame()
        };
        assert_eq!(
            frame.execution_mode(),
            None,
            "mode {reserved} must not resolve to a defined mode"
        );
    }
}

#[test]
fn utxo_frame_rejected_before_activation_and_accepted_after() {
    let tx = base_frame_tx_with_frames(vec![self_verify_frame(), utxo_frame()]);

    // Before activation mode 5 is reserved, exactly as it was before EIP-8312
    // existed — this is what keeps already-produced blocks re-executing
    // identically when a running chain adopts EIP-8312 at a future timestamp.
    let err = tx.validate_static_constraints(false).unwrap_err();
    assert!(
        err.contains("not active"),
        "expected an EIP-8312-inactive error, got: {err}"
    );

    // From activation the same transaction passes the mode gate. Whether it is
    // valid overall depends on the spend-payload rules, which are a separate
    // concern reached only once the mode is admissible — so assert on the gate,
    // not on the outcome.
    if let Err(err_after) = tx.validate_static_constraints(true) {
        assert!(
            !err_after.contains("not active"),
            "mode gate must not fire once EIP-8312 is active, got: {err_after}"
        );
    }
}

#[test]
fn modes_three_and_four_are_reserved_regardless_of_activation() {
    // Mode 3 is unassigned and EIP-8288's DEP_VERIFY (mode 4) is deferred
    // upstream, so both are invalid on both sides of the EIP-8312 boundary.
    for mode in [3u8, 4] {
        let frame = Frame {
            mode,
            ..deploy_frame()
        };
        let tx = base_frame_tx_with_frames(vec![self_verify_frame(), frame]);
        for active in [false, true] {
            let err = tx.validate_static_constraints(active).unwrap_err();
            assert!(
                err.contains(&format!("reserved execution mode {mode}")),
                "expected reserved-mode error (mode={mode}, active={active}), got: {err}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// EIP-8312 spend payload: RLP shape, static bounds, spend hash, and the
// transaction-level vault-sender rules.
// ---------------------------------------------------------------------------

fn actor_addr() -> Address {
    Address::from_low_u64_be(0xAC70)
}

fn recipient_addr() -> Address {
    Address::from_low_u64_be(0x9EC1)
}

/// A minimal well-formed spend: one input, one paying UTXO output plus a change
/// output, sponsored by a third party.
fn valid_spend() -> Spend {
    Spend {
        actors: vec![actor_addr()],
        inputs: vec![SpendInput {
            index: 7,
            creation_block: 100,
            source: Address::from_low_u64_be(0x5011),
            recipient: actor_addr(),
            value: U256::from(1_000_000u64),
            position: 0,
            siblings: vec![H256::zero(); 3],
            batch_siblings: vec![],
        }],
        utxo_outs: vec![
            SpendOutput {
                recipient: recipient_addr(),
                value: U256::from(400_000u64),
            },
            // change output: signed with value zero
            SpendOutput {
                recipient: actor_addr(),
                value: U256::zero(),
            },
        ],
        account_outs: vec![],
        change_index: 1,
        payer: Bytes::copy_from_slice(Address::from_low_u64_be(0x5907).as_bytes()),
        max_fee_per_gas: U256::from(30_000_000_000u64),
        max_priority_fee_per_gas: U256::from(1_000_000_000u64),
        max_gas_limit: 500_000,
    }
}

#[test]
fn spend_rlp_roundtrips() {
    let spend = valid_spend();
    let encoded = spend.encode_to_vec();
    let decoded = Spend::decode(&encoded).expect("spend must decode");
    assert_eq!(decoded, spend);
}

#[test]
fn spend_decoding_rejects_trailing_bytes() {
    // A frame's data must be exactly one spend and nothing more, so a relayer
    // cannot smuggle extra bytes past the signature.
    let mut encoded = valid_spend().encode_to_vec();
    encoded.push(0xFF);
    assert!(Spend::decode_frame_data(&Bytes::from(encoded)).is_err());
}

#[test]
fn spend_decoding_rejects_wrong_arity() {
    // An input item is an 8-tuple; a 7-tuple must not decode.
    let short_input: Vec<u8> = {
        let mut buf = Vec::new();
        // Encode a 7-field input list by hand-encoding a truncated structure.
        let spend = valid_spend();
        let mut inner = Vec::new();
        let inp = &spend.inputs[0];
        inp.index.encode(&mut inner);
        inp.creation_block.encode(&mut inner);
        inp.source.encode(&mut inner);
        inp.recipient.encode(&mut inner);
        inp.value.encode(&mut inner);
        inp.position.encode(&mut inner);
        inp.siblings.encode(&mut inner);
        // batch_siblings deliberately omitted
        ethrex_rlp::structs::Encoder::new(&mut buf)
            .encode_raw(&inner)
            .finish();
        buf
    };
    assert!(SpendInput::decode(&short_input).is_err());
}

#[test]
fn spend_static_bounds_accept_a_well_formed_spend() {
    assert!(valid_spend().validate_static().is_ok());
}

#[test]
fn spend_rejects_empty_actor_list_and_duplicates() {
    let mut spend = valid_spend();
    spend.actors.clear();
    assert!(spend.validate_static().unwrap_err().contains("no actors"));

    let mut spend = valid_spend();
    spend.actors = vec![actor_addr(), actor_addr()];
    assert!(
        spend
            .validate_static()
            .unwrap_err()
            .contains("more than once")
    );
}

#[test]
fn spend_rejects_non_increasing_input_indices() {
    // Strictly increasing indices statically exclude spending one UTXO twice
    // inside a single frame.
    let mut spend = valid_spend();
    let dup = spend.inputs[0].clone();
    spend.inputs.push(dup);
    assert!(
        spend
            .validate_static()
            .unwrap_err()
            .contains("strictly increasing")
    );
}

#[test]
fn spend_rejects_empty_input_list() {
    let mut spend = valid_spend();
    spend.inputs.clear();
    assert!(spend.validate_static().unwrap_err().contains("no inputs"));
}

#[test]
fn spend_rejects_oversized_sibling_path_and_out_of_range_position() {
    let mut spend = valid_spend();
    spend.inputs[0].siblings = vec![H256::zero(); MAX_SIBLINGS + 1];
    assert!(spend.validate_static().unwrap_err().contains("siblings"));

    let mut spend = valid_spend();
    // depth 3 ⇒ positions 0..=7
    spend.inputs[0].position = 8;
    assert!(spend.validate_static().unwrap_err().contains("position"));
}

#[test]
fn spend_rejects_wrong_batch_path_length() {
    // A batch path is empty (ring proof) or exactly the batch tree's depth.
    let mut spend = valid_spend();
    spend.inputs[0].batch_siblings = vec![H256::zero(); BATCH_PATH_LEN - 1];
    assert!(spend.validate_static().unwrap_err().contains("batch path"));

    let mut spend = valid_spend();
    spend.inputs[0].batch_siblings = vec![H256::zero(); BATCH_PATH_LEN];
    assert!(spend.validate_static().is_ok());
}

#[test]
fn spend_output_rules() {
    // Change output must be signed with value zero.
    let mut spend = valid_spend();
    spend.utxo_outs[1].value = U256::from(1u64);
    assert!(spend.validate_static().unwrap_err().contains("value zero"));

    // Every non-change output must carry a non-zero value.
    let mut spend = valid_spend();
    spend.utxo_outs[0].value = U256::zero();
    assert!(
        spend
            .validate_static()
            .unwrap_err()
            .contains("non-zero value")
    );

    // change_index must be in range of utxo_outs ++ account_outs.
    let mut spend = valid_spend();
    spend.change_index = 9;
    assert!(
        spend
            .validate_static()
            .unwrap_err()
            .contains("out of range")
    );

    // No zero-address recipients.
    let mut spend = valid_spend();
    spend.utxo_outs[0].recipient = Address::zero();
    assert!(
        spend
            .validate_static()
            .unwrap_err()
            .contains("zero address")
    );
}

#[test]
fn spend_payer_field_shapes() {
    // Empty payer = self-funded.
    let mut spend = valid_spend();
    spend.payer = Bytes::new();
    assert!(spend.validate_static().is_ok());
    assert!(spend.is_self_funded());
    assert_eq!(spend.sponsor(), None);

    // 20-byte payer = sponsor.
    let spend = valid_spend();
    assert!(!spend.is_self_funded());
    assert_eq!(spend.sponsor(), Some(Address::from_low_u64_be(0x5907)));

    // The vault may not be named as sponsor: it is the payer the protocol
    // assigns for self-funded spends.
    let mut spend = valid_spend();
    spend.payer = Bytes::copy_from_slice(utxo_vault().as_bytes());
    assert!(
        spend
            .validate_static()
            .unwrap_err()
            .contains("must not be the vault")
    );

    // A 20-byte zero address is NOT the self-funded marker — the two encodings
    // must not be confusable (upstream pseudocode conflates them).
    let mut spend = valid_spend();
    spend.payer = Bytes::copy_from_slice(Address::zero().as_bytes());
    assert!(!spend.is_self_funded());
    assert!(
        spend
            .validate_static()
            .unwrap_err()
            .contains("zero address")
    );

    // Any other length is malformed.
    let mut spend = valid_spend();
    spend.payer = Bytes::from_static(&[1, 2, 3]);
    assert!(
        spend
            .validate_static()
            .unwrap_err()
            .contains("0 or 20 bytes")
    );
}

#[test]
fn spend_hash_covers_signed_fields_and_ignores_the_witness() {
    let spend = valid_spend();
    let base = spend.spend_hash(1);

    // Domain separation: the same spend on another chain signs a different hash.
    assert_ne!(base, spend.spend_hash(2));

    // Witness refresh must not invalidate a signature: swapping the proof for a
    // batch path leaves the hash unchanged, because only [index, creation_block]
    // of each input is signed.
    let mut refreshed = spend.clone();
    refreshed.inputs[0].siblings = vec![H256::repeat_byte(0xAB); 5];
    refreshed.inputs[0].batch_siblings = vec![H256::repeat_byte(0xCD); BATCH_PATH_LEN];
    refreshed.inputs[0].position = 3;
    refreshed.inputs[0].source = Address::from_low_u64_be(0xDEAD);
    refreshed.inputs[0].value = U256::from(999u64);
    assert_eq!(base, refreshed.spend_hash(1));

    // Signed fields do move the hash.
    let mut altered = spend.clone();
    altered.inputs[0].index += 1;
    assert_ne!(base, altered.spend_hash(1));

    let mut altered = spend.clone();
    altered.utxo_outs[0].recipient = Address::from_low_u64_be(0xBEEF);
    assert_ne!(base, altered.spend_hash(1));

    let mut altered = spend.clone();
    altered.max_gas_limit += 1;
    assert_ne!(base, altered.spend_hash(1));

    // Moving which output is the change entry changes the hash too.
    let mut altered = spend;
    altered.change_index = 0;
    altered.utxo_outs[0].value = U256::zero();
    altered.utxo_outs[1].value = U256::from(1u64);
    assert_ne!(base, altered.spend_hash(1));
}

/// A UTXO frame carrying the given spend.
fn utxo_frame_with(spend: &Spend) -> Frame {
    Frame {
        mode: FrameMode::Utxo as u8,
        flags: 0x00,
        target: None,
        gas_limit: 100_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::from(spend.encode_to_vec()),
    }
}

/// A secp256k1 signature entry naming `signer` over `msg`. Static validation
/// checks the entry's shape and binding, not the cryptography (that happens in
/// the VM), so the signature bytes only need to be well-formed.
fn spend_sig_entry(signer: Address, msg: H256) -> FrameSignature {
    FrameSignature {
        scheme: FRAME_SIG_SCHEME_SECP256K1,
        signer: Some(signer),
        msg: Bytes::copy_from_slice(msg.as_bytes()),
        signature: Bytes::from(vec![0u8; 65]),
    }
}

#[test]
fn an_actor_is_covered_only_by_its_own_signature_over_the_spend_hash() {
    // Two independent conditions guard an actor: the signature's scheme must carry
    // a cryptographic binding, and the resolved signer must BE that actor. Dropping
    // either is theft, and both were unconstrained — the existing per-actor test
    // builds its ARBITRARY case with `signer: None`, so it fails the SIGNER check
    // and never exercises the scheme check. Both conditions also produce the same
    // error message, so each case here is built so only one of them can be failing.

    // --- the scheme condition, and why it is reachable ---------------------
    // EIP-8141 forbids an ARBITRARY entry from naming a signer, so such an entry
    // always resolves to `tx.sender` — which for any UTXO transaction is the vault.
    // Nothing forbids the vault from being an actor, and nothing forbids an output
    // naming the vault as recipient. So a spend of a vault-owned UTXO, with the
    // vault as its actor, is covered by an *unsigned* ARBITRARY entry unless the
    // scheme is checked. That check is the only reason "a UTXO paid to the vault can
    // never be spent" holds.
    let mut vault_spend = valid_spend();
    vault_spend.actors = vec![utxo_vault()];
    vault_spend.inputs[0].recipient = utxo_vault();
    // The self-funded shape: vault sender, empty payer, and the UTXO frame as the
    // transaction's only frame. That is what makes `tx.sender` the vault, which is
    // what an unnamed ARBITRARY signer resolves to.
    let mut unsigned_vault = base_frame_tx_with_frames(vec![utxo_frame_with(&vault_spend)]);
    unsigned_vault.sender = utxo_vault();
    unsigned_vault.nonce_keys = vec![];
    unsigned_vault.nonce_seq = 0;
    let vault_hash = vault_spend.spend_hash(unsigned_vault.chain_id);
    unsigned_vault.signatures.push(FrameSignature {
        scheme: FRAME_SIG_SCHEME_ARBITRARY,
        signer: None, // resolves to tx.sender == the vault == the actor
        msg: Bytes::copy_from_slice(vault_hash.as_bytes()),
        signature: Bytes::from(vec![0u8; 8]),
    });
    assert!(
        unsigned_vault
            .validate_static_constraints(true)
            .unwrap_err()
            .contains("no spend-hash signature entry"),
        "an ARBITRARY entry carries no binding and must not cover the vault as actor"
    );

    // --- the signer condition ----------------------------------------------
    // A protocol scheme over the right digest, signed by a third party. Scheme and
    // msg both match, so the signer binding is the only thing that can reject it.
    let spend = valid_spend();
    let tx = base_frame_tx_with_frames(vec![self_verify_frame(), utxo_frame_with(&spend)]);
    let hash = spend.spend_hash(tx.chain_id);

    let mut ok = tx.clone();
    ok.signatures.push(spend_sig_entry(actor_addr(), hash));
    assert!(
        ok.validate_static_constraints(true).is_ok(),
        "the correctly-signed spend must pass, or the rejection below proves nothing"
    );

    let mut wrong_signer = tx;
    wrong_signer
        .signatures
        .push(spend_sig_entry(Address::from_low_u64_be(0x5151), hash));
    assert!(
        wrong_signer
            .validate_static_constraints(true)
            .unwrap_err()
            .contains("no spend-hash signature entry"),
        "a third party's signature over the spend hash must not authorise an actor"
    );
}

#[test]
fn utxo_frame_requires_a_spend_hash_signature_per_actor() {
    let spend = valid_spend();
    let mut tx = base_frame_tx_with_frames(vec![self_verify_frame(), utxo_frame_with(&spend)]);

    // No entry for the actor → rejected.
    let err = tx.validate_static_constraints(true).unwrap_err();
    assert!(err.contains("no spend-hash signature entry"), "got: {err}");

    // With a matching entry the frame's rules pass.
    let hash = spend.spend_hash(tx.chain_id);
    tx.signatures.push(spend_sig_entry(actor_addr(), hash));
    assert!(tx.validate_static_constraints(true).is_ok());

    // An ARBITRARY-scheme entry carries no cryptographic binding, so it must not
    // satisfy an actor.
    let mut arbitrary_tx = tx.clone();
    arbitrary_tx.signatures.pop();
    arbitrary_tx.signatures.push(FrameSignature {
        scheme: FRAME_SIG_SCHEME_ARBITRARY,
        signer: None,
        msg: Bytes::copy_from_slice(hash.as_bytes()),
        signature: Bytes::from(vec![0u8; 8]),
    });
    assert!(
        arbitrary_tx
            .validate_static_constraints(true)
            .unwrap_err()
            .contains("no spend-hash signature entry")
    );

    // An entry over a different digest does not cover the actor either.
    let mut wrong_msg_tx = tx.clone();
    wrong_msg_tx.signatures.pop();
    wrong_msg_tx
        .signatures
        .push(spend_sig_entry(actor_addr(), H256::repeat_byte(0x11)));
    assert!(
        wrong_msg_tx
            .validate_static_constraints(true)
            .unwrap_err()
            .contains("no spend-hash signature entry")
    );
}

#[test]
fn utxo_frame_tuple_and_placement_rules() {
    let spend = valid_spend();
    let hash = spend.spend_hash(1);

    // flags must be zero — in particular no atomic-batch flag.
    let mut frame = utxo_frame_with(&spend);
    frame.flags = 0x04;
    let mut tx = base_frame_tx_with_frames(vec![self_verify_frame(), frame]);
    tx.signatures.push(spend_sig_entry(actor_addr(), hash));
    assert!(
        tx.validate_static_constraints(true)
            .unwrap_err()
            .contains("flags == 0")
    );

    // A target would give the spend call semantics it does not have.
    let mut frame = utxo_frame_with(&spend);
    frame.target = Some(recipient_addr());
    let mut tx = base_frame_tx_with_frames(vec![self_verify_frame(), frame]);
    tx.signatures.push(spend_sig_entry(actor_addr(), hash));
    assert!(
        tx.validate_static_constraints(true)
            .unwrap_err()
            .contains("no target")
    );

    // A UTXO frame must not be an atomic batch's terminator: the batch's revert
    // would try to roll back its irreversible spent bits.
    let mut batched = deploy_frame();
    batched.flags = 0x04;
    let mut tx =
        base_frame_tx_with_frames(vec![self_verify_frame(), batched, utxo_frame_with(&spend)]);
    tx.signatures.push(spend_sig_entry(actor_addr(), hash));
    assert!(
        tx.validate_static_constraints(true)
            .unwrap_err()
            .contains("must not follow an atomic-batch frame")
    );
}

#[test]
fn self_funded_spend_shape_rules() {
    let mut spend = valid_spend();
    spend.payer = Bytes::new();
    let hash = spend.spend_hash(1);

    // Must be the only frame in its transaction (the vault fronts a
    // transaction-scoped maximum cost). The rest of the envelope is well-formed
    // so this rule is the only one violated.
    let mut tx = base_frame_tx_with_frames(vec![deploy_frame(), utxo_frame_with(&spend)]);
    tx.sender = utxo_vault();
    tx.nonce_keys = vec![];
    tx.nonce_seq = 0;
    tx.signatures.push(spend_sig_entry(actor_addr(), hash));
    assert!(
        tx.validate_static_constraints(true)
            .unwrap_err()
            .contains("only frame")
    );

    // Must have the vault as sender.
    let mut tx = base_frame_tx_with_frames(vec![utxo_frame_with(&spend)]);
    tx.signatures.push(spend_sig_entry(actor_addr(), hash));
    assert!(
        tx.validate_static_constraints(true)
            .unwrap_err()
            .contains("vault as tx.sender")
    );

    // Well-formed self-funded spend: vault sender, single frame, no nonce keys,
    // zero nonce_seq.
    let mut tx = base_frame_tx_with_frames(vec![utxo_frame_with(&spend)]);
    tx.sender = utxo_vault();
    tx.nonce_keys = vec![];
    tx.nonce_seq = 0;
    tx.signatures.push(spend_sig_entry(actor_addr(), hash));
    assert!(
        tx.validate_static_constraints(true).is_ok(),
        "got: {:?}",
        tx.validate_static_constraints(true)
    );
}

#[test]
fn vault_sender_envelope_rules() {
    let mut spend = valid_spend();
    spend.payer = Bytes::new();
    let hash = spend.spend_hash(1);

    let base = || {
        let mut tx = base_frame_tx_with_frames(vec![utxo_frame_with(&spend)]);
        tx.sender = utxo_vault();
        tx.nonce_keys = vec![];
        tx.nonce_seq = 0;
        tx.signatures.push(spend_sig_entry(actor_addr(), hash));
        tx
    };

    // nonce_seq must be zero: nothing signs a vault-sender envelope, so every
    // field it carries has to be pinned here.
    let mut tx = base();
    tx.nonce_seq = 1;
    assert!(
        tx.validate_static_constraints(true)
            .unwrap_err()
            .contains("nonce_seq == 0")
    );

    // Blobs are forbidden at consensus level, not just as mempool policy.
    let mut tx = base();
    let mut blob_hash = H256::zero();
    blob_hash.0[0] = VERSIONED_HASH_VERSION_KZG;
    tx.blob_versioned_hashes = vec![blob_hash];
    assert!(
        tx.validate_static_constraints(true)
            .unwrap_err()
            .contains("no blobs")
    );

    // A vault-sender transaction with no UTXO frame has neither a nonce nor a
    // spend, so nothing would stop it being replayed.
    let mut tx = base();
    tx.frames = vec![deploy_frame()];
    assert!(
        tx.validate_static_constraints(true)
            .unwrap_err()
            .contains("at least one UTXO frame")
    );

    // Before activation the vault is an ordinary address with no carve-outs, so
    // the usual EIP-8250 nonce-key requirement applies.
    let mut tx = base();
    assert!(
        tx.validate_static_constraints(false)
            .unwrap_err()
            .contains("nonce_keys count")
    );
    tx.nonce_keys = vec![U256::zero()];
    assert!(
        tx.validate_static_constraints(false)
            .unwrap_err()
            .contains("not active"),
        "a UTXO frame must still be inadmissible before activation"
    );
}

// ---------------------------------------------------------------------------
// EIP-8312 openings tree and vault slot layout.
//
// These are the shared commitment primitives: root construction (block end),
// proof verification (frame execution), and mempool policy all use them, so a
// divergence here is a consensus divergence.
// ---------------------------------------------------------------------------

fn leaf(n: u64) -> H256 {
    opening_leaf(
        n,
        Address::from_low_u64_be(0x5000 + n),
        Address::from_low_u64_be(0x6000 + n),
        U256::from(1_000u64 + n),
    )
}

#[test]
fn opening_leaf_matches_the_spec_preimage() {
    // leaf = keccak256(index_be8 ++ source ++ recipient ++ value_be32): 80 bytes.
    let index = 0x0102030405060708u64;
    let source = Address::from_low_u64_be(0xAAAA);
    let recipient = Address::from_low_u64_be(0xBBBB);
    let value = U256::from(0xCCCCu64);

    let mut preimage = Vec::new();
    preimage.extend_from_slice(&index.to_be_bytes());
    preimage.extend_from_slice(source.as_bytes());
    preimage.extend_from_slice(recipient.as_bytes());
    preimage.extend_from_slice(&value.to_big_endian());
    assert_eq!(preimage.len(), 80);

    assert_eq!(
        opening_leaf(index, source, recipient, value),
        ethrex_common::utils::keccak(&preimage)
    );
}

#[test]
fn empty_openings_tree_is_the_zero_sentinel() {
    // A block that creates no UTXOs writes 32 zero bytes over its ring slot.
    // The sentinel is unforgeable because a leaf is keccak of 80 bytes and can
    // never be zero.
    assert_eq!(merkle_root(&[]), H256::zero());
}

#[test]
fn single_leaf_root_is_the_leaf() {
    // len 1 is already a power of two and the folding loop does not run.
    // Safe despite looking like a type confusion: leaves hash 80-byte preimages
    // and interior nodes 64-byte ones, so one cannot masquerade as the other.
    assert_eq!(merkle_root(&[leaf(0)]), leaf(0));
}

#[test]
fn merkle_root_pads_to_a_power_of_two_not_per_odd_level() {
    // Five leaves distinguish the two schemes. Power-of-two padding (the spec's)
    // pads to 8, so at level 2 the node covering `e` is paired with
    // keccak(0‖0). Per-odd-level padding would pair it with a raw zero word and
    // produce a different root. Pin the spec's answer.
    let leaves: Vec<H256> = (0..5).map(leaf).collect();

    let zero = H256::zero();
    let l0 = hash_pair(leaves[0], leaves[1]);
    let l1 = hash_pair(leaves[2], leaves[3]);
    let l2 = hash_pair(leaves[4], zero);
    let l3 = hash_pair(zero, zero);
    let expected = hash_pair(hash_pair(l0, l1), hash_pair(l2, l3));
    assert_eq!(merkle_root(&leaves), expected);

    // The per-odd-level alternative, for contrast: it must NOT match.
    let odd_level_variant = {
        let a = hash_pair(leaves[0], leaves[1]);
        let b = hash_pair(leaves[2], leaves[3]);
        let c = hash_pair(leaves[4], zero);
        hash_pair(hash_pair(a, b), hash_pair(c, zero))
    };
    assert_ne!(
        merkle_root(&leaves),
        odd_level_variant,
        "the two padding schemes must be observably different, or this test proves nothing"
    );
}

#[test]
fn fold_verifies_every_leaf_of_every_tree_size() {
    // The round-trip property that makes proofs work: for every tree size and
    // every position, folding the leaf with its sibling path reproduces the
    // root. This is what ties root construction to proof verification.
    for size in 1usize..=17 {
        let leaves: Vec<H256> = (0..size as u64).map(leaf).collect();
        let root = merkle_root(&leaves);
        for position in 0..size {
            let proof = merkle_proof(&leaves, position).expect("position in range");
            assert_eq!(
                fold(leaves[position], position as u64, &proof),
                root,
                "size {size}, position {position}"
            );
        }
        assert!(merkle_proof(&leaves, size).is_none());
    }
}

#[test]
fn fold_is_position_sensitive() {
    // Sibling order comes from the position bits, never from sorting: folding a
    // leaf at the wrong claimed position must not reproduce the root. (A
    // commutative/sorted-pair tree would accept either, which is exactly the
    // convention the L2 message tree uses and this one must not.)
    let leaves: Vec<H256> = (0..4).map(leaf).collect();
    let root = merkle_root(&leaves);
    let proof = merkle_proof(&leaves, 1).unwrap();
    assert_eq!(fold(leaves[1], 1, &proof), root);
    assert_ne!(fold(leaves[1], 0, &proof), root);
}

#[test]
fn batch_path_depth_matches_the_batch_size() {
    // A batch always has exactly BATCH_SIZE leaves (one openings root per
    // block), so a batch proof is exactly log2(BATCH_SIZE) siblings — the
    // constant a spend's `batch_siblings` length is validated against.
    assert_eq!(BATCH_PATH_LEN, 13);
    assert_eq!(1u64 << BATCH_PATH_LEN, BATCH_SIZE);

    // Verify against a real (sparse but full-width) batch tree.
    let roots: Vec<H256> = (0..BATCH_SIZE).map(leaf).collect();
    let batch_root = merkle_root(&roots);
    let position = 4095usize;
    let proof = merkle_proof(&roots, position).unwrap();
    assert_eq!(proof.len(), BATCH_PATH_LEN);
    assert_eq!(fold(roots[position], position as u64, &proof), batch_root);
}

#[test]
fn batch_slot_matches_the_specified_formula() {
    // The spec puts a batch root at `2**128 + block_number / BATCH_SIZE`. Pinned
    // against that arithmetic written out, NOT against `batch_slot_for_block`
    // itself: the batch-sealing test computes its expected slot with that helper,
    // so a helper that shifts by a whole batch shifts the assertion with it and
    // the sealing test still passes. Mutating the offset used to survive the suite
    // for exactly that reason.
    assert_eq!(
        slot_batch_base(),
        U256::one() << 128,
        "the batch region must start at 2**128"
    );
    // Every block of batch 0 maps to the base slot, and batch 1 to base + 1.
    for block in [0u64, 1, BATCH_SIZE - 1] {
        assert_eq!(
            batch_slot_for_block(block),
            slot_batch_base(),
            "block {block} is in batch 0"
        );
    }
    assert_eq!(
        batch_slot_for_block(BATCH_SIZE),
        slot_batch_base() + U256::one(),
        "the first block of batch 1 must map to base + 1"
    );
    assert_eq!(
        batch_slot_for_block(BATCH_SIZE * 2 + 7),
        slot_batch_base() + U256::from(2u64),
        "batch index is block / BATCH_SIZE, floored"
    );
}

#[test]
fn vault_slot_regions_are_disjoint() {
    // next-index 0 | ring 1..=8192 | batch 2**128.. | spent 2**129..
    assert_eq!(U256::from(SLOT_NEXT_INDEX), U256::zero());
    assert_eq!(ring_slot(0), U256::from(SLOT_RING_BASE));
    assert_eq!(ring_slot(RING_SIZE - 1), U256::from(RING_SIZE));
    // The ring aliases every RING_SIZE blocks — which is why a spend's window
    // check bounds how old a referenced creation block may be.
    assert_eq!(ring_slot(RING_SIZE), ring_slot(0));
    assert_eq!(ring_slot(RING_SIZE + 5), ring_slot(5));

    // Highest reachable ring slot is far below the batch region.
    let max_ring = U256::from(SLOT_RING_BASE) + U256::from(RING_SIZE - 1);
    assert!(max_ring < slot_batch_base());

    // Highest reachable batch slot (block < 2**64) is far below the spent region.
    let max_batch = batch_slot_for_block(u64::MAX);
    assert!(max_batch < slot_spent_base());
    assert_eq!(
        max_batch,
        slot_batch_base() + U256::from(u64::MAX / BATCH_SIZE)
    );

    // Spent words never wrap into anything else: index < 2**64 ⇒ word < 2**56.
    let (max_spent_slot, _) = spent_bit_location(u64::MAX);
    assert_eq!(
        max_spent_slot,
        slot_spent_base() + U256::from(u64::MAX >> 8)
    );
}

#[test]
fn spent_bit_addressing_packs_256_flags_per_slot() {
    // Bit `index & 0xFF` of word `SLOT_SPENT_BASE + (index >> 8)`.
    let (slot0, mask0) = spent_bit_location(0);
    assert_eq!(slot0, slot_spent_base());
    assert_eq!(mask0, U256::one());

    let (slot255, mask255) = spent_bit_location(255);
    assert_eq!(slot255, slot_spent_base());
    assert_eq!(mask255, U256::one() << 255usize);

    // 256 starts the next word.
    let (slot256, mask256) = spent_bit_location(256);
    assert_eq!(slot256, slot_spent_base() + U256::one());
    assert_eq!(mask256, U256::one());

    // Indices in one word are independent.
    let word = mask0 | mask256;
    assert!(is_spent(word, 0));
    assert!(!is_spent(word, 1));
    assert!(!is_spent(word, 255));
    assert!(is_spent(U256::one(), 256));
}

#[test]
fn batch_sealing_boundary() {
    // The batch root is written at the end of the batch's last block.
    assert!(!seals_batch(0));
    assert!(seals_batch(BATCH_SIZE - 1));
    assert!(!seals_batch(BATCH_SIZE));
    assert!(seals_batch(2 * BATCH_SIZE - 1));

    // The sealing block belongs to the batch it seals, and its own ring root is
    // one of that batch's leaves.
    assert_eq!(batch_slot_for_block(BATCH_SIZE - 1), batch_slot(0));
    assert_eq!(batch_slot_for_block(BATCH_SIZE), batch_slot(1));

    // RING_SIZE == BATCH_SIZE is what guarantees every root of a batch is still
    // unoverwritten in the ring when the batch is sealed.
    assert_eq!(RING_SIZE, BATCH_SIZE);
}

// ---------------------------------------------------------------------------
// EIP-8312 activation predicate.
//
// One predicate feeds every gate (mode-5 admissibility, execution dispatch,
// vault provisioning, the openings-root block-end operation, mempool admission).
// Divergent predicates between admission and execution are a documented stall
// class on this codebase, so the predicate's own behavior is pinned here.
// ---------------------------------------------------------------------------

fn config_with(hegota: Option<u64>, utxo: Option<u64>) -> ChainConfig {
    ChainConfig {
        hegota_time: hegota,
        utxo_frames_time: utxo,
        ..Default::default()
    }
}

#[test]
fn utxo_activation_requires_its_own_timestamp() {
    // Absent on every network and fixture that has not opted in: EIP-8312 is
    // then entirely inactive, however far past Hegota the chain is.
    let no_knob = config_with(Some(100), None);
    assert!(!no_knob.is_utxo_frames_activated(100));
    assert!(!no_knob.is_utxo_frames_activated(u64::MAX));

    // Present: active from that timestamp, inclusive.
    let scheduled = config_with(Some(100), Some(500));
    assert!(!scheduled.is_utxo_frames_activated(499));
    assert!(scheduled.is_utxo_frames_activated(500));
    assert!(scheduled.is_utxo_frames_activated(501));
}

#[test]
fn utxo_activation_also_requires_hegota() {
    // A UTXO frame is a frame inside an EIP-8141 frame transaction, so EIP-8312
    // cannot be active on a chain that never schedules Hegota — otherwise the
    // vault would be installed and openings roots written on a chain where no
    // UTXO frame could ever be carried.
    let no_hegota = config_with(None, Some(500));
    assert!(!no_hegota.is_utxo_frames_activated(500));
    assert!(!no_hegota.is_utxo_frames_activated(u64::MAX));

    // Knob earlier than Hegota: not active until Hegota itself is live.
    let knob_first = config_with(Some(300), Some(100));
    assert!(!knob_first.is_utxo_frames_activated(100));
    assert!(!knob_first.is_utxo_frames_activated(299));
    assert!(knob_first.is_utxo_frames_activated(300));
}

#[test]
fn utxo_activation_resolves_hegota_through_the_fork_ordinal() {
    // The Hegotá half must be resolved the way execution resolves it — through the
    // fork ordinal — not through a per-field `hegota_time` check. The two coincide
    // only while Hegotá is the newest fork. Once a successor exists, a chain that
    // schedules it without an explicit `hegotaTime` has `fork >= Hegota` while the
    // field is `None`, and a field-based gate would diverge from execution: the
    // admission-vs-consensus stall class this branch converted every other gate
    // away from.
    //
    // The expectation is derived from `get_fork` rather than restated as literals,
    // so this test starts failing the moment the predicate reverts to a field check
    // on a chain that has a fork above Hegotá.
    for (hegota, utxo, timestamp) in [
        (Some(100u64), Some(500u64), 499u64),
        (Some(100), Some(500), 500),
        (None, Some(500), u64::MAX),
        (Some(300), Some(100), 299),
        (Some(300), Some(100), 300),
    ] {
        let config = config_with(hegota, utxo);
        let expected = config.utxo_frames_time.is_some_and(|t| t <= timestamp)
            && config.get_fork(timestamp) >= Fork::Hegota;
        assert_eq!(
            config.is_utxo_frames_activated(timestamp),
            expected,
            "the predicate must equal (knob scheduled AND fork >= Hegota); \
             hegota={hegota:?} utxo={utxo:?} timestamp={timestamp}"
        );
    }
}

#[test]
fn utxo_activation_is_inert_on_a_default_config() {
    // Every existing network and fixture: no field set, nothing active. This is
    // what makes landing the implementation a no-op until someone opts in.
    let default = ChainConfig::default();
    assert!(!default.is_utxo_frames_activated(0));
    assert!(!default.is_utxo_frames_activated(u64::MAX));
}

// ---------------------------------------------------------------------------
// EIP-8141 §Structural Rules rule 8 and §Expiry Verifier Frame placement
// ---------------------------------------------------------------------------

/// A `user_op` frame: SENDER mode, no approval scope. Legal after the prefix.
fn user_op_frame() -> Frame {
    Frame {
        mode: FrameMode::Sender as u8,
        flags: 0x00,
        target: Some(Address::from_low_u64_be(0x1234)),
        gas_limit: 10_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

#[test]
fn prefix_rejection_verify_frame_after_prefix() {
    // A `pay` frame trailing a complete `self_verify` prefix is a VERIFY frame
    // outside the prefix: its revert would invalidate the whole transaction
    // against state the prefix simulation never inspected.
    let tx = base_frame_tx_with_frames(vec![self_verify_frame(), pay_frame()]);
    let prefix = tx.validation_prefix().expect("SelfVerify recognized");
    assert_eq!(prefix.shape, PrefixShape::SelfVerify);
    assert_eq!(
        tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
            .unwrap_err(),
        FrameValidationError::VerifyFrameAfterPrefix { frame_index: 1 }
    );
}

#[test]
fn prefix_accepts_non_verify_frames_after_prefix() {
    // Rule 8 bans only VERIFY frames after the prefix; `user_op` (SENDER) and
    // `post_op` (DEFAULT) frames may follow in any number.
    let post_op = Frame {
        mode: FrameMode::Default as u8,
        flags: 0x00,
        target: Some(Address::from_low_u64_be(0x5678)),
        gas_limit: 10_000,
        state_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    };
    let tx = base_frame_tx_with_frames(vec![
        self_verify_frame(),
        user_op_frame(),
        post_op,
        user_op_frame(),
    ]);
    let prefix = tx.validation_prefix().expect("SelfVerify recognized");
    tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
        .expect("non-VERIFY frames after the prefix are allowed");
}

#[test]
fn prefix_rejection_expiry_frame_not_first() {
    // An expiry verifier frame may appear only as the first frame of the list.
    let tx = base_frame_tx_with_frames(vec![
        self_verify_frame(),
        expiry_verifier_frame(),
        user_op_frame(),
    ]);
    let prefix = tx.validation_prefix().expect("SelfVerify recognized");
    assert_eq!(
        tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
            .unwrap_err(),
        FrameValidationError::ExpiryFrameNotFirst { frame_index: 1 }
    );
}

#[test]
fn prefix_accepts_expiry_frame_as_first_frame() {
    let tx = base_frame_tx_with_frames(vec![
        expiry_verifier_frame(),
        self_verify_frame(),
        user_op_frame(),
    ]);
    let prefix = tx.validation_prefix().expect("SelfVerify recognized");
    tx.validate_prefix_structure(&prefix, FRAME_TX_MAX_VERIFY_GAS)
        .expect("a leading expiry verifier frame is valid");
}

// ---------------------------------------------------------------------------
// Blob-carrying frame transactions on the wire (EIP-8141 §Networking)
// ---------------------------------------------------------------------------

/// A blob-carrying frame tx fixture plus a matching sidecar shape. The bundle
/// contents are not KZG-valid; these tests cover the wire discrimination, which
/// runs before any cryptographic check.
fn frame_tx_with_sidecar_shape() -> (FrameTransaction, BlobsBundle) {
    let mut tx = make_test_frame_tx();
    let mut hash = [0xABu8; 32];
    hash[0] = VERSIONED_HASH_VERSION_KZG;
    tx.blob_versioned_hashes = vec![H256(hash)];
    tx.max_fee_per_blob_gas = U256::from(7u64);
    let bundle = BlobsBundle {
        blobs: vec![],
        commitments: vec![],
        proofs: vec![],
        version: 1,
    };
    (tx, bundle)
}

#[test]
fn p2p_blob_carrying_frame_transaction_roundtrips_wrapped() {
    // EIP-8141 §Networking: a frame tx with blobs is wrapped per EIP-7594, as
    // `[tx_payload_body, wrapper_version, blobs, commitments, cell_proofs]`.
    let (tx, blobs_bundle) = frame_tx_with_sidecar_shape();
    let original = P2PTransaction::FrameTransactionWithBlobs(WrappedFrameTransaction {
        tx,
        wrapper_version: Some(1),
        blobs_bundle,
    });

    let encoded = original.encode_to_vec();
    let (decoded, rest) = P2PTransaction::decode_unfinished(&encoded).unwrap();
    assert!(rest.is_empty());
    assert_eq!(decoded, original);
    assert_eq!(decoded.tx_type(), TxType::Frame);
    // `encode_canonical_len` must account for the sidecar, since the wrapped
    // variant encodes it (the announced pooled-tx size depends on this).
    assert_eq!(
        decoded.encode_canonical_len(),
        decoded.encode_canonical_to_vec().len()
    );

    // The sidecar is not part of the transaction's identity: the hash matches
    // the same transaction sent unwrapped.
    let P2PTransaction::FrameTransactionWithBlobs(ref wrapped) = decoded else {
        panic!("expected the wrapped variant");
    };
    assert_eq!(
        decoded.compute_hash(),
        P2PTransaction::FrameTransaction(wrapped.tx.clone()).compute_hash(),
    );

    // Converting to a plain Transaction would drop the bundle, so it is refused.
    let as_tx: Result<Transaction, _> = decoded.try_into();
    assert!(as_tx.is_err());
}

#[test]
fn p2p_blob_carrying_frame_transaction_must_be_wrapped() {
    // Sent unwrapped, a frame tx that declares blobs is rejected: its sidecar
    // could never be recovered.
    let (tx, _) = frame_tx_with_sidecar_shape();
    let encoded = P2PTransaction::FrameTransaction(tx).encode_to_vec();
    let err = P2PTransaction::decode_unfinished(&encoded).unwrap_err();
    assert!(
        format!("{err:?}").contains("must be wrapped with its sidecar"),
        "unexpected error: {err:?}"
    );
}

#[test]
fn p2p_blobless_frame_transaction_must_not_be_wrapped() {
    // And the converse: a frame tx with no blobs uses the plain payload, so a
    // sidecar on the wire is rejected rather than silently ignored.
    let wrapped = WrappedFrameTransaction {
        tx: make_test_frame_tx(),
        wrapper_version: Some(1),
        blobs_bundle: BlobsBundle {
            blobs: vec![],
            commitments: vec![],
            proofs: vec![],
            version: 1,
        },
    };
    let mut encoded = vec![TxType::Frame as u8];
    wrapped.encode(&mut encoded);
    let mut framed = Vec::new();
    <[u8] as RLPEncode>::encode(&encoded, &mut framed);
    let err = P2PTransaction::decode_unfinished(&framed).unwrap_err();
    assert!(
        format!("{err:?}").contains("must not carry a sidecar"),
        "unexpected error: {err:?}"
    );
}
