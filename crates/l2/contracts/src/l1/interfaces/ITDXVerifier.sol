// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

/// @title TDX Verifier Interface
/// @author LambdaClass
/// @notice Interface for the TDX Verifier
interface ITDXVerifier {
    /// @notice Verifies a proof with given payload and signature
    /// @dev The signature should correspond to an address previously registered with the verifier
    /// @param payload The payload to be verified
    /// @param signature The associated signature
    function verify(
        bytes calldata payload,
        bytes memory signature
    ) external view;

    /// @notice Registers the quote
    /// @dev The data required to verify the quote must be loaded to the PCCS contracts beforehand
    /// @param quote The TDX quote, which includes the address being registered
    function register(
        bytes calldata quote
    ) external;
}
