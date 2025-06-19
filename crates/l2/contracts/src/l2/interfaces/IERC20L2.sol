// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @title Interface for an L2-capable token.
/// @author LambdaClass
interface IERC20L2 is IERC20 {
    /// @notice Returns the address of the token on the L1
    /// @dev Used to verify token reception.
    function l1Address() external pure returns (address);
    /// @notice Mints tokens to the givne address
    /// @dev Should be callable by the bridge
    function mint(address destination, uint256 amount) external;
}
