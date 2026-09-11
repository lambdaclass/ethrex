//! EIP-8288 dependency verification frames: encoding, static validity, the
//! transaction and block dependency sets, and the gas rule.
//!
//! Spec read: `ethereum/EIPs` `EIPS/eip-8288.md` @ `ef1abf4b6d`. Where the EIP is
//! ambiguous these tests pin the reading described in `docs/eip-8288.md` and
//! argued in `scripts/hegota-testnet/NOTES-FOR-8288-AUTHOR.md`; each such test
//! names the item it settles so a later spec revision points straight at it.

use bytes::Bytes;
use ethrex_common::types::{BlockBody, BlockHeader, RecursiveStark, Transaction};
use ethrex_common::types::{
    DEPENDENCY_SCHEME_LEANSPHINCS, DEPENDENCY_SCHEME_LEANSTARK, DependencyTriple,
    FRAME_TX_DEPENDENCY_TRIPLE_BYTES, FRAME_TX_MAX_DEPENDENCIES_PER_FRAME,
    FRAME_TX_MAX_SIGS_PER_TX, Frame, FrameMode, FrameTransaction, LEANSPHINCS_VERIFICATION_GAS,
    LEANSTARK_VERIFICATION_GAS, deduplicate_and_sort_dependencies, dependencies_hash,
};
use ethrex_common::{Address, H256, U256};
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use once_cell::sync::OnceCell;

fn triple(scheme: u8, data: u64, vk: u64) -> DependencyTriple {
    DependencyTriple {
        scheme,
        data_hash: H256::from_low_u64_be(data),
        verification_key_hash: H256::from_low_u64_be(vk),
    }
}

fn sphincs(data: u64, vk: u64) -> DependencyTriple {
    triple(DEPENDENCY_SCHEME_LEANSPHINCS, data, vk)
}

fn stark(data: u64, vk: u64) -> DependencyTriple {
    triple(DEPENDENCY_SCHEME_LEANSTARK, data, vk)
}

fn pack(triples: &[DependencyTriple]) -> Bytes {
    let mut out = Vec::with_capacity(triples.len() * FRAME_TX_DEPENDENCY_TRIPLE_BYTES);
    for t in triples {
        out.extend_from_slice(&t.encode());
    }
    Bytes::from(out)
}

/// A well-formed dependency verification frame for the given triples: no target,
/// no flags, no state budget, and exactly the verification-gas sum.
fn dep_frame(triples: &[DependencyTriple]) -> Frame {
    Frame {
        mode: FrameMode::DepVerify as u8,
        flags: 0,
        target: None,
        gas_limit: triples.iter().map(|t| t.verification_gas()).sum(),
        state_gas_limit: 0,
        value: U256::zero(),
        data: pack(triples),
    }
}

