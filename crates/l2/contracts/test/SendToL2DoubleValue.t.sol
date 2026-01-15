// SPDX-License-Identifier: MIT
pragma solidity ^0.8.31;

import "forge-std/Test.sol";
import "../src/l2/CommonBridgeL2.sol";
import "../src/l2/Messenger.sol";

contract SendToL2DoubleValueTest is Test {
    CommonBridgeL2 bridge;
    Messenger messenger;

    event L2Message(
        uint256 indexed chainId,
        address from,
        address to,
        uint256 value,
        uint256 gasLimit,
        uint256 txId,
        bytes data
    );

    function setUp() public {
        // Deploy messenger at the expected address
        vm.etch(0x000000000000000000000000000000000000FFFE, address(new Messenger()).code);
        messenger = Messenger(0x000000000000000000000000000000000000FFFE);

        // Deploy bridge at the expected address
        vm.etch(0x000000000000000000000000000000000000FFff, address(new CommonBridgeL2()).code);
        bridge = CommonBridgeL2(0x000000000000000000000000000000000000FFff);
    }

    function test_doubleValueInL2Messages() public {
        uint256 destChainId = 2;
        address dest = address(0xBEEF);
        uint256 gasLimit = 100000;
        bytes memory data = "test";
        uint256 sendValue = 1 ether;

        vm.deal(address(this), sendValue);

        // Record events
        vm.recordLogs();

        bridge.sendToL2{value: sendValue}(destChainId, dest, gasLimit, data);

        Vm.Log[] memory logs = vm.getRecordedLogs();

        // Should have 2 L2Message events
        uint256 l2MessageCount = 0;
        uint256 totalValueInMessages = 0;

        for (uint256 i = 0; i < logs.length; i++) {
            if (logs[i].emitter == address(messenger)) {
                l2MessageCount++;
                // Decode value from event data (4th parameter, offset 64-96)
                uint256 msgValue = abi.decode(slice(logs[i].data, 64, 32), (uint256));
                totalValueInMessages += msgValue;

                console.log("Message", l2MessageCount, "value:", msgValue);
            }
        }

        console.log("Total value in messages:", totalValueInMessages);
        console.log("Actual ETH sent:", sendValue);

        // BUG: Both messages have msg.value, so total = 2 * sendValue
        assertEq(l2MessageCount, 2, "Should emit 2 L2Message events");
        assertEq(totalValueInMessages, 2 * sendValue, "BUG CONFIRMED: Double value in messages");
    }

    function slice(bytes memory data, uint256 start, uint256 len) internal pure returns (bytes memory) {
        bytes memory result = new bytes(len);
        for (uint256 i = 0; i < len; i++) {
            result[i] = data[start + i];
        }
        return result;
    }
}
