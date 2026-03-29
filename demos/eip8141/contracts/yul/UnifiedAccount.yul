/// @title UnifiedAccount
/// @notice Smart account for EIP-8141 frame transactions with dual auth:
///         WebAuthn P256 passkey verification AND ephemeral ECDSA key verification.
/// @dev Storage layout:
///   slot 0: P256 public key X coordinate
///   slot 1: P256 public key Y coordinate
///   slot 2: currentSigner (address for ephemeral ECDSA keys, 0 = not set)
///
/// Constructor args (appended after initcode):
///   pubKeyX (uint256) — P256 public key X coordinate
///   pubKeyY (uint256) — P256 public key Y coordinate
///
/// Functions:
///   verify(sig, metadata)                  0x182ffd20 — WebAuthn P256, APPROVE scope=1
///   verifyAndPay(sig, metadata)            0x5a27d2e0 — WebAuthn P256, APPROVE scope=3
///   verifyEcdsa(uint8,bytes32,bytes32)     0xe2454522 — ECDSA ephemeral, APPROVE scope=1
///   verifyEcdsaAndPay(uint8,bytes32,bytes32) 0x450beed2 — ECDSA ephemeral, APPROVE scope=3
///   transfer(address,uint256)              0xa9059cbb — Transfer ETH
///   execute(address,uint256,bytes)         0xb61d27f6 — Arbitrary call
///   publicKeyX()                           0xfa6df55d — Read pubkey X
///   publicKeyY()                           0xd7a6f6e8 — Read pubkey Y
///   currentSigner()                        0x3a5c8c89 — Read ephemeral signer
///   rotate(address)                        0x7c281a19 — Set new ephemeral signer (self-call only)
///   receive()                              (no selector) — Accept ETH
///
/// External dependency:
///   WebAuthnVerifier at 0x1000000000000000000000000000000000000004
///   verifyForAccount(bytes32,uint256,uint256,(uint256,uint256),(bytes,string,uint16,uint16,bool))
///   selector: 0x3d5e14a0
object "UnifiedAccount" {
    code {
        // Constructor: read pubKeyX and pubKeyY from the end of the initcode.
        // During CREATE2, codesize() = initcode + appended args (64 bytes).
        let argsOffset := sub(codesize(), 64)
        codecopy(0, argsOffset, 64)
        sstore(0, mload(0))    // slot 0 = pubKeyX
        sstore(1, mload(0x20)) // slot 1 = pubKeyY
        // slot 2 = currentSigner left as 0 (not set)

        // Deploy runtime
        datacopy(0, dataoffset("runtime"), datasize("runtime"))
        return(0, datasize("runtime"))
    }
    object "runtime" {
        code {
            // receive() — accept ETH
            if iszero(calldatasize()) {
                stop()
            }

            if lt(calldatasize(), 4) {
                revert(0, 0)
            }

            let selector := shr(224, calldataload(0))

            switch selector

            // ── verify((uint256,uint256),(bytes,string,uint16,uint16,bool)) ──
            // Verify WebAuthn signature, APPROVE as sender (scope=1)
            case 0x182ffd20 {
                verifyAndApprove(1)
                stop() // unreachable — APPROVE halts
            }

            // ── verifyAndPay((uint256,uint256),(bytes,string,uint16,uint16,bool)) ──
            // Verify WebAuthn signature, APPROVE as sender+payer (scope=3)
            case 0x5a27d2e0 {
                verifyAndApprove(3)
                stop() // unreachable — APPROVE halts
            }

            // ── verifyEcdsa(uint8 v, bytes32 r, bytes32 s) ────────────
            // Verify ECDSA signature against currentSigner, APPROVE scope=1
            case 0xe2454522 {
                ecrecoverAndApprove(1)
                stop()
            }

            // ── verifyEcdsaAndPay(uint8 v, bytes32 r, bytes32 s) ──────
            // Verify ECDSA signature against currentSigner, APPROVE scope=3
            case 0x450beed2 {
                ecrecoverAndApprove(3)
                stop()
            }

            // ── transfer(address,uint256) ─────────────────────────────
            case 0xa9059cbb {
                let to := and(calldataload(4), 0xffffffffffffffffffffffffffffffffffffffff)
                let amount := calldataload(0x24)
                let ok := call(gas(), to, amount, 0, 0, 0, 0)
                if iszero(ok) {
                    revert(0, 0)
                }
                mstore(0, 1)
                return(0, 0x20)
            }

            // ── execute(address,uint256,bytes) ────────────────────────
            case 0xb61d27f6 {
                let to := and(calldataload(4), 0xffffffffffffffffffffffffffffffffffffffff)
                let value := calldataload(0x24)
                // bytes data offset (relative to params start at byte 4)
                let dataOffset := calldataload(0x44)
                let dataLen := calldataload(add(0x04, dataOffset))
                let dataStart := add(add(0x04, dataOffset), 0x20)

                // Copy call data to memory
                calldatacopy(0, dataStart, dataLen)

                let ok := call(gas(), to, value, 0, dataLen, 0, 0)
                if iszero(ok) {
                    returndatacopy(0, 0, returndatasize())
                    revert(0, returndatasize())
                }
                returndatacopy(0, 0, returndatasize())
                return(0, returndatasize())
            }

            // ── publicKeyX() ──────────────────────────────────────────
            case 0xfa6df55d {
                mstore(0, sload(0))
                return(0, 0x20)
            }

            // ── publicKeyY() ──────────────────────────────────────────
            case 0xd7a6f6e8 {
                mstore(0, sload(1))
                return(0, 0x20)
            }

            // ── currentSigner() ───────────────────────────────────────
            case 0x3a5c8c89 {
                mstore(0, sload(2))
                return(0, 0x20)
            }

            // ── rotate(address newSigner) ─────────────────────────────
            // Callable by the account itself (SENDER frame) or by anyone
            // if no signer is set yet (initial setup during registration).
            case 0x7c281a19 {
                let current := sload(2)
                // Allow external calls only when currentSigner is unset (0)
                if and(iszero(iszero(current)), iszero(eq(caller(), address()))) {
                    mstore(0x00, shl(224, 0x08c379a0))
                    mstore(0x04, 0x20)
                    mstore(0x24, 15)
                    mstore(0x44, "only self-call")
                    revert(0x00, 0x64)
                }
                let newSigner := and(calldataload(4), 0xffffffffffffffffffffffffffffffffffffffff)
                sstore(2, newSigner)
                stop()
            }

            default {
                revert(0, 0)
            }

            // ── Internal: verify WebAuthn signature and APPROVE ───────
            //
            // Reads sig_hash from TXPARAMLOAD(0x08, 0), reads pubkey from
            // storage, forwards verification to the WebAuthnVerifier helper
            // contract, then calls APPROVE with the given scope.
            //
            // The incoming calldata (from verify/verifyAndPay) is:
            //   [0x00..0x04) selector
            //   [0x04..0x24) sig.r
            //   [0x24..0x44) sig.s
            //   [0x44..0x64) metadata offset (relative to byte 0x04)
            //   [0x64..)     metadata data
            //
            // We build new calldata for verifyForAccount():
            //   [0x00..0x04) 0x3d5e14a0 (verifyForAccount selector)
            //   [0x04..0x24) challenge (sig_hash)
            //   [0x24..0x44) pubKeyX
            //   [0x44..0x64) pubKeyY
            //   [0x64..0x84) sig.r     (from original calldata)
            //   [0x84..0xa4) sig.s     (from original calldata)
            //   [0xa4..0xc4) metadata offset + 0x60 (adjusted for 3 new words)
            //   [0xc4..)     metadata data (from original calldata)
            //
            function verifyAndApprove(scope) {
                // Read sig_hash via TXPARAMLOAD(param_id=0x08, index=0)
                let sigHash := verbatim_2i_1o(hex"B0", 0x08, 0)

                // Write verifyForAccount selector
                mstore(0x00, shl(224, 0x3d5e14a0))

                // Write challenge (sig_hash)
                mstore(0x04, sigHash)

                // Write public key from storage
                mstore(0x24, sload(0)) // pubKeyX
                mstore(0x44, sload(1)) // pubKeyY

                // Copy sig.r and sig.s from original calldata[4..68)
                calldatacopy(0x64, 0x04, 0x40)

                // Read original metadata offset and adjust by +96 (0x60)
                // for the 3 new head words (challenge, pubKeyX, pubKeyY)
                let origMetaOffset := calldataload(0x44)
                mstore(0xa4, add(origMetaOffset, 0x60))

                // Copy metadata data: calldata[100..end) → memory[0xc4..)
                let restLen := sub(calldatasize(), 0x64)
                calldatacopy(0xc4, 0x64, restLen)

                // Total call size = calldatasize + 96
                let totalSize := add(calldatasize(), 0x60)

                // STATICCALL WebAuthnVerifier.verifyForAccount(...)
                let verifier := 0x1000000000000000000000000000000000000004
                let ok := staticcall(gas(), verifier, 0x00, totalSize, 0x00, 0x20)

                if iszero(ok) {
                    // Forward revert data from verifier
                    returndatacopy(0, 0, returndatasize())
                    revert(0, returndatasize())
                }

                // Check return value (bool encoded as uint256)
                let result := mload(0x00)
                if iszero(result) {
                    // Signature verification failed
                    mstore(0x00, shl(224, 0x08c379a0))
                    mstore(0x04, 0x20)
                    mstore(0x24, 17)
                    mstore(0x44, "invalid signature")
                    revert(0x00, 0x64)
                }

                // APPROVE with the given scope
                // Stack order for APPROVE: offset (top), length, scope (bottom)
                verbatim_3i_0o(hex"AA", 0, 0, scope)
            }

            // ── Internal: verify ECDSA signature and APPROVE ──────────
            //
            // Reads sig_hash from TXPARAMLOAD(0x08, 0), loads the expected
            // signer from slot 2 (currentSigner), ecrecovers the signature,
            // and APPROVEs if they match.
            //
            // Calldata layout:
            //   [0x00..0x04) selector
            //   [0x04..0x24) v (uint8, right-aligned in 32 bytes)
            //   [0x24..0x44) r (bytes32)
            //   [0x44..0x64) s (bytes32)
            //
            function ecrecoverAndApprove(scope) {
                // Read sig_hash via TXPARAMLOAD(param_id=0x08, index=0)
                let sigHash := verbatim_2i_1o(hex"B0", 0x08, 0)

                // Read v, r, s from calldata
                let v := calldataload(4)
                let r := calldataload(0x24)
                let s := calldataload(0x44)

                // Load currentSigner from slot 2
                let expectedSigner := and(sload(2), 0xffffffffffffffffffffffffffffffffffffffff)

                if iszero(expectedSigner) {
                    mstore(0x00, shl(224, 0x08c379a0))
                    mstore(0x04, 0x20)
                    mstore(0x24, 13)
                    mstore(0x44, "no signer set")
                    revert(0x00, 0x64)
                }

                // ecrecover: precompile at address 0x01
                // Input: hash (32) + v (32) + r (32) + s (32)
                mstore(0x00, sigHash)
                mstore(0x20, v)
                mstore(0x40, r)
                mstore(0x60, s)

                let ok := staticcall(3000, 1, 0x00, 0x80, 0x00, 0x20)
                if iszero(ok) {
                    mstore(0x00, shl(224, 0x08c379a0))
                    mstore(0x04, 0x20)
                    mstore(0x24, 16)
                    mstore(0x44, "ecrecover failed")
                    revert(0x00, 0x64)
                }

                let recovered := and(mload(0x00), 0xffffffffffffffffffffffffffffffffffffffff)

                if iszero(eq(recovered, expectedSigner)) {
                    mstore(0x00, shl(224, 0x08c379a0))
                    mstore(0x04, 0x20)
                    mstore(0x24, 17)
                    mstore(0x44, "invalid signature")
                    revert(0x00, 0x64)
                }

                // APPROVE with the given scope
                verbatim_3i_0o(hex"AA", 0, 0, scope)
            }
        }
    }
}
