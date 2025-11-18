// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "../l1/interfaces/ICommonBridge.sol";
import "../l2/interfaces/ICommonBridgeL2.sol";

/// @title Example Counter
/// @author LambdaClass
contract Counter {
    uint256 public number = 0;

    bytes public res;

    function increment() public {
        number += 1;
    }

    function set(uint256 _number) public {
        number = _number;
    }

    function update(address bridge) public {
        bytes memory response = ICommonBridgeL2(address(0xffff)).scopedCall(bridge, abi.encodeCall(ICommonBridge.ON_CHAIN_PROPOSER, ()));
        res = response;
    }
}
