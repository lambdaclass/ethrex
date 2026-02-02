// SPDX-License-Identifier: AGPL-3.0
pragma solidity ^0.8.31;

/// @title Zisk Verifier Interface
/// @author SilentSig
/// @notice This contract is the interface for the Zisk Verifier.
interface IZiskVerifier {
    /// @notice Verifies a proof with given public values and vkey.
    /// @param programVK The verification key for the ZisK program.
    /// @param publicValues The public values encoded as bytes.
    /// @param proofBytes The proof of the program execution the Zisk zkVM encoded as bytes.
    function verifySnarkProof(
        uint64[4] calldata programVK, 
        bytes calldata publicValues,
        bytes calldata proofBytes
    ) external view;
}
