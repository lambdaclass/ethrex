/// @title WebAuthnP256Account
/// @notice Smart account for EIP-8141 frame transactions with WebAuthn passkey auth.
/// @dev Storage layout:
///   slot 0: P256 public key X coordinate
///   slot 1: P256 public key Y coordinate
///
/// Constructor args (appended after initcode):
///   pubKeyX (uint256) — P256 public key X coordinate
///   pubKeyY (uint256) — P256 public key Y coordinate
///
/// Functions:
///   verify(sig, metadata)        0x182ffd20 — Verify WebAuthn sig, APPROVE scope=0
///   verifyAndPay(sig, metadata)  0x5a27d2e0 — Verify WebAuthn sig, APPROVE scope=2
///   transfer(address,uint256)    0xa9059cbb — Transfer ETH
///   execute(address,uint256,bytes) 0xb61d27f6 — Arbitrary call
///   publicKeyX()                 0xfa6df55d — Read pubkey X
///   publicKeyY()                 0xd7a6f6e8 — Read pubkey Y
///   receive()                    (no selector) — Accept ETH
///
/// External dependency:
///   WebAuthnVerifier at 0x1000000000000000000000000000000000000004
///   verifyForAccount(bytes32,uint256,uint256,(uint256,uint256),(bytes,string,uint16,uint16,bool))
///   selector: 0x3d5e14a0
object "WebAuthnP256Account" {
    code {
        // Constructor: read pubKeyX and pubKeyY from the end of the initcode.
        // During CREATE2, codesize() = initcode + appended args (64 bytes).
        let argsOffset := sub(codesize(), 64)
        codecopy(0, argsOffset, 64)
        sstore(0, mload(0))    // slot 0 = pubKeyX
        sstore(1, mload(0x20)) // slot 1 = pubKeyY

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
            // Verify WebAuthn signature, APPROVE as sender (scope=0)
            case 0x182ffd20 {
                verifyAndApprove(0)
                stop() // unreachable — APPROVE halts
            }

            // ── verifyAndPay((uint256,uint256),(bytes,string,uint16,uint16,bool)) ──
            // Verify WebAuthn signature, APPROVE as sender+payer (scope=2)
            case 0x5a27d2e0 {
                verifyAndApprove(2)
                stop() // unreachable — APPROVE halts
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
        }
    }
}
