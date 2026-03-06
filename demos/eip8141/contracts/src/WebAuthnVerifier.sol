// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import "../lib/ECDSA.sol";
import "../lib/WebAuthnP256.sol";

/// @title WebAuthnVerifier
/// @author Lambda Class
/// @notice Helper contract that wraps WebAuthnP256.verify() with an external function.
/// @dev This contract is called from Yul-based account contracts that cannot
///      include the complex WebAuthn verification logic inline. The Yul contracts
///      use verbatim for EIP-8141 frame opcodes, which requires standalone Yul
///      compilation where library imports are unavailable.
contract WebAuthnVerifier {
    /// @notice Verifies a WebAuthn P256 signature on behalf of an account.
    /// @param challenge The sig_hash from the frame transaction
    /// @param pubKeyX The x-coordinate of the signer's P256 public key
    /// @param pubKeyY The y-coordinate of the signer's P256 public key
    /// @param sig The P256 ECDSA signature (r, s)
    /// @param metadata The WebAuthn assertion metadata
    /// @return True if the signature is valid
    function verifyForAccount(
        bytes32 challenge,
        uint256 pubKeyX,
        uint256 pubKeyY,
        ECDSA.Signature calldata sig,
        WebAuthnP256.Metadata calldata metadata
    ) external view returns (bool) {
        ECDSA.PublicKey memory pubKey = ECDSA.PublicKey(pubKeyX, pubKeyY);
        return WebAuthnP256.verify(challenge, metadata, sig, pubKey);
    }
}
