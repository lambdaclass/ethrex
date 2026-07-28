/// @title GuardMandatingAccount
/// @notice An EIP-8141 account that authorizes a transaction only if that transaction
///         carries a POST_TX assertion frame matching this account's stored policy.
///         A hostile transaction composer therefore cannot strip, substitute, or
///         weaken the guard: doing so makes the VERIFY frame revert, which invalidates
///         the whole transaction before its body runs.
///
/// @dev WHY THIS EXISTS. Every published EIP-7906 proof-of-concept assumes the POST_TX
///      guard is present on the victim's transaction. But under EIP-8141 whoever
///      composes the transaction composes the frame list — so in exactly the threat
///      models that matter (a phishing frontend, compromised signing infrastructure)
///      the adversary simply omits the guard and the honest wallet signs a guardless
///      transaction. Enforcing guard presence at the ACCOUNT closes that hole, using
///      only primitives EIP-8141 already ships. No spec change is required.
///
/// @dev WHY THIS IS YUL AND MUST STAY YUL. Do not "port this to Solidity for
///      readability" — it would silently break, for two independent reasons:
///        1. APPROVE (0xAA) cannot be delegated. Its semantics bind to the executing
///           frame, and a VERIFY frame runs in static mode, so an APPROVE inside a
///           callee would fail. This code must emit 0xAA itself, and Solidity cannot
///           emit it (`verbatim_*` is unavailable in Solidity inline assembly).
///        2. The VERIFY frame must make NO external calls to stay admissible through
///           the public mempool under the ERC-7562 validation observer. The shim
///           indirection used by the Solidity guards is therefore unavailable here.
///      Consequently this contract reads only its OWN storage and calls nothing.
///
/// Opcodes used (hegota-devnet assignment). Argument order below matches each opcode's
/// pop order, where the first `verbatim` argument is the operand popped first:
///   APPROVE        0xAA  [offset, length, scope]
///   TXPARAM        0xB0  [param_id]
///   FRAMEDATACOPY  0xB2  [memOffset, dataOffset, length, frameIndex]
///   FRAMEPARAM     0xB3  [frameIndex, param_id]
///   SIGPARAM       0xB4  [signatureIndex, param]
///
/// Storage layout:
///   slot 0: owner              — the only address whose signature authorizes a tx,
///                                and the only address that may change the policy
///   slot 1: guard              — the assertion contract the POST_TX frame must target
///   slot 2: policyCommitment   — keccak256 of the exact calldata the POST_TX frame
///                                must carry, so a genuine guard cannot be attached
///                                with permissive parameters
///
/// Revert codes (returned as a bare 4-byte body so the failing check is identifiable):
///   0xE1000001 NotOwnerSigner   — signature[0]'s signer is not the owner
///   0xE1000002 TooFewFrames     — fewer than 2 frames, so there is no body + guard
///   0xE1000003 MissingGuard     — the final frame is not a POST_TX frame
///   0xE1000004 WrongGuard       — the POST_TX frame targets a different contract
///   0xE1000005 PolicyMismatch   — the POST_TX frame's calldata is not the committed one
///
/// ABI:
///   (empty calldata)                              the VERIFY-frame entry point
///   setPolicy(address guard, bytes32 commitment)  0xc7d746fc  owner only
///   policy()                                      0x0505c8c9  -> (owner, guard, commitment)
///
/// @dev FUNDING. Empty calldata is the VERIFY entry point, so a plain value transfer to
///      this account runs the verification path and reverts outside a frame transaction.
///      Fund the account at construction (CREATE with value). This keeps the VERIFY
///      entry unambiguous, which matters more than convenience for a demonstration.
object "GuardMandatingAccount" {
    code {
        // constructor: owner = caller. Guard and policy are set afterwards by the owner.
        sstore(0, caller())
        datacopy(0, dataoffset("runtime"), datasize("runtime"))
        return(0, datasize("runtime"))
    }
    object "runtime" {
        code {
            function fail(code_) {
                mstore(0, shl(224, code_))
                revert(0, 4)
            }

            // ---- VERIFY-frame entry: empty calldata ----
            if iszero(calldatasize()) {
                // 1. Authenticate: the protocol already recovered the signer, so read it
                //    rather than doing an ecrecover here. SIGPARAM 0x00 = effective signer.
                if iszero(eq(verbatim_2i_1o(hex"b4", 0, 0x00), sload(0))) { fail(0xE1000001) }

                // 2. Locate the final frame. TXPARAM 0x09 = len(frames).
                let n := verbatim_1i_1o(hex"b0", 0x09)
                if lt(n, 2) { fail(0xE1000002) }
                let last := sub(n, 1)

                // 3. It must be a POST_TX frame. FRAMEPARAM 0x02 = mode; POST_TX = 3.
                if iszero(eq(verbatim_2i_1o(hex"b3", last, 0x02), 3)) { fail(0xE1000003) }

                // 4. It must target this account's configured guard.
                //    FRAMEPARAM 0x00 = resolved target.
                if iszero(eq(verbatim_2i_1o(hex"b3", last, 0x00), sload(1))) { fail(0xE1000004) }

                // 5. It must carry exactly the committed calldata, so that attaching the
                //    real guard with permissive parameters does not satisfy the policy.
                //    FRAMEPARAM 0x04 = len(data); FRAMEDATACOPY copies that frame's data.
                let dlen := verbatim_2i_1o(hex"b3", last, 0x04)
                verbatim_4i_0o(hex"b2", 0, 0, dlen, last)
                if iszero(eq(keccak256(0, dlen), sload(2))) { fail(0xE1000005) }

                // 6. Authorize execution and payment (scope 0x03).
                verbatim_3i_0o(hex"aa", 0, 0, 0x03)
            }

            let selector := shr(224, calldataload(0))

            // setPolicy(address guard, bytes32 commitment) — owner only
            if eq(selector, 0xc7d746fc) {
                if iszero(eq(caller(), sload(0))) { revert(0, 0) }
                sstore(1, calldataload(4))
                sstore(2, calldataload(36))
                return(0, 0)
            }

            // policy() -> (owner, guard, commitment)
            if eq(selector, 0x0505c8c9) {
                mstore(0, sload(0))
                mstore(32, sload(1))
                mstore(64, sload(2))
                return(0, 96)
            }

            revert(0, 0)
        }
    }
}
