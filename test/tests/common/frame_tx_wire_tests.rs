//! EIP-8141 wire-format pin, cross-checked against the Python encoder.
//!
//! Nothing in this suite pinned the frame-transaction encoding before: every frame-tx test
//! either round-trips (so a changed layout stays symmetric and invisible) or asserts
//! behaviour. Changing the envelope therefore passed 1367 tests without a murmur, while
//! every joiner's transaction builder would have broken. This file pins the bytes.
//!
//! The same vector is asserted by `scripts/hegota-testnet/frametx.py`, so the two
//! independent encoders — the client's and the one joiners are handed — are held to one
//! set of bytes. If this test and that script disagree, one of them is wrong and the chain
//! has two truths.

use ethrex_common::types::{
    FRAME_SIG_SCHEME_SECP256K1, Frame, FrameMode, FrameSignature, FrameTransaction, Transaction,
};
use ethrex_common::{Address, Bytes, U256};
use ethrex_rlp::encode::RLPEncode;

/// The golden vector: two frames (a targetless VERIFY carrying data, then a SENDER frame
/// with a target), one SECP256K1 signature entry with an empty msg, and a state budget on
/// neither frame — the shape `frametx.py`'s `__main__` builds.
fn golden() -> FrameTransaction {
    FrameTransaction {
        chain_id: 1,
        nonce_keys: vec![U256::zero()],
        nonce_seq: 7,
        sender: Address::from_low_u64_be(0xABCD),
        frames: vec![
            Frame {
                mode: FrameMode::Verify as u8,
                flags: 3,
                target: None,
                gas_limit: 0x5208,
                state_limit: 0,
                value: U256::zero(),
                data: Bytes::from_static(&[0x11, 0x22]),
            },
            Frame {
                mode: FrameMode::Sender as u8,
                flags: 0,
                target: Some(Address::from_low_u64_be(0x1234)),
                gas_limit: 0x9c40,
                state_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
            },
        ],
        signatures: vec![FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(Address::from_low_u64_be(0xABCD)),
            msg: Bytes::new(),
            signature: Bytes::from(vec![0x01u8; 65]),
        }],
        max_priority_fee_per_gas: 0x3b9aca00,
        max_fee_per_gas: 0x6fc23ac00,
        max_fee_per_blob_gas: U256::zero(),
        blob_versioned_hashes: vec![],
        recent_root_references: vec![],
        ..Default::default()
    }
}

/// Byte-for-byte what `scripts/hegota-testnet/frametx.py` produces for `golden()`.
///
/// **These two strings must not move again.** The encoding is settled; every later step of
/// adopting the updated spec is semantics. If a gas, receipt or opcode change moves the golden
/// vector, an encoding change leaked into a step that had no business touching the wire,
/// and the right response is to find it rather than to re-pin these constants.
const GOLDEN_RLP: &str = "f8b301c1800794000000000000000000000000000000000000abcdeccc010380c48252088080821122de0280940000000000000000000000000000000000001234c4829c40808080f85cf85a0194000000000000000000000000000000000000abcd80b8410101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101cc843b9aca008506fc23ac0080c0c0";
const GOLDEN_SIG_HASH: &str = "0xd4df51143828c0338882dbd10c3308f3569972fe1928a7b5040ee18057920510";

#[test]
fn the_v2_envelope_encodes_to_the_golden_vector() {
    let tx = golden();
    let mut encoded = vec![];
    tx.encode(&mut encoded);
    assert_eq!(
        hex::encode(&encoded),
        GOLDEN_RLP,
        "the envelope must match the vector frametx.py asserts"
    );
}

#[test]
fn the_v2_sig_hash_matches_the_golden_vector() {
    assert_eq!(
        format!("{:#x}", golden().compute_sig_hash()),
        GOLDEN_SIG_HASH,
        "sig_hash covers the envelope, so nesting the fees changes it"
    );
}

