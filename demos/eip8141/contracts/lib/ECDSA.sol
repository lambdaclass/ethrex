// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

library ECDSA {
    struct PublicKey {
        uint256 x;
        uint256 y;
    }

    struct Signature {
        uint256 r;
        uint256 s;
    }

    struct RecoveredSignature {
        uint256 r;
        uint256 s;
        uint8 yParity;
    }
}
