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
    bytes public RTMR0 = hex'4f3d617a1c89bd9a89ea146c15b04383b7db7318f41a851802bba8eace5a6cf71050e65f65fd50176e4f006764a42643';
    bytes public RTMR1 = hex'1513041951b4a2da3c982b7146dbb4c4cc6e4cac8bd0cba86e1423d5a49393aae4b682ca6bdd9c280a71624c6e1e0044';
    bytes public RTMR2 = hex'6357e84fc4a382f1b7552ddcf504d88cd75b9847d7957da3b5181e6f50a183f30e8239f19ab4cf1fc7752091160ed4a9';

    function update(uint64 newval, bytes memory quote) public returns (uint64) {
        (bool success, bytes memory report) = quoteVerifier.verifyAndAttestOnChain(quote);
        require(success, "quote verification failed");
        bytes memory expected = expectedHash(current, newval);
        require(report[6] == 0, "TCB_STATUS != OK");
        require(_rangeEquals(report, 341, RTMR0), "RTMR0 mismatch");
        require(_rangeEquals(report, 389, RTMR1), "RTMR1 mismatch");
        require(_rangeEquals(report, 437, RTMR2), "RTMR2 mismatch");
        // RTMR3 is ignored
        require(_rangeEquals(report, 533, expected), "hash mismatch");
        current = newval;
        return current;
    }

    function _rangeEquals(bytes memory report, uint256 offset, bytes memory other) pure internal returns (bool) {
        for (uint256 i; i < other.length; i++) {
            if (report[offset + i] != other[i]) return false;
        }
        return true;
    }

    function expectedHash(uint64 input, uint64 output) pure public returns (bytes memory) {
        bytes32 a = 0;
        bytes32 b = keccak256(abi.encodePacked(input, output));
        return abi.encodePacked(a, b);
    }
}
