// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "./IERC20L2.sol";

/// @title IFeeToken
/// @notice Interface that L2 fee tokens must implement so the sequencer can lock and distribute fees.
interface IFeeToken is IERC20, IERC20L2 {
    /// @notice Locks `amount` of tokens from `payer`. Must only be callable by the fee collector.
    /// @dev The L2 hook invokes this to reserve funds up-front.
    function lockFee(address payer, uint256 amount) external;

    /// @notice Pays `amount` of tokens to `receiver`. Must only be callable by the fee collector.
    /// @dev A zero receiver is treated as a burn by the reference implementation.
    function payFee(address receiver, uint256 amount) external;
}
