use ethrex_common::utils::keccak;
use ethrex_l2::sequencer::utils::{
    ALIGNED_PROOF_VERIFICATION_FAILED_SELECTOR, CALLER_NOT_ON_CHAIN_PROPOSER_SELECTOR,
    CALLER_NOT_SHARED_BRIDGE_ROUTER_SELECTOR, DEPOSIT_AMOUNT_IS_ZERO_SELECTOR,
    EXCEEDS_PENDING_L2_MESSAGES_LENGTH_SELECTOR, EXCEEDS_PENDING_TX_HASHES_LENGTH_SELECTOR,
    FAILED_TO_SEND_CLAIMED_AMOUNT_SELECTOR, INSUFFICIENT_DEPOSITS_SELECTOR,
    INSUFFICIENT_TOKEN_DEPOSITS_SELECTOR, INVALID_RISC0_PROOF_SELECTOR,
    INVALID_SP1_PROOF_SELECTOR, INVALID_TDX_PROOF_SELECTOR, INVALID_WITHDRAWAL_PROOF_SELECTOR,
    NUMBER_IS_ZERO_SELECTOR, ON_CHAIN_PROPOSER_IS_ZERO_ADDRESS_SELECTOR,
    USE_CLAIM_WITHDRAWAL_FOR_ETH_SELECTOR, WITHDRAWAL_ALREADY_CLAIMED_SELECTOR,
    WITHDRAWAL_BATCH_NOT_COMMITTED_SELECTOR, WITHDRAWAL_BATCH_NOT_VERIFIED_SELECTOR,
    WITHDRAWAL_LOGS_ALREADY_PUBLISHED_SELECTOR,
};

/// Computes the 4-byte ABI error selector for a Solidity custom error signature,
/// e.g. `"InvalidRisc0Proof()"` → `"0x14add973"`.
fn error_selector(signature: &str) -> String {
    let hash = keccak(signature.as_bytes());
    format!("0x{}", hex::encode(&hash[..4]))
}

#[test]
fn on_chain_proposer_error_selectors_match_solidity_signatures() {
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

#[test]
fn common_bridge_error_selectors_match_solidity_signatures() {
    assert_eq!(
        error_selector("OnChainProposerIsZeroAddress()"),
        ON_CHAIN_PROPOSER_IS_ZERO_ADDRESS_SELECTOR
    );
    assert_eq!(
        error_selector("DepositAmountIsZero()"),
        DEPOSIT_AMOUNT_IS_ZERO_SELECTOR
    );
    assert_eq!(
        error_selector("NumberIsZero()"),
        NUMBER_IS_ZERO_SELECTOR
    );
    assert_eq!(
        error_selector("ExceedsPendingTxHashesLength()"),
        EXCEEDS_PENDING_TX_HASHES_LENGTH_SELECTOR
    );
    assert_eq!(
        error_selector("ExceedsPendingL2MessagesLength()"),
        EXCEEDS_PENDING_L2_MESSAGES_LENGTH_SELECTOR
    );
    assert_eq!(
        error_selector("WithdrawalLogsAlreadyPublished()"),
        WITHDRAWAL_LOGS_ALREADY_PUBLISHED_SELECTOR
    );
    assert_eq!(
        error_selector("InsufficientDeposits()"),
        INSUFFICIENT_DEPOSITS_SELECTOR
    );
    assert_eq!(
        error_selector("WithdrawalBatchNotCommitted()"),
        WITHDRAWAL_BATCH_NOT_COMMITTED_SELECTOR
    );
    assert_eq!(
        error_selector("WithdrawalBatchNotVerified()"),
        WITHDRAWAL_BATCH_NOT_VERIFIED_SELECTOR
    );
    assert_eq!(
        error_selector("WithdrawalAlreadyClaimed()"),
        WITHDRAWAL_ALREADY_CLAIMED_SELECTOR
    );
    assert_eq!(
        error_selector("InvalidWithdrawalProof()"),
        INVALID_WITHDRAWAL_PROOF_SELECTOR
    );
    assert_eq!(
        error_selector("FailedToSendClaimedAmount()"),
        FAILED_TO_SEND_CLAIMED_AMOUNT_SELECTOR
    );
    assert_eq!(
        error_selector("UseClaimWithdrawalForETH()"),
        USE_CLAIM_WITHDRAWAL_FOR_ETH_SELECTOR
    );
    assert_eq!(
        error_selector("InsufficientTokenDeposits()"),
        INSUFFICIENT_TOKEN_DEPOSITS_SELECTOR
    );
    assert_eq!(
        error_selector("CallerNotOnChainProposer()"),
        CALLER_NOT_ON_CHAIN_PROPOSER_SELECTOR
    );
    assert_eq!(
        error_selector("CallerNotSharedBridgeRouter()"),
        CALLER_NOT_SHARED_BRIDGE_ROUTER_SELECTOR
    );
}
