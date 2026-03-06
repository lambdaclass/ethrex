// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title FrameOps
/// @author Lambda Class
/// @notice Solidity wrappers for EIP-8141 Frame Transaction opcodes.
/// @dev These opcodes are not recognized by solc. We use `verbatim` in Yul
///      assembly to emit the raw opcode bytes. Requires compilation with --via-ir.
library FrameOps {
    /// @notice APPROVE (0xAA) — Approve the current frame transaction.
    /// @dev Pops [offset, length, scope] from the stack.
    ///      scope 0x0 = sender approval, 0x1 = payer approval, 0x2 = combined.
    ///      Copies memory[offset..offset+length] to output and halts the frame.
    /// @param offset Memory offset of return data
    /// @param length Length of return data
    /// @param scope Approval scope (0=sender, 1=payer, 2=both)
    function approve(uint256 offset, uint256 length, uint256 scope) internal {
        assembly {
            verbatim_3i_0o(hex"AA", offset, length, scope)
        }
    }

    /// @notice TXPARAMLOAD (0xB0) — Load a transaction parameter as a 32-byte word.
    /// @param paramId The parameter identifier (see EIP-8141 parameter table)
    /// @param index For per-frame parameters, the frame index
    /// @return result The parameter value as a uint256
    function txParamLoad(uint256 paramId, uint256 index) internal view returns (uint256 result) {
        assembly {
            result := verbatim_2i_1o(hex"B0", paramId, index)
        }
    }

    /// @notice TXPARAMSIZE (0xB1) — Get the byte size of a transaction parameter.
    /// @param paramId The parameter identifier
    /// @param index For per-frame parameters, the frame index
    /// @return size The parameter size in bytes
    function txParamSize(uint256 paramId, uint256 index) internal view returns (uint256 size) {
        assembly {
            size := verbatim_2i_1o(hex"B1", paramId, index)
        }
    }

    /// @notice TXPARAMCOPY (0xB2) — Copy transaction parameter data to memory.
    /// @param paramId The parameter identifier
    /// @param index For per-frame parameters, the frame index
    /// @param destOffset Memory destination offset
    /// @param srcOffset Source offset within the parameter data
    /// @param length Number of bytes to copy
    function txParamCopy(uint256 paramId, uint256 index, uint256 destOffset, uint256 srcOffset, uint256 length) internal view {
        assembly {
            verbatim_5i_0o(hex"B2", paramId, index, destOffset, srcOffset, length)
        }
    }
}
