use ethrex_common::utils::keccak;
use ethrex_l2::sequencer::utils::{
    ALIGNED_PROOF_VERIFICATION_FAILED_SELECTOR, INVALID_RISC0_PROOF_SELECTOR,
    INVALID_SP1_PROOF_SELECTOR, INVALID_TDX_PROOF_SELECTOR,
};

/// Computes the 4-byte ABI error selector for a Solidity custom error signature,
/// e.g. `"InvalidRisc0Proof()"` → `"0x14add973"`.
fn error_selector(signature: &str) -> String {
    let hash = keccak(signature.as_bytes());
    format!("0x{}", hex::encode(&hash[..4]))
}

#[test]
fn error_selectors_match_solidity_signatures() {
    assert_eq!(
        error_selector("InvalidRisc0Proof()"),
        INVALID_RISC0_PROOF_SELECTOR
    );
    assert_eq!(
        error_selector("InvalidSp1Proof()"),
        INVALID_SP1_PROOF_SELECTOR
    );
    assert_eq!(
        error_selector("InvalidTdxProof()"),
        INVALID_TDX_PROOF_SELECTOR
    );
    assert_eq!(
        error_selector("AlignedProofVerificationFailed()"),
        ALIGNED_PROOF_VERIFICATION_FAILED_SELECTOR
    );
}
