use ethrex_common::types::{BYTECODE_PADDING, Code};
use ethrex_crypto::NativeCrypto;

// A PUSH32 as the last opcode forces `pc` to step `BYTECODE_PADDING - 1` bytes
// past the real code end, so the deserialized `Code` must be re-padded or the
// dispatch-loop reads would be OOB.
fn sample_code() -> Code {
    let bytecode = vec![0x7f /* PUSH32 */; 8];
    Code::from_bytecode(bytecode.into(), &NativeCrypto)
}

#[test]
fn code_serde_roundtrip_preserves_logical_code_and_repads() {
    let code = sample_code();

    let bytes = serde_json::to_vec(&code).expect("serialize");
    let restored: Code = serde_json::from_slice(&bytes).expect("deserialize");

    assert_eq!(restored.code(), code.code());
    assert_eq!(restored.len(), code.len());
    assert_eq!(restored.hash, code.hash);
    assert_eq!(restored.jumpdests(), code.jumpdests());
    // The dispatch buffer must carry the trailing padding after a round-trip.
    assert_eq!(restored.dispatch_buf().len(), code.len() + BYTECODE_PADDING);
    assert_eq!(restored, code);
}

#[test]
fn code_serde_does_not_persist_padding() {
    // The serialized form must encode the logical code, not the padded buffer:
    // serializing the padding would both waste space and let unpadded input
    // through on deserialize.
    let code = sample_code();
    let restored: Code = serde_json::from_slice(&serde_json::to_vec(&code).unwrap()).unwrap();
    assert_eq!(restored.dispatch_buf().len(), code.dispatch_buf().len());
    assert_eq!(restored.code(), &[0x7f; 8]);
}

#[test]
fn default_code_is_padded() {
    let code = Code::default();
    assert!(code.is_empty());
    assert_eq!(code.dispatch_buf().len(), BYTECODE_PADDING);
}

/// The wire format SHALL carry only the hash and the bytecode. Jump destinations are a
/// pure function of the bytecode. The wire format carries `hash`, `code`, and a
/// `jump_targets` field that is ALWAYS empty: the field exists only to keep the
/// bincode layout byte-identical to the pre-bitmap `Code`, so existing rows in the
/// L2 rollup store (`AccountUpdate`, persisted with bincode) still decode. Because
/// it is always empty regardless of the bytecode's jump destinations, the stored
/// format stays decoupled from how jump destinations are represented — which is the
/// property this test protects.
#[test]
fn code_serde_does_not_persist_derived_jumpdests() {
    let bytecode = vec![0x5b /* JUMPDEST */; 512];
    let code = Code::from_bytecode(bytecode.into(), &NativeCrypto);
    assert!(
        !code.jumpdests().is_empty(),
        "fixture must have a non-empty bitmap for this to prove anything"
    );

    let json = serde_json::to_value(&code).expect("serialize");
    let obj = json.as_object().expect("Code serializes as a struct");
    // `serde_json` orders its map keys, so compare against a sorted expectation.
    let fields: Vec<&str> = obj.keys().map(String::as_str).collect();
    assert_eq!(
        fields,
        vec!["code", "hash", "jump_targets"],
        "unexpected field set in the Code wire format"
    );
    // The retained field must be empty even for jumpdest-dense bytecode — that is
    // what keeps the stored format independent of jumpdest representation.
    assert_eq!(
        obj["jump_targets"].as_array().map(Vec::len),
        Some(0),
        "jump_targets must be persisted empty, never the derived destinations"
    );

    // Recomputed on the way back in, so the round trip is still lossless.
    let restored: Code = serde_json::from_value(json).expect("deserialize");
    assert_eq!(restored.jumpdests(), code.jumpdests());
    assert_eq!(restored, code);
}
