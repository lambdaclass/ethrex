//! Demonstrations for the EIP-8288 questions that experiment can settle.
//!
//! Each test here exists to make one candidate reading of the spec untenable, or to
//! measure something the spec asserts without data. They are evidence for a
//! discussion with the EIP authors rather than conformance checks, and each says
//! which question it bears on.

use bytes::Bytes;
use ethrex_common::types::{
    DEPENDENCY_SCHEME_LEANSPHINCS, DependencyTriple, Frame, FrameMode, FrameTransaction,
    LEANSPHINCS_VERIFICATION_GAS, PrefixShape,
};
use ethrex_common::{Address, U256};

const APPROVE_EXECUTION_AND_PAYMENT: u8 = 0x03;

fn triple(n: u64) -> DependencyTriple {
    DependencyTriple {
        scheme: DEPENDENCY_SCHEME_LEANSPHINCS,
        data_hash: ethrex_common::H256::from_low_u64_be(n),
        verification_key_hash: ethrex_common::H256::from_low_u64_be(n + 1000),
    }
}

fn dep_frame(count: u64) -> Frame {
    let mut data = Vec::new();
    for n in 0..count {
        data.extend_from_slice(&triple(n).encode());
    }
    Frame {
        mode: FrameMode::DepVerify as u8,
        flags: 0,
        target: None,
        gas_limit: count * LEANSPHINCS_VERIFICATION_GAS,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::from(data),
    }
}

