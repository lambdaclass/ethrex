// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @notice Calls the ECMUL precompile (0x07) and stores the result on-chain.
/// Used in integration tests to ensure the prover guest computes the same
/// precompile output as the sequencer. Any mismatch (e.g. Jacobian vs affine
/// coordinates) causes a state root divergence and proof failure.
contract EcmulStore {
    uint256 public storedX;
    uint256 public storedY;

    function ecmulAndStore(uint256 px, uint256 py, uint256 s) external {
        bytes memory input = abi.encodePacked(px, py, s);
        (bool success, bytes memory result) = address(7).staticcall(input);
        require(success, "ECMUL precompile call failed");
        require(result.length == 64, "Invalid ECMUL result length");

        uint256 rx;
        uint256 ry;
        assembly {
            rx := mload(add(result, 32))
            ry := mload(add(result, 64))
        }
        storedX = rx;
        storedY = ry;
    }
}
