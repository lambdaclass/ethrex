/// @title OpenSponsor
/// @notice A trustless gas sponsor (paymaster) for EIP-8141 frame transactions
///         on the Hegotá devnet. In its pay VERIFY frame it simply calls
///         APPROVE(scope=1 = APPROVE_PAYMENT), becoming the transaction's payer.
///         The frame's resolved target is this contract (P != tx.sender), so the
///         end-of-tx refund and the max-cost debit both land on the sponsor —
///         which is exactly what the EIP-8141 `[only_verify, pay]`
///         (canonical-paymaster) shape is for.
///
/// @dev Design notes (why this and not the older demo contracts):
///   - CanonicalPaymaster.yul ecrecovers the owner's signature over
///     TXPARAM(0x08)=sig_hash, carrying that signature in the VERIFY frame's
///     calldata. On the current spec `compute_sig_hash` commits ALL frame data
///     verbatim, so the owner would have to sign a hash that already contains
///     their signature — an ECDSA fixed point that cannot be satisfied. That
///     contract is unusable on this devnet; do not port it verbatim.
///   - GasSponsor.yul STATICCALLs `token.balanceOf(sender)`. An external call
///     from a NON-canonical paymaster is rejected by the mempool's ERC-7562
///     validation observer, so it can only be included builder-direct.
///   - OpenSponsor makes NO external calls and reads NO storage in the verify
///     path — only APPROVE (0xAA) and (optionally) TXPARAM (0xB0), neither of
///     which the observer bans — so it is admissible via the public mempool.
///     The trade-off is that it sponsors ANY sender (an open faucet-sponsor).
///     A sender-restricted trustless variant would ecrecover an owner signature
///     over a domain that EXCLUDES the signature (e.g. keccak(sender ‖ chain_id
///     ‖ nonce_seq ‖ expiry)); that avoids the sig_hash circularity above while
///     staying observer-friendly. Left out of the minimal demo on purpose.
///
/// Scope bitmask (post-spec-update EIP-8141):
///   0x01 = APPROVE_PAYMENT, 0x02 = APPROVE_EXECUTION, 0x03 = both.
///
/// Storage layout:
///   slot 0: owner (may withdraw the sponsor's ETH)
///
/// Functions:
///   verify()                     0xfc735e99 — APPROVE(scope=1); sponsor pays.
///   owner()                      0x8da5cb5b — read the owner address.
///   withdraw(address,uint256)    0xf3fef3a3 — owner-only ETH withdrawal.
///   receive()                    (no selector) — accept ETH funding.
object "OpenSponsor" {
    code {
        // Constructor: owner address appended as a 32-byte arg after initcode.
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
            // receive() — accept ETH funding.
            if iszero(calldatasize()) { stop() }
            if lt(calldatasize(), 4) { revert(0, 0) }

            let selector := shr(224, calldataload(0))

            switch selector

            // ── verify() ── pay VERIFY frame entrypoint ──────────────
            // APPROVE(scope=1 = APPROVE_PAYMENT). No external calls, no SLOAD,
            // no banned opcodes — safe under the mempool ERC-7562 observer.
            case 0xfc735e99 {
                verbatim_3i_0o(hex"AA", 0, 0, 1)
            }

            // ── owner() ──────────────────────────────────────────────
            case 0x8da5cb5b {
                mstore(0, sload(0))
                return(0, 0x20)
            }

            // ── withdraw(address to, uint256 amount) ── owner only ────
            case 0xf3fef3a3 {
                if iszero(eq(caller(), sload(0))) { revert(0, 0) }
                let to := and(calldataload(4), 0xffffffffffffffffffffffffffffffffffffffff)
                if iszero(to) { revert(0, 0) }
                let amount := calldataload(36)
                let ok := call(gas(), to, amount, 0, 0, 0, 0)
                if iszero(ok) { revert(0, 0) }
                stop()
            }

            default { revert(0, 0) }
        }
    }
}
