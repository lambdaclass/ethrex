// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title MPTProof — Merkle Patricia Trie proof verification library.
///
/// Provides functions to verify Ethereum state and storage proofs:
///   - verifyMptProof: core trie traversal from root to leaf
///   - decodeAccountStorageRoot: extract storageRoot from RLP account
///   - decodeRlpUint: decode an RLP-encoded unsigned integer
///
/// Used by NativeRollup to verify L2 withdrawal claims against the L2 state root.
library MPTProof {
    /// @dev Core MPT proof verification. Walks the trie from root to leaf.
    function verifyMptProof(
        bytes32 root,
        bytes memory path,
        bytes[] calldata proof
    ) internal pure returns (bytes memory) {
        bytes32 expectedHash = root;
        uint256 pathOffset = 0;

        for (uint256 i = 0; i < proof.length; i++) {
            bytes calldata node = proof[i];
            require(keccak256(node) == expectedHash, "MPT: invalid node hash");

            (uint256 listLen, uint256 listOffset) = rlpListHeader(node);
            uint256 listEnd = listOffset + listLen;
            uint256 itemCount = rlpListItemCount(node, listOffset, listEnd);

            if (itemCount == 17) {
                // Branch node
                if (pathOffset == path.length) {
                    (bytes memory val, ) = rlpListItem(node, listOffset, listEnd, 16);
                    return val;
                }
                uint8 nibble = uint8(path[pathOffset]);
                pathOffset++;
                (bytes memory child, ) = rlpListItem(node, listOffset, listEnd, nibble);
                require(child.length == 32, "MPT: branch child not hash");
                expectedHash = bytes32(child);
            } else if (itemCount == 2) {
                // Extension or leaf node
                (bytes memory encodedPath, ) = rlpListItem(node, listOffset, listEnd, 0);
                uint256 prefix = uint8(encodedPath[0]) >> 4;
                bool isLeaf = (prefix == 2 || prefix == 3);
                bool isOdd = (prefix == 1 || prefix == 3);
                uint256 nibbleStart = isOdd ? 1 : 2;
                uint256 nibbleCount = encodedPath.length * 2 - nibbleStart;

                for (uint256 j = 0; j < nibbleCount; j++) {
                    require(pathOffset < path.length, "MPT: path too short");
                    uint256 byteIdx = (nibbleStart + j) / 2;
                    uint8 expected;
                    if ((nibbleStart + j) % 2 == 0) {
                        expected = uint8(encodedPath[byteIdx]) >> 4;
                    } else {
                        expected = uint8(encodedPath[byteIdx]) & 0x0f;
                    }
                    require(uint8(path[pathOffset]) == expected, "MPT: path mismatch");
                    pathOffset++;
                }

                (bytes memory next, ) = rlpListItem(node, listOffset, listEnd, 1);
                if (isLeaf) {
                    require(pathOffset == path.length, "MPT: leaf path incomplete");
                    return next;
                }
                require(next.length == 32, "MPT: ext next not hash");
                expectedHash = bytes32(next);
            } else {
                revert("MPT: invalid node");
            }
        }
        revert("MPT: proof incomplete");
    }

    function toNibbles(bytes memory data) internal pure returns (bytes memory nibbles) {
        nibbles = new bytes(data.length * 2);
        for (uint256 i = 0; i < data.length; i++) {
            nibbles[i * 2] = bytes1(uint8(data[i]) >> 4);
            nibbles[i * 2 + 1] = bytes1(uint8(data[i]) & 0x0f);
        }
    }

    function rlpListHeader(bytes calldata data) internal pure returns (uint256 length, uint256 offset) {
        uint8 p = uint8(data[0]);
        if (p >= 0xc0 && p <= 0xf7) {
            return (p - 0xc0, 1);
        }
        uint256 lenBytes = p - 0xf7;
        length = 0;
        for (uint256 i = 0; i < lenBytes; i++) {
            length = (length << 8) | uint8(data[1 + i]);
        }
        offset = 1 + lenBytes;
    }

    function rlpListItemCount(bytes calldata data, uint256 start, uint256 end) internal pure returns (uint256 count) {
        uint256 pos = start;
        while (pos < end) {
            (, uint256 total) = rlpItemLen(data, pos);
            pos += total;
            count++;
        }
    }

    function rlpListItem(bytes calldata data, uint256 start, uint256 end, uint256 idx) internal pure returns (bytes memory item, uint256 itemStart) {
        uint256 pos = start;
        uint256 count = 0;
        while (pos < end) {
            (uint256 cOff, uint256 total) = rlpItemLen(data, pos);
            if (count == idx) {
                uint256 cLen = total - (cOff - pos);
                item = data[cOff : cOff + cLen];
                return (item, pos);
            }
            pos += total;
            count++;
        }
        return (new bytes(0), end);
    }

    function rlpItemLen(bytes calldata data, uint256 pos) internal pure returns (uint256 contentOffset, uint256 totalLength) {
        uint8 p = uint8(data[pos]);
        if (p < 0x80) {
            return (pos, 1);
        } else if (p <= 0xb7) {
            return (pos + 1, 1 + (p - 0x80));
        } else if (p <= 0xbf) {
            uint256 lenBytes = p - 0xb7;
            uint256 len = 0;
            for (uint256 i = 0; i < lenBytes; i++) {
                len = (len << 8) | uint8(data[pos + 1 + i]);
            }
            return (pos + 1 + lenBytes, 1 + lenBytes + len);
        } else if (p <= 0xf7) {
            return (pos + 1, 1 + (p - 0xc0));
        } else {
            uint256 lenBytes = p - 0xf7;
            uint256 len = 0;
            for (uint256 i = 0; i < lenBytes; i++) {
                len = (len << 8) | uint8(data[pos + 1 + i]);
            }
            return (pos + 1 + lenBytes, 1 + lenBytes + len);
        }
    }

    /// @dev Decode storageRoot (3rd field) from RLP-encoded account [nonce, balance, storageRoot, codeHash].
    function decodeAccountStorageRoot(bytes memory account) internal pure returns (bytes32 storageRoot) {
        uint256 pos = 0;
        // Skip list header
        uint8 p = uint8(account[pos]);
        if (p >= 0xf8) { pos += 1 + (uint256(p) - 0xf7); }
        else if (p >= 0xc0) { pos += 1; }
        else { revert("MPT: account not list"); }

        // Skip nonce (item 0)
        pos = skipRlpItem(account, pos);
        // Skip balance (item 1)
        pos = skipRlpItem(account, pos);
        // Read storageRoot (item 2) — must be 32 bytes
        (uint256 cStart, uint256 cLen) = decodeRlpItemMem(account, pos);
        require(cLen == 32, "MPT: storageRoot not 32 bytes");
        assembly ("memory-safe") { storageRoot := mload(add(add(account, 32), cStart)) }
    }

    function skipRlpItem(bytes memory data, uint256 pos) internal pure returns (uint256) {
        uint8 p = uint8(data[pos]);
        if (p < 0x80) return pos + 1;
        if (p <= 0xb7) return pos + 1 + (uint256(p) - 0x80);
        if (p <= 0xbf) {
            uint256 lb = uint256(p) - 0xb7;
            uint256 l = 0;
            for (uint256 i = 0; i < lb; i++) l = (l << 8) | uint8(data[pos+1+i]);
            return pos + 1 + lb + l;
        }
        if (p <= 0xf7) return pos + 1 + (uint256(p) - 0xc0);
        uint256 lb2 = uint256(p) - 0xf7;
        uint256 l2 = 0;
        for (uint256 i = 0; i < lb2; i++) l2 = (l2 << 8) | uint8(data[pos+1+i]);
        return pos + 1 + lb2 + l2;
    }

    function decodeRlpItemMem(bytes memory data, uint256 pos) internal pure returns (uint256 cStart, uint256 cLen) {
        uint8 p = uint8(data[pos]);
        if (p < 0x80) return (pos, 1);
        if (p <= 0xb7) return (pos + 1, uint256(p) - 0x80);
        uint256 lb = uint256(p) - 0xb7;
        cLen = 0;
        for (uint256 i = 0; i < lb; i++) cLen = (cLen << 8) | uint8(data[pos+1+i]);
        cStart = pos + 1 + lb;
    }

    function decodeRlpUint(bytes memory data) internal pure returns (uint256 value) {
        for (uint256 i = 0; i < data.length; i++) {
            value = (value << 8) | uint8(data[i]);
        }
    }
}
