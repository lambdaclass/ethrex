// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "./interfaces/ICommonBridgeL2.sol";
import "./interfaces/IL1Messenger.sol";
import "./interfaces/IERC20L2.sol";

/// @title CommonBridge L2 contract.
/// @author LambdaClass
contract CommonBridgeL2 is ICommonBridgeL2 {
    address public constant L1_MESSENGER = 
        0x000000000000000000000000000000000000FFFE;
    address public constant BURN_ADDRESS =
        0x0000000000000000000000000000000000000000;
    /// @notice Token address used to represent ETH
    address public constant ETH_TOKEN =  0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE;

    function withdraw(address _receiverOnL1) external payable {
        require(msg.value > 0, "Withdrawal amount must be positive");

        (bool success, ) = BURN_ADDRESS.call{value: msg.value}("");
        require(success, "Failed to burn Ether");

        IL1Messenger(L1_MESSENGER).sendMessageToL1(keccak256(abi.encodePacked(
            ETH_TOKEN,
            ETH_TOKEN,
            _receiverOnL1,
            msg.value
        )));
    }

    function mintERC20(address tokenL1, address tokenL2, address destination, uint256 amount) external {
        // The call comes as a privileged transaction, whose sender is the bridge itself.
        require(msg.sender == address(this));
        IERC20L2 token = IERC20L2(tokenL2);
        require(token.l1Address() == tokenL1);
        token.mint(destination, amount);
    }

    function withdrawERC20(address tokenL1, address tokenL2, address destination, uint256 amount) external {
        require(amount > 0, "Withdrawal amount must be positive");

        require(IERC20(tokenL2).transferFrom(msg.sender, address(this), amount), "CommonBridge: burn failed");

        IL1Messenger(L1_MESSENGER).sendMessageToL1(keccak256(abi.encodePacked(
            tokenL1,
            tokenL2,
            destination,
            amount
        )));
    }
}
