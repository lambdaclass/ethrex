// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "../lib/openzeppelin-contracts/contracts/utils/cryptography/ECDSA.sol";
import "../lib/openzeppelin-contracts/contracts/utils/cryptography/MessageHashUtils.sol";

interface IAttestation {
    function verifyAndAttestOnChain(bytes calldata rawQuote)
        external
        payable
        returns (bool success, bytes memory output);
}

contract Counter {
    IAttestation quoteVerifier = IAttestation(address(0));
    constructor(address _dcap) {
        quoteVerifier = IAttestation(_dcap);
    }
    address public authorizedSignature = address(0);
    uint64 public current = 100;
    bytes public RTMR0 = hex'4f3d617a1c89bd9a89ea146c15b04383b7db7318f41a851802bba8eace5a6cf71050e65f65fd50176e4f006764a42643';
    bytes public RTMR1 = hex'4a2b3ad09e6ae0b9a2189f1f7564b7ca88c4c634fc26a5e5faec2196a810f58da59fce360d2fbb065f35627d36e2ce11';
    bytes public RTMR2 = hex'f85ab445249999ee2c5e71919b1f42cc746ba07a4bd2464fa2b4e61ff06232e51bb8976db6c3cb040e7e033931332fa7';
    bytes public MRTD = hex'91eb2b44d141d4ece09f0c75c2c53d247a3c68edd7fafe8a3520c942a604a407de03ae6dc5f87f27428b2538873118b7';

    function update(uint256 to, bytes memory signature) public {
        uint64 to = uint64(to);
        require(authorizedSignature != address(0), "authorized signer not set");
        bytes32 signedHash = MessageHashUtils.toEthSignedMessageHash(abi.encodePacked(current, to));
        require(ECDSA.recover(signedHash, signature) == authorizedSignature, "invalid signature");
        current = to;
    }

    function updateKey(address signer, bytes memory quote) public {
        // TODO: only allow the owner to update the key, to avoid DoS
        (bool success, bytes memory report) = quoteVerifier.verifyAndAttestOnChain(quote);
        require(success, "quote verification failed");
        require(_rangeEquals(report, 0, hex'0004'), "Unsupported quote version");
        require(report[2] == 0x81, "Quote is not of type TDX");
        require(report[6] == 0, "TCB_STATUS != OK");
        require(uint8(report[133]) & 15 == 0, "debug attributes are set");
        require(_rangeEquals(report, 149, MRTD), "MRTD mismatch");
        require(_rangeEquals(report, 341, RTMR0), "RTMR0 mismatch");
        require(_rangeEquals(report, 389, RTMR1), "RTMR1 mismatch");
        require(_rangeEquals(report, 437, RTMR2), "RTMR2 mismatch");
        // RTMR3 is ignored
        require(_rangeEquals(report, 533, expectedReportData(signer)), "reportData mismatch");
        authorizedSignature = signer;
    }

    function _rangeEquals(bytes memory report, uint256 offset, bytes memory other) pure internal returns (bool) {
        for (uint256 i; i < other.length; i++) {
            if (report[offset + i] != other[i]) return false;
        }
        return true;
    }

    function expectedReportData(address addr) pure public returns (bytes memory) {
        return abi.encodePacked(bytes20(addr), bytes20(0), bytes20(0), bytes4(0));
    }
}
