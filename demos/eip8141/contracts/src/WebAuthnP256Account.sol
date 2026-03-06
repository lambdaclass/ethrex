// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import "../lib/ECDSA.sol";
import "../lib/WebAuthnP256.sol";
import "../lib/FrameOps.sol";

/// @title WebAuthnP256Account
/// @author Lambda Class
/// @notice Smart account that authenticates frame transactions using WebAuthn
///         passkey signatures over the P256 curve (EIP-7212).
contract WebAuthnP256Account {
    /// @notice The x-coordinate of the owner's P256 public key.
    uint256 public publicKeyX;

    /// @notice The y-coordinate of the owner's P256 public key.
    uint256 public publicKeyY;

    /// @notice Deploys the account with the given P256 public key.
    /// @param x The x-coordinate of the owner's P256 public key
    /// @param y The y-coordinate of the owner's P256 public key
    constructor(uint256 x, uint256 y) {
        publicKeyX = x;
        publicKeyY = y;
    }

    /// @notice Verifies a WebAuthn signature for a frame transaction and approves
    ///         as the sender (scope = 0).
    /// @dev Reads the frame transaction's sig_hash via TXPARAMLOAD(0x08, 0) and
    ///      uses it as the WebAuthn challenge. Calls APPROVE with scope 0 on success.
    /// @param sig The P256 ECDSA signature (r, s)
    /// @param metadata The WebAuthn assertion metadata (authenticatorData, clientDataJSON, etc.)
    function verify(
        ECDSA.Signature calldata sig,
        WebAuthnP256.Metadata calldata metadata
    ) external {
        bytes32 sigHash = bytes32(FrameOps.txParamLoad(0x08, 0));

        ECDSA.PublicKey memory pubKey = ECDSA.PublicKey(publicKeyX, publicKeyY);
        require(
            WebAuthnP256.verify(sigHash, metadata, sig, pubKey),
            "WebAuthnP256Account: invalid signature"
        );

        FrameOps.approve(0, 0, 0);
    }

    /// @notice Verifies a WebAuthn signature and approves as both sender and payer
    ///         (scope = 2).
    /// @dev Same verification logic as `verify`, but calls APPROVE with scope 2
    ///      so this account pays for gas as well.
    /// @param sig The P256 ECDSA signature (r, s)
    /// @param metadata The WebAuthn assertion metadata (authenticatorData, clientDataJSON, etc.)
    function verifyAndPay(
        ECDSA.Signature calldata sig,
        WebAuthnP256.Metadata calldata metadata
    ) external {
        bytes32 sigHash = bytes32(FrameOps.txParamLoad(0x08, 0));

        ECDSA.PublicKey memory pubKey = ECDSA.PublicKey(publicKeyX, publicKeyY);
        require(
            WebAuthnP256.verify(sigHash, metadata, sig, pubKey),
            "WebAuthnP256Account: invalid signature"
        );

        FrameOps.approve(0, 0, 2);
    }

    /// @notice Executes an arbitrary call to a target address.
    /// @dev Only callable within a frame transaction after approval.
    /// @param to The target address to call
    /// @param value The ETH value to send with the call
    /// @param data The calldata to pass to the target
    /// @return result The return data from the call
    function execute(
        address to,
        uint256 value,
        bytes calldata data
    ) external returns (bytes memory result) {
        bool success;
        (success, result) = to.call{value: value}(data);
        require(success, "WebAuthnP256Account: call failed");
    }

    /// @notice Transfers ETH to the specified address.
    /// @param to The recipient address
    /// @param amount The amount of ETH (in wei) to transfer
    function transfer(address to, uint256 amount) external {
        (bool success, ) = to.call{value: amount}("");
        require(success, "WebAuthnP256Account: transfer failed");
    }

    /// @notice Accepts incoming ETH transfers.
    receive() external payable {}
}
