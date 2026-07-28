/// @title TxIntrospection
/// @notice Exposes the three EIP-7906 transaction-introspection opcodes as ordinary
///         ABI functions so that assertion guards can be written in readable
///         Solidity instead of raw Yul.
///
/// @dev WHY THIS EXISTS. Solidity cannot emit the EIP-7906 opcodes: `verbatim_*` is
///      available only in pure Yul, not inside a Solidity `assembly { }` block
///      (verified on solc 0.8.31, both the legacy and --via-ir pipelines). Without
///      this shim every guard would have to be hand-written Yul. With it, the raw
///      opcodes live in exactly one auditable place and guard logic stays readable.
///
/// @dev WHY IT IS SOUND. The opcodes are gated on the mode of the *enclosing
///      transaction frame*, not on the immediate call frame, and that gate holds
///      throughout the POST_TX call subtree. A STATICCALL from a POST_TX guard into
///      this shim therefore still satisfies the gate. A POST_TX frame is already
///      static, so calling out of it is permitted.
///
/// @dev TRUST. This contract holds no storage, has no owner, and performs no logic
///      beyond argument marshalling. A malicious variant could only make a guard
///      PASS, which the guards' mandatory negative controls are designed to catch.
///
/// Opcode bytes are the hegota-devnet assignment (TXTRACE/EVENTDATACOPY/TXDIFF
/// shifted up one byte from the draft because EIP-8272 owns 0xB5 here):
///   TXTRACE       0xB6  stack [in2, param]                        -> value
///   EVENTDATACOPY 0xB7  stack [eventIndex, memOff, dataOff, len]  -> (copies to memory)
///   TXDIFF        0xB8  stack [param, address, in3]               -> value
///
/// Stack note: for `verbatim_ni_1o(code, a1, ..., an)` the FIRST argument ends up on
/// top of the stack, i.e. it is the operand the opcode pops first. The argument
/// order below therefore mirrors each opcode's documented pop order exactly.
///
/// ABI:
///   txtrace(uint256 param, uint256 in2)                  0x55f6f8d7 -> uint256
///   txdiff(uint256 param, address account, uint256 in3)  0x42abe519 -> uint256
///   eventData(uint256 idx, uint256 off, uint256 len)     0x50ef5616 -> bytes
object "TxIntrospection" {
    code {
        datacopy(0, dataoffset("runtime"), datasize("runtime"))
        return(0, datasize("runtime"))
    }
    object "runtime" {
        code {
            let selector := shr(224, calldataload(0))

            // txtrace(uint256 param, uint256 in2) -> uint256
            if eq(selector, 0x55f6f8d7) {
                let param := calldataload(4)
                let in2 := calldataload(36)
                // TXTRACE pops in2 first, then param.
                mstore(0, verbatim_2i_1o(hex"b6", in2, param))
                return(0, 32)
            }

            // txdiff(uint256 param, address account, uint256 in3) -> uint256
            if eq(selector, 0x42abe519) {
                let param := calldataload(4)
                let account := calldataload(36)
                let in3 := calldataload(68)
                // TXDIFF pops param, then address, then in3.
                mstore(0, verbatim_3i_1o(hex"b8", param, account, in3))
                return(0, 32)
            }

            // eventData(uint256 idx, uint256 off, uint256 len) -> bytes
            if eq(selector, 0x50ef5616) {
                let idx := calldataload(4)
                let off := calldataload(36)
                let len := calldataload(68)
                // ABI-encode a dynamic bytes return: offset, length, then the payload
                // copied straight from the event's data by EVENTDATACOPY.
                mstore(0, 32)
                mstore(32, len)
                // EVENTDATACOPY pops eventIndex, memOff, dataOff, length.
                verbatim_4i_0o(hex"b7", idx, 64, off, len)
                // Pad the payload to a 32-byte boundary.
                return(0, add(64, and(add(len, 31), not(31))))
            }

            revert(0, 0)
        }
    }
}
