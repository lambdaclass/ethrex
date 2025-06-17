// SPDX-License-Identifier: GPL-3.0
// Contract for testing printing in LEVM with the debug feature enabled.
// If used without this feature enabled, execution of contract will inevitably revert in MSTORE opcode.
// Import this with `import "path_to_file/Print.sol"`
pragma solidity ^0.8.0;

// Functions:
// print(e)
//   Print one element of any length in UTF-8 format.
//   `e` can be either a uint256, address, bytes32 or a string.
//   Example: print(unicode"Execution was incredibly successful 😄🎉")
// print(e1, e2)
//   Here at least one of both elements has to be a string.
//   Example: print("Value sent in transaction: ", msg.value);

// Note: uint256 is printed as decimal. If hexadecimal is preferred, wrap it in bytes32. Example: print(bytes32(number))
// Note 2: When doing print("something") Solidity can't infer if it is bytes32 or string, so we should clarify it by casting into string.

uint256 constant MAGIC_PRINT_OFFSET = 0xFEDEBEBECAFEDECEBADA;

// Main logic.
function print(string memory _str) pure {
    bytes32[] memory chunks = splitStringIntoChunks(_str);
    uint256 chunkCount = chunks.length;

    assembly {
        mstore(MAGIC_PRINT_OFFSET, 0) // Enable print mode
    }

    // Fill buffer with data (logic in LEVM)
    for (uint256 i = 0; i < chunkCount; i++) {
        bytes32 chunk = chunks[i];
        assembly {
            mstore(0, chunk)
        }
    }

    // Print data stored in buffer
    assembly {
        mstore(MAGIC_PRINT_OFFSET, 0) // Disable print mode
    }
}

function print(bytes32 _bytes) pure {
    print(bytes32ToString(_bytes));
}

function print(uint256 _i) pure {
    print(uintToDecString(_i));
}

function print(address _addr) pure {
    print(addressToString(_addr));
}

function print(string memory a, string memory b) pure {
    bytes memory packed = abi.encodePacked(a, b);
    print(string(packed));
}

function print(string memory a, address b) pure {
    print(a, addressToString(b));
}

function print(string memory a, uint256 b) pure {
    print(a, uintToDecString(b));
}

function print(uint256 a, string memory b) pure {
    print(uintToDecString(a), b);
}

function print(address a, string memory b) pure {
    print(addressToString(a), b);
}

function print(string memory a, bytes32 b) pure {
    print(a, bytes32ToString(b));
}

function print(bytes32 a, string memory b) pure {
    print(bytes32ToString(a), b);
}

contract PrintTest {
    function test() public pure {
        uint256 integer = 50000;
        address addr = 0x123456789012345678901234567890123456789a;
        string memory unicode_str = unicode"Hello, world! 🥩 ";
        string memory ascii_str = "ASCII works fine too!";
        string
            memory longstring = unicode"This is very likely the longest string that you will ever see in solidity. I don't know why would someone write something this long but it's pretty cool to be able to do so 🕺🏼";
        bytes32 myBytes = "HelloBytes32Value";

        // Print simple types
        print(ascii_str);
        print(longstring);
        print(integer);
        print(addr);
        print(myBytes); // Notice this won't print the string, just the bytes.
        print(string(abi.encodePacked(myBytes))); // But this will

        // Some possible combinations between types
        print(unicode_str, ascii_str);
        print("This is fifty thousand:", integer);
        print(addr, " is a random address.");
        print(unicode_str, addr);
    }
}

// ----- Helper functions ----- //

function addressToString(address _addr) pure returns (string memory) {
    return toHexString(abi.encodePacked(_addr));
}

function bytes32ToString(bytes32 _data) pure returns (string memory) {
    return toHexString(abi.encodePacked(_data));
}

function toHexString(bytes memory data) pure returns (string memory) {
    bytes memory hexChars = "0123456789abcdef";
    bytes memory str = new bytes(2 + data.length * 2);
    str[0] = "0";
    str[1] = "x";
    for (uint i = 0; i < data.length; i++) {
        str[2 + i * 2] = hexChars[uint8(data[i] >> 4)];
        str[3 + i * 2] = hexChars[uint8(data[i] & 0x0f)];
    }
    return string(str);
}

// This converts the number to a decimal UTF-8 readable format.
// Example: 5000 -> "5000"
function uintToDecString(uint256 _i) pure returns (string memory) {
    if (_i == 0) return "0";
    uint256 j = _i;
    uint256 len;
    while (j != 0) {
        len++;
        j /= 10;
    }
    bytes memory bstr = new bytes(len);
    uint256 k = len;
    while (_i != 0) {
        k--;
        bstr[k] = bytes1(uint8(48 + (_i % 10)));
        _i /= 10;
    }
    return string(bstr);
}

// The EVM uses 32 byte values, so if what we want to print is larger than that we'll have to do it in chunks.
function splitStringIntoChunks(
    string memory str
) pure returns (bytes32[] memory) {
    // Convert string to bytes first
    bytes memory strBytes = bytes(str);
    uint256 length = strBytes.length;

    // Calculate number of 32-byte chunks needed
    uint256 chunksCount = (length + 31) / 32;
    bytes32[] memory chunks = new bytes32[](chunksCount);

    // Fill the chunks array
    for (uint256 i = 0; i < chunksCount; i++) {
        uint256 chunkStart = i * 32;
        uint256 chunkLength = (chunkStart + 32 > length)
            ? length - chunkStart
            : 32;

        bytes memory temp = new bytes(32);
        for (uint256 j = 0; j < chunkLength; j++) {
            temp[j] = strBytes[chunkStart + j];
        }

        chunks[i] = bytes32(temp);
    }

    return chunks;
}
