// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title L2WithdrawalBridge — Withdrawal bridge for Native Rollups PoC.
///
/// Users call withdraw(receiverOnL1) with ETH to initiate a withdrawal from L2.
/// The EXECUTE precompile detects WithdrawalInitiated events and includes them
/// in the withdrawal Merkle root returned alongside the post-state root.
///
/// The received ETH is burned by sending it to address(0), matching the
/// existing CommonBridgeL2 pattern.
contract L2WithdrawalBridge {
    address public constant BURN_ADDRESS =
        0x0000000000000000000000000000000000000000;

    uint256 public messageId;

    event WithdrawalInitiated(
        address indexed from,
        address indexed receiver,
        uint256 amount,
        uint256 indexed messageId
    );

    /// @notice Initiate a withdrawal by sending ETH with the L1 receiver address.
    /// @param _receiver Address on L1 that will receive the withdrawn ETH.
    function withdraw(address _receiver) external payable {
        require(msg.value > 0, "Withdrawal amount must be positive");
        require(_receiver != address(0), "Invalid receiver");

        (bool success, ) = BURN_ADDRESS.call{value: msg.value}("");
        require(success, "Failed to burn Ether");

        uint256 currentMessageId = messageId;
        messageId = currentMessageId + 1;

        emit WithdrawalInitiated(msg.sender, _receiver, msg.value, currentMessageId);
    }
}
