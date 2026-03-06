/// @title GasSponsor
/// @notice Gas sponsor for EIP-8141 frame transactions.
///         Approves as payer (scope=1) if the TX sender holds ERC20 tokens.
/// @dev Storage layout:
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
            // receive() — accept ETH when no calldata
            if iszero(calldatasize()) {
                stop()
            }

            // Need at least 4 bytes for a function selector
            if lt(calldatasize(), 4) {
                revert(0, 0)
            }

            let selector := shr(224, calldataload(0))

            switch selector

            // ── verify() ──────────────────────────────────────────────
            // Reads the frame TX sender via TXPARAMLOAD(0x02, 0),
            // checks their ERC20 balance, and APPROVE(0,0,1) as payer.
            case 0xfc735e99 {
                // TXPARAMLOAD(param_id=0x02, index=0) → sender address
                let sender := verbatim_2i_1o(hex"B0", 0x02, 0)
                sender := and(sender, 0xffffffffffffffffffffffffffffffffffffffff)

                // Read token address from storage slot 0
                let tokenAddr := sload(0)

                // STATICCALL token.balanceOf(sender)
                // balanceOf(address) selector = 0x70a08231
                mstore(0x00, shl(224, 0x70a08231))
                mstore(0x04, sender)

                let ok := staticcall(gas(), tokenAddr, 0x00, 0x24, 0x00, 0x20)
                if iszero(ok) {
                    mstore(0x00, shl(224, 0x08c379a0)) // Error(string)
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

                // APPROVE scope=1 (payer approval)
                // Stack: offset=0, length=0, scope=1
                verbatim_3i_0o(hex"AA", 0, 0, 1)
            }

            // ── setConfig(address _token) ─────────────────────────────
            case 0x20e3dbd4 {
                let tokenAddr := and(
                    calldataload(4),
                    0xffffffffffffffffffffffffffffffffffffffff
                )
                sstore(0, tokenAddr)
                stop()
            }

            // ── token() ───────────────────────────────────────────────
            case 0xfc0c546a {
                mstore(0, sload(0))
                return(0, 0x20)
            }

            // Unknown selector
            default {
                revert(0, 0)
            }
        }
    }
}