/// Walk one RLP item and return `(payload, rest, is_list)` without interpreting it, so the
/// envelope's *shape* can be asserted independently of any Rust type.
fn rlp_item(input: &[u8]) -> (&[u8], &[u8], bool) {
    let first = input[0];
    match first {
        0x00..=0x7f => (&input[..1], &input[1..], false),
        0x80..=0xb7 => {
            let n = (first - 0x80) as usize;
            (&input[1..1 + n], &input[1 + n..], false)
        }
        0xb8..=0xbf => {
            let len_len = (first - 0xb7) as usize;
            let n = input[1..1 + len_len]
                .iter()
                .fold(0usize, |acc, b| (acc << 8) | *b as usize);
            (
                &input[1 + len_len..1 + len_len + n],
                &input[1 + len_len + n..],
                false,
            )
        }
        0xc0..=0xf7 => {
            let n = (first - 0xc0) as usize;
            (&input[1..1 + n], &input[1 + n..], true)
        }
        _ => {
            let len_len = (first - 0xf7) as usize;
            let n = input[1..1 + len_len]
                .iter()
                .fold(0usize, |acc, b| (acc << 8) | *b as usize);
            (
                &input[1 + len_len..1 + len_len + n],
                &input[1 + len_len + n..],
                true,
            )
        }
    }
}

fn rlp_children(payload: &[u8]) -> Vec<(&[u8], bool)> {
    let mut out = vec![];
    let mut rest = payload;
    while !rest.is_empty() {
        let (item, tail, is_list) = rlp_item(rest);
        out.push((item, is_list));
        rest = tail;
    }
    out
}

/// The nesting is the whole point of EIP-8141's envelope change, so assert the *shape* and not
/// only the bytes: nine top-level fields, a two-element `limits` list inside each frame
/// where a scalar gas limit used to sit, and a three-element `fees` list. A pin on bytes
/// alone would still pass if the layout were right by accident.
#[test]
fn the_v2_envelope_nests_limits_and_fees() {
    let mut encoded = vec![];
    golden().encode(&mut encoded);
    let (payload, rest, is_list) = rlp_item(&encoded);
    assert!(is_list && rest.is_empty(), "the envelope is one RLP list");

    let fields = rlp_children(payload);
    assert_eq!(
        fields.len(),
        9,
        "chain_id, nonce_keys, nonce_seq, sender, frames, signatures, fees, blob_hashes, recent_root_references"
    );

    // field 4 is the frame list; each frame is [mode, flags, target, limits, value, data]
    let frames = rlp_children(fields[4].0);
    assert_eq!(frames.len(), 2, "two frames");
    for (frame, _) in &frames {
        let parts = rlp_children(frame);
        assert_eq!(parts.len(), 6, "a frame is a six-tuple");
        assert!(
            parts[3].1,
            "the fourth field is the `limits` list, not a scalar"
        );
        assert_eq!(
            rlp_children(parts[3].0).len(),
            2,
            "limits = [execution, state]"
        );
    }

    // field 6 is the fees list
    assert!(fields[6].1, "fees is a list");
    assert_eq!(
        rlp_children(fields[6].0).len(),
        3,
        "fees = [max_priority_fee_per_gas, max_fee_per_gas, max_fee_per_blob_gas]"
    );
}

#[test]
fn the_v2_envelope_round_trips_through_the_canonical_decoder() {
    let tx = golden();
    let mut raw = vec![0x06u8];
    tx.encode(&mut raw);
    let decoded = Transaction::decode_canonical(&raw).expect("decodes as a typed transaction");
    let Transaction::FrameTransaction(decoded) = decoded else {
        panic!("decoded to the wrong transaction type");
    };
    assert_eq!(decoded.frames[0].gas_limit, 0x5208);
    assert_eq!(decoded.frames[0].state_limit, 0);
    assert_eq!(decoded.max_fee_per_gas, 0x6fc23ac00);
    assert_eq!(decoded.recent_root_references.len(), 0);
}

/// A frame carrying a state budget must survive the round trip, since `limits.state` is
/// the field EIP-8141 adds and the one an old decoder cannot see.
#[test]
fn a_state_budget_round_trips() {
    let mut tx = golden();
    tx.frames[1].state_limit = 4_000_000;
    let mut raw = vec![0x06u8];
    tx.encode(&mut raw);
    let Transaction::FrameTransaction(decoded) =
        Transaction::decode_canonical(&raw).expect("decodes")
    else {
        panic!("wrong type")
    };
    assert_eq!(decoded.frames[1].state_limit, 4_000_000);
    assert_eq!(
        decoded.state_gas_limit(),
        4_000_000,
        "the transaction's state dimension is the sum of its frames'"
    );
}
