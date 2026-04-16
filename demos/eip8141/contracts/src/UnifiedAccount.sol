// SPDX-License-Identifier: MIT
pragma solidity ^0.8.25;

/// @title UnifiedAccount
/// @notice Smart account for EIP-8141 frame transactions with dual authentication:
///         WebAuthn P256 passkey verification AND ephemeral ECDSA key rotation.
/// @dev Uses EIP-8141 custom opcodes:
///   - TXPARAM (0xB0): reads transaction parameters (sig_hash, etc.)
///   - APPROVE (0xAA): approves sender/payer for the frame transaction
///
/// Storage layout:
///   slot 0: P256 public key X coordinate
///   slot 1: P256 public key Y coordinate
///   slot 2: currentSigner (ephemeral ECDSA signer address, 0 = not set)

interface IWebAuthnVerifier {
    struct Signature {
        uint256 r;
        uint256 s;
    }
    struct AuthenticatorMetadata {
        bytes authenticatorData;
        string clientDataJSON;
        uint16 challengeIndex;
        uint16 typeIndex;
        bool userVerificationRequired;
    }
    function verifyForAccount(
        bytes32 challenge,
        uint256 pubKeyX,
        uint256 pubKeyY,
        Signature calldata sig,
        AuthenticatorMetadata calldata metadata
    ) external view returns (bool);
}

contract UnifiedAccount {
    uint256 public publicKeyX;
    uint256 public publicKeyY;
    address public currentSigner;

    address constant WEBAUTHN_VERIFIER = 0x1000000000000000000000000000000000000004;

    constructor(uint256 _pubKeyX, uint256 _pubKeyY) {
        publicKeyX = _pubKeyX;
        publicKeyY = _pubKeyY;
    }

    receive() external payable {}

    // ═══════════════════════════════════════════════════════════════
    // WebAuthn P256 verification
    // ═══════════════════════════════════════════════════════════════

    /// @notice Verify WebAuthn P256 signature, APPROVE as sender (scope=1)
    function verify(
        IWebAuthnVerifier.Signature calldata sig,
        IWebAuthnVerifier.AuthenticatorMetadata calldata metadata
    ) external {
        _verifyWebAuthn(sig, metadata);
        _approve(1);
    }

    /// @notice Verify WebAuthn P256 signature, APPROVE as sender+payer (scope=3)
    function verifyAndPay(
        IWebAuthnVerifier.Signature calldata sig,
        IWebAuthnVerifier.AuthenticatorMetadata calldata metadata
    ) external {
        _verifyWebAuthn(sig, metadata);
        _approve(3);
    }

    // ═══════════════════════════════════════════════════════════════
    // Ephemeral ECDSA key verification
    // ═══════════════════════════════════════════════════════════════

    /// @notice Verify ECDSA signature against currentSigner, APPROVE as sender (scope=1)
    function verifyEcdsa(uint8 v, bytes32 r, bytes32 s) external {
        _verifyEcdsa(v, r, s);
        _approve(1);
    }

    /// @notice Verify ECDSA signature against currentSigner, APPROVE as sender+payer (scope=3)
    function verifyEcdsaAndPay(uint8 v, bytes32 r, bytes32 s) external {
        _verifyEcdsa(v, r, s);
        _approve(3);
    }

    // ═══════════════════════════════════════════════════════════════
    // Key rotation
    // ═══════════════════════════════════════════════════════════════

    /// @notice Set the ephemeral ECDSA signer address.
    ///         Callable by anyone when no signer is set (initial registration),
    ///         or by the account itself (SENDER frame) after that.
    function rotate(address newSigner) external {
        if (currentSigner != address(0)) {
            require(msg.sender == address(this), "only self-call");
        }
        currentSigner = newSigner;
    }

    // ═══════════════════════════════════════════════════════════════
    // Execution
    // ═══════════════════════════════════════════════════════════════

    /// @notice Transfer ETH to an address
    function transfer(address to, uint256 amount) external returns (bool) {
        (bool ok,) = to.call{value: amount}("");
        require(ok, "transfer failed");
        return true;
    }

    /// @notice Execute an arbitrary call
    function execute(address to, uint256 value, bytes calldata data) external returns (bytes memory) {
        (bool ok, bytes memory result) = to.call{value: value}(data);
        if (!ok) {
            assembly {
                revert(add(result, 32), mload(result))
            }
        }
        return result;
    }

    // ═══════════════════════════════════════════════════════════════
    // Internal: EIP-8141 custom opcodes
    // ═══════════════════════════════════════════════════════════════

    /// @dev Read sig_hash from TXPARAM opcode (0xB0), param_id=0x08, index=0
    function _txparamSigHash() internal view returns (bytes32 result) {
        assembly {
            result := verbatim_2i_1o(hex"B0", 0x08, 0)
        }
    }

    /// @dev Call APPROVE opcode (0xAA) with offset=0, length=0, and given scope.
    ///      This halts the current frame — code after this call is unreachable.
    function _approve(uint256 scope) internal view {
        assembly {
            verbatim_3i_0o(hex"AA", 0, 0, scope)
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // Internal: verification logic
    // ═══════════════════════════════════════════════════════════════

    function _verifyWebAuthn(
        IWebAuthnVerifier.Signature calldata sig,
        IWebAuthnVerifier.AuthenticatorMetadata calldata metadata
    ) internal view {
        bytes32 sigHash = _txparamSigHash();
        bool valid = IWebAuthnVerifier(WEBAUTHN_VERIFIER).verifyForAccount(
            sigHash, publicKeyX, publicKeyY, sig, metadata
        );
        require(valid, "invalid signature");
    }

    function _verifyEcdsa(uint8 v, bytes32 r, bytes32 s) internal view {
        bytes32 sigHash = _txparamSigHash();
        address signer = currentSigner;
        require(signer != address(0), "no signer set");
        address recovered = ecrecover(sigHash, v, r, s);
        require(recovered == signer, "invalid signature");
    }
}
