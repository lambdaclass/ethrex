// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

import "../../lib/openzeppelin-contracts/contracts/access/Ownable.sol";
import "../../lib/openzeppelin-contracts/contracts/utils/ReentrancyGuard.sol";
import "./interfaces/ICommonBridgeL2.sol";

/// @title CommonBridge L2 contract.
/// @author LambdaClass
contract CommonBridgeL2 is ICommonBridgeL2 {
    address public constant BURN_ADDRESS =
        0x0000000000000000000000000000000000000000;

    function withdraw(address _receiverOnL1) external payable {
            require(msg.value > 0, "Withdrawal amount must be positive");

            (bool success, ) = BURN_ADDRESS.call{value: msg.value}("");
            require(success, "Failed to burn Ether");

            // This event gets pushed to L1, the sequencer monitors
            // them on every block.
            emit WithdrawalInitiated(msg.sender, _receiverOnL1, msg.value);
    }
}
