/// @title CanonicalPaymaster (Yul port)
/// @notice Minimal canonical paymaster for EIP-8141 frame transactions.
/// @dev Ported from Solidity because solc doesn't support verbatim in inline assembly.
///
/// Auth model: The paymaster owner's secp256k1 signature over the frame tx sig_hash
/// must be provided as VERIFY frame calldata: r(32) || s(32) || v(1) = 65 bytes.
///
/// Scope: Uses APPROVE(scope=1 = APPROVE_PAYMENT) under the post-update EIP-8141 bitmask.
///
/// Withdrawal protection: 12-hour timelock prevents instant balance draining.
///
/// Storage: slot 0=owner, slot 1=pendingTo, slot 2=pendingAmount, slot 3=pendingReadyAt
///
/// Constructor: owner address appended as 32-byte arg after initcode
object "CanonicalPaymaster" {
    code {
        let argOffset := sub(codesize(), 32)
        codecopy(0, argOffset, 32)
        let owner := and(mload(0), 0xffffffffffffffffffffffffffffffffffffffff)
        if iszero(owner) { revert(0, 0) }
        sstore(0, owner)

        datacopy(0, dataoffset("runtime"), datasize("runtime"))
        return(0, datasize("runtime"))
    }
    object "runtime" {
        code {
            // receive() — accept ETH
            if iszero(calldatasize()) { stop() }

            // ── Verify path: exactly 65 bytes = r(32) + s(32) + v(1) ──
            // Must check BEFORE selector parsing: signature bytes would
            // be misinterpreted as a function selector.
            if eq(calldatasize(), 65) {
                let r := calldataload(0)
                let s := calldataload(32)
                let v := byte(0, calldataload(64))

                // EIP-2 s-value check
                let sHalf := 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0
                if gt(s, sHalf) { revert(0, 0) }
                if iszero(or(eq(v, 27), eq(v, 28))) { revert(0, 0) }

                // TXPARAM(0x08) → sig_hash  (post-spec-update: TXPARAM takes 1 arg)
                let sigHash := verbatim_1i_1o(hex"B0", 0x08)

                // ecrecover(hash, v, r, s)
                mstore(0x00, sigHash)
                mstore(0x20, v)
                mstore(0x40, r)
                mstore(0x60, s)

                let success := staticcall(gas(), 0x01, 0x00, 0x80, 0x00, 0x20)
                if iszero(success) { revert(0, 0) }

                let recovered := and(mload(0x00), 0xffffffffffffffffffffffffffffffffffffffff)
                if iszero(eq(recovered, sload(0))) { revert(0, 0) }

                // APPROVE scope=1 (APPROVE_PAYMENT under post-update spec bitmask)
                let payerScope := 1
                verbatim_3i_0o(hex"AA", 0, 0, payerScope)
                stop()
            }

            // ── Function selector routing ──
            if lt(calldatasize(), 4) { revert(0, 0) }
            let selector := shr(224, calldataload(0))

            switch selector

            // owner() 0x8da5cb5b
            case 0x8da5cb5b {
                mstore(0, sload(0))
                return(0, 0x20)
            }

            // requestWithdrawal(address,uint256) 0xdbaf2145
            case 0xdbaf2145 {
                if iszero(eq(caller(), sload(0))) { revert(0, 0) }
                let to := and(calldataload(4), 0xffffffffffffffffffffffffffffffffffffffff)
                if iszero(to) { revert(0, 0) }
                sstore(1, to)
                sstore(2, calldataload(36))
                sstore(3, add(timestamp(), 43200))
                stop()
            }

            // executeWithdrawal() 0x9e6371ba
            case 0x9e6371ba {
                if iszero(eq(caller(), sload(0))) { revert(0, 0) }
                let to := sload(1)
                let amount := sload(2)
                let readyAt := sload(3)
                if iszero(readyAt) { revert(0, 0) }
                if lt(timestamp(), readyAt) { revert(0, 0) }
                sstore(1, 0)
                sstore(2, 0)
                sstore(3, 0)
                let ok := call(gas(), to, amount, 0, 0, 0, 0)
                if iszero(ok) { revert(0, 0) }
                stop()
            }

            default { revert(0, 0) }
        }
    }
}
