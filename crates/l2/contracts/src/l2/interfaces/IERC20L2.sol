// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @title Interface for an L2-capable token.
/// @author LambdaClass
/// @dev Uses the interface described in the ERC-7802 draft
interface IERC20L2 is IERC20 {
    /// @notice Returns the address of the token on the L1
    /// @dev Used to verify token reception.
    function l1Address() external returns (address);

    /// @notice Mints tokens to the given address
    /// @dev Should be callable by the bridge
    function crosschainMint(address to, uint256 amount) external;

    /// @notice Burns tokens from the given address
    /// @dev Should be callable by the bridge
    function crosschainBurn(address from, uint256 amount) external;
}
