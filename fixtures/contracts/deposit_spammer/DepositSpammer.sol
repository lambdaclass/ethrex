// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

struct SendValues {
    address to;
    uint256 gasLimit;
    uint256 value;
    bytes data;
}

interface ICommonBridge {
    function sendToL2(SendValues calldata sendValues) external;
}

contract InnerSpammer {
    function sendCall(address bridge, uint256 gasLimit) internal {
        SendValues memory values = SendValues({
            to: address(uint160(0xffff)),
            gasLimit: gasLimit,
            value: 1,
            data: ""
        });
        ICommonBridge(bridge).sendToL2(values);
    }
    constructor(address bridge) {
        sendCall(bridge, 1);
        sendCall(bridge, 1_000_000);
        for (uint256 i = 0; i < 10; i++) {
            sendCall(bridge, 1);
        }
    }
}

contract DepositSpammer {
    function spam(address bridge, uint256 n) public {
        for (uint256 i = 0; i < n; i++) {
            new InnerSpammer(bridge);
        }
    }
}
