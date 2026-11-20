// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

/// @title ZisK Verifier Interface
/// @notice The subset of ZisK's `ZiskVerifier.sol` (zisk-contracts, ZisK
/// v1.3.2-alpha) the OnChainProposer calls. The verifier itself is deployed
/// from the ZisK release the prover is pinned to: its PLONK verification key
/// and recursion root are generated per ZisK version.
interface IZiskVerifier {
    /// @notice Verifies a PLONK-wrapped ZisK proof.
    /// @dev Reverts when the proof is invalid. The verifier hashes
    /// `sha256(programVK ‖ publicValues ‖ rootCVadcopFinal)` into the single
    /// PLONK public input, so both keys must come from the caller's own
    /// storage or from this contract, never from the submitted proof.
    /// @param programVK The ROM root of the guest program, four u64 words packed big-endian.
    /// @param rootCVadcopFinal The recursion root the proof was wrapped under; `getRootCVadcopFinal()` for a plain (non-aggregated) proof.
    /// @param publicValues The 64 u64 public slots the guest committed, each little-endian (512 bytes).
    /// @param proofBytes The PLONK proof, ABI-decoded as `uint256[24]`.
    function verifySnarkProof(
        bytes32 programVK,
        bytes32 rootCVadcopFinal,
        bytes calldata publicValues,
        bytes calldata proofBytes
    ) external view;

    /// @notice The recursion root of this verifier's ZisK version, for plain proofs.
    function getRootCVadcopFinal() external pure returns (bytes32);
}
