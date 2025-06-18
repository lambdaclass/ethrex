// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "./interfaces/ICommonBridgeL2.sol";
import "./interfaces/IL1Messenger.sol";

/// @title CommonBridge L2 contract.
/// @author LambdaClass
contract CommonBridgeL2 is ICommonBridgeL2 {
    address public constant L1_MESSENGER = 
        0x000000000000000000000000000000000000FFFE;
    address public constant BURN_ADDRESS =
        0x0000000000000000000000000000000000000000;

    function withdraw(address _receiverOnL1) external payable {
        require(msg.value > 0, "Withdrawal amount must be positive");

        (bool success, ) = BURN_ADDRESS.call{value: msg.value}("");
        require(success, "Failed to burn Ether");

        IL1Messenger(L1_MESSENGER).sendMessageToL1(keccak256(abi.encodePacked(
            _receiverOnL1,
            msg.value
        )));
    }
}
