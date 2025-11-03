// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

interface IFeeTokenRegistry {
    /// @notice Returns true if the token is registered as a fee token.
    /// @param token The address of the token to check.
    /// @return True if the token is registered as a fee token, false otherwise.
    function isFeeToken(address token) external view returns (bool);
}
