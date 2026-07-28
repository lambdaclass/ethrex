// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {ITxIntrospection} from "./ITxIntrospection.sol";

/// @title ShimProbe
/// @notice Smoke test proving that a **Solidity** contract can read the EIP-7906
///         transaction diff through the Yul introspection shim while running inside a
///         POST_TX frame. This is the load-bearing assumption behind writing the whole
///         guard library in Solidity rather than Yul, so it is kept as a permanent
///         regression check rather than deleted after the initial spike.
///
/// The two entry points are deliberately complementary: against any transaction that
/// moves value, `assertBalanceChangesAtLeast(shim, 1)` must succeed and
/// `assertBalanceChangesEquals(shim, 0)` must revert. A shim that returned garbage, or
/// an opcode gate that did not hold across the STATICCALL, could not produce that pair.
contract ShimProbe {
    uint256 private constant BALANCE_CHANGE_COUNT = 0x00;

    error CountBelowMinimum(uint256 got, uint256 min);
    error CountNotEqual(uint256 got, uint256 want);

    /// @notice Passes when the transaction changed at least `min` balances.
    function assertBalanceChangesAtLeast(ITxIntrospection shim, uint256 min) external view {
        uint256 n = shim.txtrace(BALANCE_CHANGE_COUNT, 0);
        if (n < min) revert CountBelowMinimum(n, min);
    }

    /// @notice Passes only when the transaction changed exactly `want` balances.
    function assertBalanceChangesEquals(ITxIntrospection shim, uint256 want) external view {
        uint256 n = shim.txtrace(BALANCE_CHANGE_COUNT, 0);
        if (n != want) revert CountNotEqual(n, want);
    }
}
