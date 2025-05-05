// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

interface IAttestation {
    function verifyAndAttestOnChain(bytes calldata rawQuote)
        external
        payable
        returns (bool success, bytes memory output);
}

contract Counter {
    IAttestation constant quoteVerifier = IAttestation(0xe74B98ac3a47615b0Ff8478cB7dEcF332DA0f422);
    uint64 public current = 100;

    function update(uint64 newval, bytes memory quote) public returns (uint64) {
        (bool success, bytes memory report) = quoteVerifier.verifyAndAttestOnChain(quote);
        require(success);
        bytes memory expectedHash = _expectedHash(current, newval);
        require(_rangeEquals(report, 520, expectedHash));
        return newval;
    }

    function _rangeEquals(bytes memory report, uint256 offset, bytes memory other) view internal returns (bool equal) {
        equal = true;
        for (uint256 i; i < other.length; i++) {
            if (report[offset + i] != other[i]) equal = false;
        }
    }
    function _expectedHash(uint64 input, uint64 output) returns (bytes memory) {
        bytes32 a = 0;
        bytes32 b = keccak256(abi.encodePacked([input, output]));
        return abi.encodePacked([a, b]);
    }
}
