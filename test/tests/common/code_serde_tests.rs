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
    assert_eq!(restored.jump_targets, code.jump_targets);
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
