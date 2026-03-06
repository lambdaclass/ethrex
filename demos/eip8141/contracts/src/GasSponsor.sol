// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import "../lib/FrameOps.sol";

/// @title GasSponsor
/// @author Lambda Class
/// @notice Sponsors gas for frame transactions. Approves as payer (scope = 1)
///         if the transaction sender holds a minimum ERC20 token balance.
contract GasSponsor {
    /// @notice The ERC20 token address used for balance checks.
    address public token;

    /// @notice Configures the ERC20 token address used for eligibility checks.
    /// @param _token The ERC20 token contract address
    function setConfig(address _token) external {
        token = _token;
    }

    /// @notice Verifies the frame transaction sender holds tokens and approves
    ///         as the gas payer (scope = 1).
    /// @dev Reads the sender address via TXPARAMLOAD(0x02, 0). Performs a
    ///      staticcall to the configured ERC20 token's `balanceOf(sender)`.
    ///      Approves if the sender's balance is greater than zero.
    function verify() external view {
        address sender = address(uint160(FrameOps.txParamLoad(0x02, 0)));

        (bool success, bytes memory result) = token.staticcall(
            abi.encodeWithSignature("balanceOf(address)", sender)
        );
        require(success, "GasSponsor: balanceOf call failed");

        uint256 balance = abi.decode(result, (uint256));
        require(balance > 0, "GasSponsor: sender has no tokens");

        FrameOps.approve(0, 0, 1);
    }

    /// @notice Accepts incoming ETH to fund gas sponsorship.
    receive() external payable {}
}