fn self_verify_frame(gas: u64) -> Frame {
    Frame {
        mode: FrameMode::Verify as u8,
        flags: APPROVE_EXECUTION_AND_PAYMENT,
        target: None,
        gas_limit: gas,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

fn tx(frames: Vec<Frame>) -> FrameTransaction {
    FrameTransaction {
        chain_id: 1,
        nonce_keys: vec![U256::zero()],
        nonce_seq: 0,
        sender: Address::from_low_u64_be(0xABCD),
        frames,
        max_priority_fee_per_gas: U256::from(1u64),
        max_fee_per_gas: U256::from(1_000u64),
        ..Default::default()
    }
}

/// Question 2: where may a dependency frame sit?
///
/// EIP-8288's Test Cases 1 and 2 both put a `DEP_VERIFY` frame before the approving
/// `VERIFY` frame. EIP-8141 defines the validation prefix as the shortest run of
/// frames whose execution sets `payer`, and requires it to match one of four
/// recognized shapes, all built from deploy / self_verify / only_verify / pay.
///
/// So the reading decides whether the EIP's own examples can be broadcast. Under the
/// literal reading the prefix is `[dep_verify, self_verify]`, which matches nothing.
/// Under transparency it is `[self_verify]`, which is SelfVerify. There is no third
/// option, and the EIP states neither.
#[test]
fn test_case_1_shape_is_only_broadcastable_if_dependency_frames_are_transparent() {
    let t = tx(vec![dep_frame(1), self_verify_frame(50_000)]);
    let prefix = t
        .validation_prefix()
        .expect("a recognized prefix under the transparent reading");

    assert_eq!(
        prefix.shape,
        PrefixShape::SelfVerify,
        "the EIP's Test Case 1 shape is recognized only because the dependency \
         frame is skipped by shape matching"
    );
    assert_eq!(
        prefix.frame_indices,
        vec![1],
        "the dependency frame at index 0 is not part of the prefix; were it counted, \
         the prefix would be [dep_verify, self_verify] and match none of EIP-8141's \
         four recognized shapes, making Test Cases 1 and 2 unbroadcastable"
    );
}

/// Question 3: whose budget does a dependency frame's declared gas belong to?
///
/// EIP-8141 structural rule 6 sums `limits.execution` across the validation prefix
/// against `MAX_VERIFY_GAS`, which bounds the work a node does simulating the
/// prefix. A dependency frame causes none of that work: it never executes.
///
/// This pins the size of the mismatch rather than arguing about it. If dependency
/// frames counted, a single transaction at `MAX_SIGS_PER_TX` would claim just under
/// half the budget, and one frame at `MAX_DEPENDENCIES_PER_FRAME` leanSTARK
/// dependencies would claim 76 times it.
#[test]
fn dependency_gas_would_dominate_the_verify_budget_if_it_counted() {
    const MAX_VERIFY_GAS: u64 = 100_000;
    const MAX_SIGS_PER_TX: u64 = 16;
    const MAX_DEPENDENCIES_PER_FRAME: u64 = 256;
    const LEANSTARK_VERIFICATION_GAS: u64 = 30_000;

    let sixteen_sigs = MAX_SIGS_PER_TX * LEANSPHINCS_VERIFICATION_GAS;
    assert_eq!(sixteen_sigs, 48_000);
    assert!(
        sixteen_sigs * 2 < MAX_VERIFY_GAS * 3 / 2,
        "sixteen leanSPHINCS dependencies declare {sixteen_sigs}, near half of \
         MAX_VERIFY_GAS ({MAX_VERIFY_GAS})"
    );

    let full_frame_of_starks = MAX_DEPENDENCIES_PER_FRAME * LEANSTARK_VERIFICATION_GAS;
    assert_eq!(full_frame_of_starks, 7_680_000);
    assert!(
        full_frame_of_starks > MAX_VERIFY_GAS * 76,
        "one frame at MAX_DEPENDENCIES_PER_FRAME leanSTARK dependencies declares \
         {full_frame_of_starks}, over 76 times MAX_VERIFY_GAS"
    );

    // And the frame that declares all of it does no EVM work at all, so none of it
    // measures what MAX_VERIFY_GAS exists to bound.
    let t = tx(vec![
        dep_frame(MAX_SIGS_PER_TX),
        self_verify_frame(MAX_VERIFY_GAS - sixteen_sigs),
    ]);
    let prefix = t.validation_prefix().expect("recognized");
    assert!(
        !prefix.frame_indices.contains(&0),
        "this implementation keeps the dependency frame out of the budgeted prefix"
    );
}

/// Question 5: does a dependency verification frame get a receipt entry?
///
/// EIP-8288 says the frame is "not executed" and stops. EIP-8141 requires a
/// `frame_receipt = [status, gas_used, logs]` per frame, builds the transaction's
/// logs by concatenating them "in frame order", and exposes per-frame results
/// through `FRAMEPARAM` 0x05, 0x0A and 0x0B, which read by frame index.
///
/// So the two candidate readings are not symmetric: omitting the entry renumbers
/// every later frame's results. This makes that concrete by showing the offset a
/// contract would read, which is the argument that the entry is not optional.
#[test]
fn omitting_the_dependency_frame_receipt_renumbers_every_later_frame() {
    let frames = vec![dep_frame(1), self_verify_frame(50_000), dep_frame(2)];
    let t = tx(frames);

    let dependency_indices: Vec<usize> = t
        .frames
        .iter()
        .enumerate()
        .filter(|(_, f)| f.is_dependency_verification())
        .map(|(i, _)| i)
        .collect();
    assert_eq!(dependency_indices, vec![0, 2]);

    // With an entry per frame, receipt index equals frame index, which is what
    // FRAMEPARAM 0x05 / 0x0A / 0x0B assume when a contract walks the frame list.
    let with_entries: Vec<usize> = (0..t.frames.len()).collect();
    assert_eq!(with_entries, vec![0, 1, 2]);

    // Skipping the dependency frames instead maps receipt slots to frames 1 only,
    // so FRAMEPARAM(0x05, 1) would read the wrong frame and FRAMEPARAM(0x05, 2)
    // would be out of bounds on a transaction the EIP invites contracts to inspect.
    let without_entries: Vec<usize> = (0..t.frames.len())
        .filter(|i| !t.frames[*i].is_dependency_verification())
        .collect();
    assert_eq!(without_entries, vec![1]);
    assert_ne!(
        with_entries.len(),
        without_entries.len(),
        "the two readings disagree about how many receipt slots exist, so the EIP \
         has to say which, or every later frame's introspection shifts"
    );
}
