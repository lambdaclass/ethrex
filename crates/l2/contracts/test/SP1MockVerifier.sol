// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "../src/l1/interfaces/ISP1Verifier.sol";

/// @title SP1 Mock Verifier
/// @notice Mock verifier for testing. Accepts any proof with empty bytes
///         (matching SP1's mock proof format where encoded_proof is empty).
///         For non-empty proofs, it accepts any proof unconditionally —
///         useful for integration testing the verification flow without
///         requiring real Groth16 proofs.
contract SP1MockVerifier is ISP1Verifier {
    /// @notice Always succeeds — accepts any proof for testing.
    function verifyProof(
        bytes32 /* programVKey */,
        bytes calldata /* publicValues */,
        bytes calldata /* proofBytes */
    ) external pure override {
        // Mock: accept all proofs
    }
}
