// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

/// @title IFeeTokenPricer
/// @notice Interface for a contract that provides pricing information for fee tokens.
interface IFeeTokenPricer {
    /// @notice Emitted when a new ratio is set.
    event FeeTokenRatioSet(address indexed feeToken, uint256 indexed ratio);

    /// @notice Emitted when a ratio is unset.
    event FeeTokenRatioUnset(address indexed feeToken);

    /// @notice Returns the ratio of fee token (implements FeeToken) to ETH in wei.
    /// @param feeToken The address of the fee token.
    /// @return ratio The amount of fee token (in its smallest unit) equivalent to 1 wei.
    function getFeeTokenRatio(address feeToken) external view returns (uint256);

    /// @notice Sets the ratio of a fee token to ETH in wei.
    /// @param feeToken The address of the fee token.
    /// @param ratio Amount of the token (in its smallest unit) per wei.
    function setFeeTokenRatio(address feeToken, uint256 ratio) external;

    /// @notice Removes the ratio of a fee token.
    /// @param feeToken Address of the fee token.
    function unsetFeeTokenRatio(address feeToken) external;
}
