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
    IAttestation public quoteVerifier = IAttestation(address(0));
    address public authorizedSignature = address(0);
    uint64 public current = 100;
    bytes public RTMR0 = hex'4f3d617a1c89bd9a89ea146c15b04383b7db7318f41a851802bba8eace5a6cf71050e65f65fd50176e4f006764a42643';
    bytes public RTMR1 = hex'ff6520a395eadfb0b8621f08cc2816598dfe87271c1705f8ed6a9ea989343b7d25ad2c7c04731a6e94ef14b451d4e1f0';
    bytes public RTMR2 = hex'a873631d01cf9faf887caab6b3c77a79927845ae7b99754a93699dae0af79c7d8520e8e371be5655dc8a68ad5c0e4d6d';
    bytes public MRTD = hex'91eb2b44d141d4ece09f0c75c2c53d247a3c68edd7fafe8a3520c942a604a407de03ae6dc5f87f27428b2538873118b7';

    constructor(address _dcap) {
        quoteVerifier = IAttestation(_dcap);
    }

    function reset() public {
        current = 100;
        authorizedSignature = address(0);
    }

    function update(uint256 _to, bytes memory signature) public {
        uint64 to = uint64(_to);
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
