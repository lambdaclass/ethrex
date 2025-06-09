// SPDX-License-Identifier: GPL-3.0
// Contract for testing printing in LEVM with the debug feature enabled.
pragma solidity ^0.8.13;

contract PrintTest {
    function toBytes32(uint256 value) private pure returns (bytes32) {
        return bytes32(value);
    }

    function toBytes32(address addr) private pure returns (bytes32) {
        return bytes32(uint256(uint160(addr)));
    }

    function toBytes32(string memory str) private pure returns (bytes32) {
        return bytes32(bytes(str));
    }

    function print(bytes32 value) private pure {
        assembly {
            mstore(0xFEDEBEBECAFEDECEBADA, value)
        }
    }

    function printAll() public pure {
        uint256 integer = 50000;
        address addr = 0x123456789012345678901234567890123456789a;
        string memory str = unicode"Hello, world! 😛";

        print(toBytes32(integer));
        print(toBytes32(addr));
        print(toBytes32(str));
    }
}
