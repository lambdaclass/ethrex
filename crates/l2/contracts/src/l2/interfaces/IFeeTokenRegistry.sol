// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

interface IFeeTokenRegistry {
    /// @notice Emitted when a new fee token is registered.
    event FeeTokenRegistered(address indexed token);
    /// @notice Emitted when a fee token is unregistered.
    event FeeTokenUnregistered(address indexed token);

    /// @notice Returns true if the token is registered as a fee token.
    /// @param token The address of the token to check.
    /// @return True if the token is registered as a fee token, false otherwise.
    function isFeeToken(address token) external view returns (bool);

    /// @notice Registers a new fee token.
    /// @param token The address of the token to register.
    function registerFeeToken(address token) external;

    /// @notice Unregisters a fee token.
    /// @param token The address of the token to unregister.
    function unregisterFeeToken(address token) external;
}