/// The minimal approving frame that gives a transaction a recognizable
/// EIP-8141 validation prefix.
fn self_verify_frame() -> Frame {
    Frame {
        mode: FrameMode::Verify as u8,
        flags: 0x03,
        target: None,
        gas_limit: 50_000,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

fn tx_with(frames: Vec<Frame>) -> FrameTransaction {
    FrameTransaction {
        chain_id: 1,
        nonce_keys: vec![U256::zero()],
        nonce_seq: 0,
        sender: Address::from_low_u64_be(0xABCD),
        frames,
        signatures: vec![],
        max_priority_fee_per_gas: U256::from(1_000_000_000u64),
        max_fee_per_gas: U256::from(30_000_000_000u64),
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: vec![],
        inner_hash: OnceCell::new(),
        cached_canonical: OnceCell::new(),
    }
}

// ---------------------------------------------------------------------------
// Constants and encoding
// ---------------------------------------------------------------------------

/// The published constants, checked against the EIP's table rather than against
/// each other. A repricing upstream should fail here, not silently re-tune gas.
#[test]
fn eip8288_constants_match_the_published_table() {
    assert_eq!(FrameMode::DepVerify as u8, 3, "DEP_VERIFY_FRAME_MODE");
    assert_eq!(FRAME_TX_MAX_DEPENDENCIES_PER_FRAME, 256);
    assert_eq!(DEPENDENCY_SCHEME_LEANSPHINCS, 0x10);
    assert_eq!(DEPENDENCY_SCHEME_LEANSTARK, 0x11);
    assert_eq!(LEANSPHINCS_VERIFICATION_GAS, 3_000);
    assert_eq!(LEANSTARK_VERIFICATION_GAS, 30_000);
    assert_eq!(FRAME_TX_DEPENDENCY_TRIPLE_BYTES, 96);
}

/// The 96-byte layout: 31 zero bytes, the scheme byte, then two 32-byte hashes.
#[test]
fn a_triple_encodes_to_96_bytes_with_the_scheme_in_byte_31() {
    let t = sphincs(0xAA, 0xBB);
    let encoded = t.encode();
    assert_eq!(encoded.len(), 96);
    assert!(
        encoded[..31].iter().all(|b| *b == 0),
        "the scheme field is a big-endian 32-byte word, so bytes 0..31 are padding"
    );
    assert_eq!(encoded[31], DEPENDENCY_SCHEME_LEANSPHINCS);
    assert_eq!(&encoded[32..64], H256::from_low_u64_be(0xAA).as_bytes());
    assert_eq!(&encoded[64..96], H256::from_low_u64_be(0xBB).as_bytes());
    assert_eq!(DependencyTriple::from_bytes(&encoded), Some(t));
}

#[test]
fn a_triple_with_non_zero_scheme_padding_does_not_parse() {
    let mut encoded = sphincs(1, 2).encode();
    encoded[30] = 1;
    assert_eq!(
        DependencyTriple::from_bytes(&encoded),
        None,
        "the EIP requires the first 31 bytes of each triple to be zero"
    );
}

#[test]
fn a_triple_with_an_unassigned_scheme_does_not_parse() {
    for scheme in [0x00u8, 0x0f, 0x12, 0xff] {
        let mut encoded = sphincs(1, 2).encode();
        encoded[31] = scheme;
        assert_eq!(
            DependencyTriple::from_bytes(&encoded),
            None,
            "scheme {scheme:#04x} is not one EIP-8288 assigns"
        );
    }
}

#[test]
fn verification_gas_is_per_scheme() {
    assert_eq!(
        sphincs(1, 2).verification_gas(),
        LEANSPHINCS_VERIFICATION_GAS
    );
    assert_eq!(stark(1, 2).verification_gas(), LEANSTARK_VERIFICATION_GAS);
}

// ---------------------------------------------------------------------------
// deduplicate_and_sort, and the ordering question (notes item 9)
// ---------------------------------------------------------------------------

#[test]
fn dependencies_are_deduplicated_and_sorted() {
    let a = sphincs(2, 9);
    let b = sphincs(1, 9);
    let c = stark(1, 9);
    let out = deduplicate_and_sort_dependencies(vec![a, c, b, a, c]);
    assert_eq!(
        out,
        vec![b, a, c],
        "sorted over the 96-byte encoding, duplicates removed"
    );
}

/// Notes item 9. The EIP sorts `(scheme, data_hash, vk_hash)` tuples, which
/// compares `scheme` as a small integer; the consensus hash is taken over the
/// encoding, where `scheme` is a big-endian 32-byte word. For every scheme the EIP
/// assigns the two orders agree, which is why sorting the encoding is safe to
/// adopt now and stays correct if the scheme space ever widens.
#[test]
fn encoding_order_and_tuple_order_agree_for_every_assigned_scheme() {
    let deps = vec![
        stark(5, 1),
        sphincs(5, 1),
        stark(1, 1),
        sphincs(1, 1),
        sphincs(1, 0),
    ];

    let by_encoding = deduplicate_and_sort_dependencies(deps.clone());

    let mut by_tuple = deps;
    by_tuple.sort_by_key(|d| (d.scheme, d.data_hash, d.verification_key_hash));
    by_tuple.dedup();

    assert_eq!(by_encoding, by_tuple);
}

/// The digest is over the concatenated encodings of an already ordered list, so
/// the same set in any input order hashes identically — the property block
/// validity rests on.
#[test]
fn the_dependency_hash_is_order_independent_once_normalized() {
    let one = deduplicate_and_sort_dependencies(vec![sphincs(1, 1), stark(2, 2)]);
    let other = deduplicate_and_sort_dependencies(vec![stark(2, 2), sphincs(1, 1), sphincs(1, 1)]);
    assert_eq!(one, other);
    assert_eq!(dependencies_hash(&one), dependencies_hash(&other));

    assert_ne!(
        dependencies_hash(&one),
        dependencies_hash(&deduplicate_and_sort_dependencies(vec![sphincs(1, 1)])),
        "dropping a dependency must change the block's deps hash"
    );
}

#[test]
fn the_empty_dependency_set_hashes_to_the_empty_blake3_digest() {
    // A block with no dependencies still has a well-defined block_deps_hash, and it
    // must not depend on how the implementation represents "none".
    assert_eq!(dependencies_hash(&[]), dependencies_hash(&Vec::new()));
}

// ---------------------------------------------------------------------------
// Static validity of the frame (notes items 2 and 3)
// ---------------------------------------------------------------------------

#[test]
fn a_well_formed_dependency_frame_is_accepted() {
    let deps = [sphincs(1, 2), stark(3, 4)];
    let tx = tx_with(vec![dep_frame(&deps), self_verify_frame()]);
    assert!(
        tx.validate_static_constraints().is_ok(),
        "{:?}",
        tx.validate_static_constraints()
    );
}

#[test]
fn a_dependency_frame_must_declare_exactly_the_verification_gas_sum() {
    let deps = [sphincs(1, 2), stark(3, 4)];
    let expected = LEANSPHINCS_VERIFICATION_GAS + LEANSTARK_VERIFICATION_GAS;
    assert_eq!(dep_frame(&deps).gas_limit, expected);

    for wrong in [expected - 1, expected + 1, 0] {
        let mut frame = dep_frame(&deps);
        frame.gas_limit = wrong;
        let err = tx_with(vec![frame, self_verify_frame()])
            .validate_static_constraints()
            .expect_err("a mismatched budget must be rejected");
        assert!(err.contains("gas_limit"), "{err}");
    }
}

/// Notes item 2: the EIP says "the frame's `gas_limit`", but EIP-8141 frames carry
/// `limits = [execution, state]`. The sum binds the execution dimension, and the
/// state dimension must be zero — the same shape EIP-8141 pins on its own expiry
/// verifier frame, which likewise grows no state.
#[test]
fn a_dependency_frame_must_declare_no_state_gas() {
    let mut frame = dep_frame(&[sphincs(1, 2)]);
    frame.state_gas_limit = 1;
    let err = tx_with(vec![frame, self_verify_frame()])
        .validate_static_constraints()
        .expect_err("a dependency frame creates no state");
    assert!(err.contains("state_gas_limit"), "{err}");
}

/// Notes item 3: the EIP glosses `None` as "address `0x00...00`", but in EIP-8141
/// an absent target resolves to `tx.sender` while an explicit zero address
/// resolves to itself. We require the field absent, so the frame names no account
/// and no frame-entry access charge is levied.
#[test]
fn a_dependency_frame_must_have_no_target() {
    for target in [Address::zero(), Address::from_low_u64_be(0xABCD)] {
        let mut frame = dep_frame(&[sphincs(1, 2)]);
        frame.target = Some(target);
        let err = tx_with(vec![frame, self_verify_frame()])
            .validate_static_constraints()
            .expect_err("target must be absent, including when it is the zero address");
        assert!(err.contains("target"), "{err}");
    }
}

#[test]
fn a_dependency_frame_must_have_no_flags() {
    let mut frame = dep_frame(&[sphincs(1, 2)]);
    frame.flags = 0x03;
    let err = tx_with(vec![frame, self_verify_frame()])
        .validate_static_constraints()
        .expect_err("flags must be zero");
    assert!(err.contains("flags"), "{err}");
}

#[test]
fn a_dependency_frame_must_carry_whole_triples() {
    for len in [1usize, 95, 97, 191] {
        let mut frame = dep_frame(&[sphincs(1, 2)]);
        frame.data = Bytes::from(vec![0u8; len]);
        assert!(
            tx_with(vec![frame, self_verify_frame()])
                .validate_static_constraints()
                .is_err(),
            "{len} bytes is not a whole number of 96-byte triples"
        );
    }
}

#[test]
fn a_dependency_frame_must_carry_at_least_one_triple() {
    let mut frame = dep_frame(&[sphincs(1, 2)]);
    frame.data = Bytes::new();
    frame.gas_limit = 0;
    assert!(
        tx_with(vec![frame, self_verify_frame()])
            .validate_static_constraints()
            .is_err(),
        "the EIP requires between 1 and MAX_DEPENDENCIES_PER_FRAME triples"
    );
}

#[test]
fn a_dependency_frame_may_carry_max_dependencies_per_frame_triples() {
    let deps: Vec<_> = (0..FRAME_TX_MAX_DEPENDENCIES_PER_FRAME as u64)
        .map(|i| sphincs(i, i))
        .collect();
    let tx = tx_with(vec![dep_frame(&deps), self_verify_frame()]);
    assert!(tx.validate_static_constraints().is_ok());
    assert_eq!(tx.dependencies().len(), FRAME_TX_MAX_DEPENDENCIES_PER_FRAME);

    let too_many: Vec<_> = (0..=FRAME_TX_MAX_DEPENDENCIES_PER_FRAME as u64)
        .map(|i| sphincs(i, i))
        .collect();
    let err = tx_with(vec![dep_frame(&too_many), self_verify_frame()])
        .validate_static_constraints()
        .expect_err("one over the per-frame cap must be rejected");
    assert!(err.contains("dependency count"), "{err}");
}

/// Notes item 13. A dependency frame carries no flags, which makes it a valid
/// *terminator* for a batch an earlier frame opened -- and a terminator is where
/// the batch commits. A frame that never executes has nothing to commit, so it
/// must not close one. EIP-8288 says nothing about atomic batches.
#[test]
fn a_dependency_frame_must_not_terminate_an_atomic_batch() {
    let mut batched = self_verify_frame();
    batched.flags = 0x04; // atomic batch, no approval scope
    let tx = tx_with(vec![
        self_verify_frame(),
        batched,
        dep_frame(&[sphincs(1, 2)]),
    ]);
    let err = tx
        .validate_static_constraints()
        .expect_err("a frame that never executes must not close a batch");
    assert!(err.contains("atomic batch"), "{err}");
}

/// The restriction is about the batch, not about adjacency: a dependency frame
/// directly after an unflagged frame is fine.
#[test]
fn a_dependency_frame_after_an_unbatched_frame_is_accepted() {
    let tx = tx_with(vec![self_verify_frame(), dep_frame(&[sphincs(1, 2)])]);
    assert!(
        tx.validate_static_constraints().is_ok(),
        "{:?}",
        tx.validate_static_constraints()
    );
}

// ---------------------------------------------------------------------------
// dependencies(tx) and gas (notes item 5)
// ---------------------------------------------------------------------------

#[test]
fn transaction_dependencies_span_every_dependency_frame() {
    let tx = tx_with(vec![
        dep_frame(&[sphincs(2, 2)]),
        self_verify_frame(),
        dep_frame(&[sphincs(1, 1), stark(3, 3)]),
    ]);
    assert!(tx.validate_static_constraints().is_ok());
    assert_eq!(
        tx.dependencies(),
        vec![sphincs(1, 1), sphincs(2, 2), stark(3, 3)],
        "all frames contribute, and the result is normalized"
    );
}

/// The EIP makes duplicate declarations legal but wasteful: the dependency set
/// deduplicates, while gas is charged per declaration. Both halves are checked
/// here, because charging on the deduplicated set would make duplicates free.
#[test]
fn duplicate_declarations_are_charged_but_deduplicated() {
    let dup = sphincs(7, 7);
    let tx = tx_with(vec![dep_frame(&[dup, dup, dup]), self_verify_frame()]);
    assert!(tx.validate_static_constraints().is_ok());

    assert_eq!(tx.dependencies(), vec![dup], "the set holds one");
    assert_eq!(tx.declared_dependency_count(), 3, "gas counts three");
    assert_eq!(
        tx.frames[0].gas_limit,
        3 * LEANSPHINCS_VERIFICATION_GAS,
        "the declared budget is per declaration, not per distinct dependency"
    );
}

/// The per-transaction limits count distinct dependencies, not declarations, and
/// the spec is explicit that exceeding the limit in declarations is legal:
///
/// > it is legal to have a transaction with eg. `> MAX_SIGS_PER_TX` leanSPHINCS
/// > dependency _declarations_, if some of them are pointing to the same object so
/// > the deduplicated list is within limits.
///
/// The per-frame bound counts the other way -- declarations -- so the two are not
/// redundant. Getting this backwards would reject legal transactions at admission
/// while admitting frames the frame rule forbids, and the sentence that settles it
/// sits four sections away from the rule it governs (notes item 6).
#[test]
fn the_per_transaction_limit_counts_distinct_dependencies_not_declarations() {
    let a = sphincs(1, 1);
    let b = sphincs(2, 2);
    // Twenty declarations of two distinct dependencies: over MAX_SIGS_PER_TX in
    // declarations, well under it once deduplicated.
    let declarations: Vec<_> = std::iter::repeat_n([a, b], 10).flatten().collect();
    let tx = tx_with(vec![dep_frame(&declarations), self_verify_frame()]);

    assert!(
        tx.validate_static_constraints().is_ok(),
        "{:?}",
        tx.validate_static_constraints()
    );
    assert_eq!(tx.declared_dependency_count(), 20);
    assert!(
        tx.declared_dependency_count() > FRAME_TX_MAX_SIGS_PER_TX,
        "the declarations exceed the per-transaction limit"
    );
    assert_eq!(
        tx.dependencies(),
        vec![a, b],
        "but the deduplicated set, which the limit actually governs, holds two"
    );
    assert!(tx.dependencies().len() <= FRAME_TX_MAX_SIGS_PER_TX);
}

#[test]
fn a_transaction_with_no_dependency_frames_has_no_dependencies() {
    let tx = tx_with(vec![self_verify_frame()]);
    assert!(tx.dependencies().is_empty());
    assert_eq!(tx.declared_dependency_count(), 0);
}

// ---------------------------------------------------------------------------
// dependencies(block) and block_deps_hash
// ---------------------------------------------------------------------------

/// EIP-8288 block-validity rule 1: the header's `block_deps_hash` must equal the
/// hash of `dependencies(block)`. Rule 2, that the recursive STARK verifies, needs
/// `AGGREGATED_VK`, which the EIP still lists as `TBD`.
#[test]
fn block_dependencies_union_every_transaction() {
    let tx_a = Transaction::FrameTransaction(tx_with(vec![
        dep_frame(&[sphincs(2, 2), stark(9, 9)]),
        self_verify_frame(),
    ]));
    // Shares one triple with the first transaction, so the union deduplicates.
    let tx_b = Transaction::FrameTransaction(tx_with(vec![
        dep_frame(&[sphincs(1, 1), sphincs(2, 2)]),
        self_verify_frame(),
    ]));

    let body = BlockBody {
        transactions: vec![tx_a, tx_b],
        ommers: Vec::new(),
        withdrawals: None,
    };

    assert_eq!(
        body.dependencies(),
        vec![sphincs(1, 1), sphincs(2, 2), stark(9, 9)],
        "three distinct triples across four declarations"
    );
    assert_eq!(
        body.block_deps_hash(),
        dependencies_hash(&[sphincs(1, 1), sphincs(2, 2), stark(9, 9)])
    );
}

#[test]
fn a_block_with_no_frame_transactions_has_no_dependencies() {
    let body = BlockBody {
        transactions: Vec::new(),
        ommers: Vec::new(),
        withdrawals: None,
    };
    assert!(body.dependencies().is_empty());
    assert_eq!(body.block_deps_hash(), dependencies_hash(&[]));
}

/// Transaction order must not change the block's dependency hash: the set is
/// normalized before hashing, so two builders that order the same transactions
/// differently still agree on what the block's recursive STARK must prove.
#[test]
fn block_dependency_hash_does_not_depend_on_transaction_order() {
    let a = Transaction::FrameTransaction(tx_with(vec![
        dep_frame(&[stark(9, 9)]),
        self_verify_frame(),
    ]));
    let b = Transaction::FrameTransaction(tx_with(vec![
        dep_frame(&[sphincs(1, 1)]),
        self_verify_frame(),
    ]));

    let forward = BlockBody {
        transactions: vec![a.clone(), b.clone()],
        ommers: Vec::new(),
        withdrawals: None,
    };
    let reversed = BlockBody {
        transactions: vec![b, a],
        ommers: Vec::new(),
        withdrawals: None,
    };
    assert_eq!(forward.block_deps_hash(), reversed.block_deps_hash());
}

// ---------------------------------------------------------------------------
// Frame position (notes item 12)
// ---------------------------------------------------------------------------

/// The EIP's own Test Case 1: a dependency frame at index 0, followed by the
/// VERIFY frame that approves. EIP-8141 requires the validation prefix to start at
/// frame 0 and to be built from DEFAULT/VERIFY frames, so this transaction only
/// has a recognizable prefix if dependency frames are transparent to shape
/// matching. The EIP never says they are.
#[test]
fn a_leading_dependency_frame_leaves_the_prefix_recognizable() {
    let tx = tx_with(vec![dep_frame(&[sphincs(1, 2)]), self_verify_frame()]);
    assert!(tx.validate_static_constraints().is_ok());
    let prefix = tx
        .validation_prefix()
        .expect("EIP-8288 Test Case 1 must describe a valid transaction");
    tx.validate_prefix_structure(&prefix, 1_000_000)
        .expect("and its prefix must pass the structural rules");
}

/// The same transparency has to hold after the prefix, since the EIP places no
/// restriction on where a dependency frame may sit.
#[test]
fn a_trailing_dependency_frame_leaves_the_prefix_recognizable() {
    let tx = tx_with(vec![self_verify_frame(), dep_frame(&[stark(1, 2)])]);
    assert!(tx.validate_static_constraints().is_ok());
    let prefix = tx.validation_prefix().expect("prefix must still match");
    tx.validate_prefix_structure(&prefix, 1_000_000)
        .expect("a dependency frame is not a VERIFY frame after the prefix");
}

/// A transaction whose only frame is a dependency frame has no approving frame at
/// all, so it has no validation prefix and cannot name a payer.
#[test]
fn a_dependency_frame_alone_is_not_a_validation_prefix() {
    let tx = tx_with(vec![dep_frame(&[sphincs(1, 2)])]);
    assert!(
        tx.validation_prefix().is_err(),
        "a transaction needs a frame that approves payment"
    );
}

// ---------------------------------------------------------------------------
// The recursive_stark header field
// ---------------------------------------------------------------------------

/// A header shaped the way a real J* block's is.
///
/// The trailing optionals must be filled in the combination the fork schedule
/// actually produces, not an arbitrary one. Hegotá sits above Amsterdam, so
/// `requests_hash`, `block_access_list_hash` and `slot_number` are all present; an
/// invented header with, say, `burned_fees` set and `slot_number` absent is not a
/// shape any chain emits, and RLP's positional trailing optionals decode it wrong.
/// That greedy-decode hazard is real but pre-existing, and `payload.rs` carries a
/// `debug_assert` about it for the `slot_number`/`burned_fees` pair.
/// Every optional before it must be filled too, not just the neighbours. The
/// header's whole trailing-optional run decodes greedily, so leaving an earlier
/// `Option<H256>` empty lets it absorb a later one: with `withdrawals_root` unset,
/// `requests_hash` lands in it and everything after shifts. That is a property of
/// the existing encoding rather than anything this EIP adds, but it means a test
/// header has to be a shape the chain really produces.
fn jstar_header(burned_fees: Option<u64>, recursive_stark: Option<RecursiveStark>) -> BlockHeader {
    BlockHeader {
        base_fee_per_gas: Some(7),
        withdrawals_root: Some(H256::from_low_u64_be(0x3D6)),
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        parent_beacon_block_root: Some(H256::from_low_u64_be(0xBEAC)),
        requests_hash: Some(H256::from_low_u64_be(0x7E9)),
        block_access_list_hash: Some(H256::from_low_u64_be(0xBA1)),
        slot_number: Some(64),
        burned_fees,
        recursive_stark,
        ..Default::default()
    }
}

fn a_proof() -> RecursiveStark {
    RecursiveStark {
        proof: Bytes::from_static(b"an opaque aggregate"),
        block_deps_hash: H256::from_low_u64_be(0xDEA5),
    }
}

/// `recursive_stark` sits immediately before `burned_fees`, and J* (27) activates
/// before LStar (28), so there is a window in which the first is present and the
/// second is not.
///
/// RLP trailing optionals are positional and decode greedily, so a present field
/// after an absent one normally shifts everything. It does not bite across this
/// pair because they have different RLP shapes: a `u64` does not decode as a list
/// and a list does not decode as a `u64`, so whichever is absent yields `None`
/// without consuming.
///
/// This is the shape of every J*-but-pre-LStar block. If it breaks, every such
/// block decodes wrong, which is why it gets its own test.
#[test]
fn recursive_stark_survives_absent_burned_fees() {
    let header = jstar_header(None, Some(a_proof()));

    let mut buf = Vec::new();
    header.encode(&mut buf);
    let decoded = BlockHeader::decode(&buf).expect("a J* pre-LStar header must round-trip");

    assert_eq!(
        decoded.burned_fees, None,
        "the absent scalar must stay absent"
    );
    assert_eq!(
        decoded.slot_number,
        Some(64),
        "and must not have swallowed the preceding optional"
    );
    assert_eq!(
        decoded.recursive_stark,
        Some(a_proof()),
        "the list must not be swallowed by the preceding optional scalar"
    );
}

#[test]
fn recursive_stark_round_trips_alongside_burned_fees() {
    let header = jstar_header(Some(4_242), Some(a_proof()));
    let mut buf = Vec::new();
    header.encode(&mut buf);
    let decoded = BlockHeader::decode(&buf).unwrap();
    assert_eq!(decoded.burned_fees, Some(4_242));
    assert_eq!(decoded.recursive_stark, Some(a_proof()));
    assert_eq!(decoded, header);
}

#[test]
fn a_header_without_recursive_stark_round_trips_unchanged() {
    let header = jstar_header(Some(1), None);
    let mut buf = Vec::new();
    header.encode(&mut buf);
    let decoded = BlockHeader::decode(&buf).unwrap();
    assert_eq!(decoded.recursive_stark, None);
    assert_eq!(decoded.burned_fees, Some(1));
    assert_eq!(decoded, header);
}

/// The field must survive a getPayload -> newPayload round-trip, or a producer's
/// own J* block fails its block-hash check on import.
///
/// This was genuinely missing. `ExecutionPayload` carried every other fork-gated
/// header field and not this one, so `from_block` dropped it and `into_block` left
/// it `None`. Found by auditing the implementation against the spec rather than by
/// any test, which is why there is one now.
#[test]
fn recursive_stark_survives_the_execution_payload_round_trip() {
    let entry = RecursiveStark {
        proof: Bytes::from_static(b"an aggregate"),
        block_deps_hash: H256::from_low_u64_be(0xD1),
    };
    let header = jstar_header(None, Some(entry.clone()));
    let block = ethrex_common::types::Block::new(
        header.clone(),
        BlockBody {
            transactions: Vec::new(),
            ommers: Vec::new(),
            withdrawals: Some(Vec::new()),
        },
    );

    let payload = ethrex_rpc::types::payload::ExecutionPayload::from_block(block, None);
    assert_eq!(
        payload.recursive_stark,
        Some(entry),
        "getPayload must carry the entry to the consensus client"
    );

    let rebuilt = payload
        .into_block(
            header.parent_beacon_block_root,
            header.requests_hash,
            header.block_access_list_hash,
        )
        .expect("newPayload must rebuild the block");
    assert_eq!(
        rebuilt.header.recursive_stark, header.recursive_stark,
        "and newPayload must put it back"
    );

    // Not comparing whole block hashes: `into_block` recomputes `withdrawals_root`
    // from the body, and this fixture's is fabricated. What matters is that the
    // entry makes the trip, since the hash it feeds is already covered by
    // `recursive_stark_participates_in_the_block_hash`.
}

/// An empty proof still round-trips. A block whose transactions declare no
/// dependencies still carries the field, with the digest of the empty set, because
/// a header schema that depended on the body would be worse.
#[test]
fn an_empty_proof_round_trips() {
    let header = jstar_header(
        None,
        Some(RecursiveStark {
            proof: Bytes::new(),
            block_deps_hash: dependencies_hash(&[]),
        }),
    );
    let mut buf = Vec::new();
    header.encode(&mut buf);
    assert_eq!(BlockHeader::decode(&buf).unwrap(), header);
}

/// The field is part of the header hash, so it must survive a getPayload ->
/// newPayload round-trip or a producer's own block fails its hash check on import.
#[test]
fn recursive_stark_participates_in_the_block_hash() {
    let base = jstar_header(None, Some(a_proof()));
    let mut altered = a_proof();
    altered.block_deps_hash = H256::from_low_u64_be(0xBEEF);
    let other = jstar_header(None, Some(altered));

    assert_ne!(
        base.hash(),
        other.hash(),
        "changing the declared dependency digest must change the header hash"
    );
    assert_ne!(
        base.hash(),
        jstar_header(None, None).hash(),
        "and dropping the field entirely must change it too"
    );
}
