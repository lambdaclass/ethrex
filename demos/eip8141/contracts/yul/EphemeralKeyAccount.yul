/// Reference implementation. The production demo uses UnifiedAccount.yul
/// which combines P256 and ECDSA auth in a single contract.
///
/// @title EphemeralKeyAccount
/// @notice Smart account for EIP-8141 frame transactions with ephemeral ECDSA key auth.
/// @dev Storage layout:
///   slot 0: SignerRegistry address
///
/// Constructor args (appended after initcode):
///   registryAddress (uint256) — SignerRegistry contract address
///
/// Functions:
///   verify(uint8,bytes32,bytes32)          0xe2454522 — Verify ECDSA sig, APPROVE scope=1
///   verifyAndPay(uint8,bytes32,bytes32)    0x450beed2 — Verify ECDSA sig, APPROVE scope=3
///   transfer(address,uint256)              0xa9059cbb — Transfer ETH
///   execute(address,uint256,bytes)         0xb61d27f6 — Arbitrary call
///   registry()                             0x7b103999 — Read registry address
///   receive()                              (no selector) — Accept ETH
///
/// External dependency:
///   SignerRegistry at the address stored in slot 0
///   resolve(address) selector: 0x55ea6c47
object "EphemeralKeyAccount" {
    code {
        // Constructor: read registryAddress from end of initcode (32 bytes)
        let argsOffset := sub(codesize(), 32)
        codecopy(0, argsOffset, 32)
        sstore(0, mload(0))    // slot 0 = registry address

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

            // ── verify(uint8 v, bytes32 r, bytes32 s) ────────────────
            // Verify ECDSA signature against registry signer, APPROVE scope=1
            case 0xe2454522 {
                ecrecoverAndApprove(1)
                stop()
            }

            // ── verifyAndPay(uint8 v, bytes32 r, bytes32 s) ──────────
            // Verify ECDSA signature against registry signer, APPROVE scope=3
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
                let dataOffset := calldataload(0x44)
                let dataLen := calldataload(add(0x04, dataOffset))
                let dataStart := add(add(0x04, dataOffset), 0x20)

                calldatacopy(0, dataStart, dataLen)

                let ok := call(gas(), to, value, 0, dataLen, 0, 0)
                if iszero(ok) {
                    returndatacopy(0, 0, returndatasize())
                    revert(0, returndatasize())
                }
                returndatacopy(0, 0, returndatasize())
                return(0, returndatasize())
            }

            // ── registry() ────────────────────────────────────────────
            case 0x7b103999 {
                mstore(0, sload(0))
                return(0, 0x20)
            }

            default {
                revert(0, 0)
            }

            // ── Internal: verify ECDSA signature and APPROVE ──────────
            //
            // Reads sig_hash from TXPARAMLOAD(0x08, 0), calls the
            // SignerRegistry to resolve the expected signer, ecrecovers
            // the signature, and APPROVEs if they match.
            //
            // Calldata layout:
            //   [0x00..0x04) selector
            //   [0x04..0x24) v (uint8, right-aligned in 32 bytes)
            //   [0x24..0x44) r (bytes32)
            //   [0x44..0x64) s (bytes32)
            //
            function ecrecoverAndApprove(scope) {
                // Read sig_hash via TXPARAMLOAD(param_id=0x08, index=0)
                let sigHash := verbatim_1i_1o(hex"B0", 0x08)

                // Read v, r, s from calldata
                let v := calldataload(4)
                let r := calldataload(0x24)
                let s := calldataload(0x44)

                // Call registry.resolve(address(this)) to get expected signer
                let registry := sload(0)
                mstore(0x00, shl(224, 0x55ea6c47))  // resolve(address) selector
                mstore(0x04, address())

                let ok := staticcall(gas(), registry, 0x00, 0x24, 0x00, 0x20)
                if iszero(ok) {
                    mstore(0x00, shl(224, 0x08c379a0))
                    mstore(0x04, 0x20)
                    mstore(0x24, 14)
                    mstore(0x44, "resolve failed")
                    revert(0x00, 0x64)
                }

                let expectedSigner := and(mload(0x00), 0xffffffffffffffffffffffffffffffffffffffff)

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

                ok := staticcall(3000, 1, 0x00, 0x80, 0x00, 0x20)
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
