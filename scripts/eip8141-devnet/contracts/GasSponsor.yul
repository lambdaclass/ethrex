/// @title GasSponsor
/// @notice Gas sponsor for EIP-8141 frame transactions.
///         Approves as payer (scope=1 = APPROVE_PAYMENT) if the TX sender holds ERC20 tokens.
/// @dev Uses EIP-8141 opcodes via Yul verbatim:
///      - TXPARAM (0xB0): reads transaction parameters
///      - APPROVE (0xAA): approves sender/payer role
///
/// Post-spec-update scope bitmask:
///   0x01 = APPROVE_PAYMENT
///   0x02 = APPROVE_EXECUTION
///   0x03 = PAYMENT + EXECUTION
///
/// Storage layout:
///   slot 0: ERC20 token address for balance checks
///
/// Functions:
///   verify()              0xfc735e99 — Check sender token balance, APPROVE scope=1
///   setConfig(address)    0x20e3dbd4 — Set the ERC20 token address
///   token()               0xfc0c546a — Read the token address
///   receive()             (no selector) — Accept ETH
object "GasSponsor" {
    code {
        datacopy(0, dataoffset("runtime"), datasize("runtime"))
        return(0, datasize("runtime"))
    }
    object "runtime" {
        code {
            if iszero(calldatasize()) { stop() }
            if lt(calldatasize(), 4) { revert(0, 0) }

            let selector := shr(224, calldataload(0))

            switch selector

            // ── verify() ──────────────────────────────────────────────
            case 0xfc735e99 {
                // TXPARAM(0x02) → sender address  (post-spec-update: TXPARAM takes 1 arg)
                let sender := verbatim_1i_1o(hex"B0", 0x02)
                sender := and(sender, 0xffffffffffffffffffffffffffffffffffffffff)

                let tokenAddr := sload(0)

                // STATICCALL token.balanceOf(sender)
                mstore(0x00, shl(224, 0x70a08231))
                mstore(0x04, sender)

                let ok := staticcall(gas(), tokenAddr, 0x00, 0x24, 0x00, 0x20)
                if iszero(ok) {
                    mstore(0x00, shl(224, 0x08c379a0))
                    mstore(0x04, 0x20)
                    mstore(0x24, 22)
                    mstore(0x44, "balanceOf call failed")
                    revert(0x00, 0x64)
                }

                let bal := mload(0x00)
                if iszero(bal) {
                    mstore(0x00, shl(224, 0x08c379a0))
                    mstore(0x04, 0x20)
                    mstore(0x24, 24)
                    mstore(0x44, "sender has no tokens")
                    revert(0x00, 0x64)
                }

                // APPROVE scope=1 (APPROVE_PAYMENT) under the post-update EIP-8141
                // scope bitmask (0x01=PAYMENT, 0x02=EXECUTION, 0x03=both).
                let payerScope := 1
                verbatim_3i_0o(hex"AA", 0, 0, payerScope)
            }

            // ── setConfig(address _token) ─────────────────────────────
            case 0x20e3dbd4 {
                sstore(0, and(calldataload(4), 0xffffffffffffffffffffffffffffffffffffffff))
                stop()
            }

            // ── token() ───────────────────────────────────────────────
            case 0xfc0c546a {
                mstore(0, sload(0))
                return(0, 0x20)
            }

            default { revert(0, 0) }
        }
    }
}
