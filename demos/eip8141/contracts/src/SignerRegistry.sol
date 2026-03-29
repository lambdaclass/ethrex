// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title SignerRegistry
/// @notice Singleton registry mapping account addresses to their current ECDSA signer.
/// @dev Used by EphemeralKeyAccount contracts to look up the authorized signer.
///      Only the account itself can rotate its signer (msg.sender = account).
contract SignerRegistry {
    mapping(address account => address signer) public signerOf;

    event SignerRotated(address indexed account, address indexed oldSigner, address indexed newSigner);

    /// @notice Set or rotate the signer for msg.sender.
    /// @param nextSigner The new authorized signer address.
    function rotate(address nextSigner) external {
        address old = signerOf[msg.sender];
        signerOf[msg.sender] = nextSigner;
        emit SignerRotated(msg.sender, old, nextSigner);
    }

    /// @notice Resolve the current signer for an account.
    /// @param account The account to query.
    /// @return The current signer address (address(0) if none set).
    function resolve(address account) external view returns (address) {
        return signerOf[account];
    }
}
