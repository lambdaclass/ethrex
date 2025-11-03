// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

/// @title IFeeTokenPricer
/// @notice Interface for a contract that provides pricing information for fee tokens.
interface IFeeTokenPricer {
    /// @notice Returns the ratio of fee token (implements FeeToken) to ETH in wei.
    /// @param feeToken The address of the fee token.
    /// @return ratio The amount of fee token (in its smallest unit) equivalent to 1 wei.
    function getFeeTokenRatio(address feeToken) external view returns (uint256);
}
