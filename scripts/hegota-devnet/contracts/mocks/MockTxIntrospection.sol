// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {ITxIntrospection} from "../guards/ITxIntrospection.sol";

/// @title MockTxIntrospection
/// @notice A scriptable stand-in for the Yul introspection shim, so guard LOGIC can be tested
///         without a chain that implements the EIP-7906 opcodes.
///
/// @dev Why this exists. No local EVM implements TXTRACE/TXDIFF/EVENTDATACOPY, so an all-Yul
///      guard could only ever be exercised by a devnet round-trip. Because the guards read
///      through an interface, tests can inject scripted diff data here and cover the cases the
///      devnet scenarios cannot reach cheaply: empty diffs, boundary indices, allowlist misses,
///      and slot-derivation correctness.
///
///      These tests establish that a guard's logic is right. The devnet scenarios establish
///      that the real opcodes behave as assumed. Neither substitutes for the other — a guard
///      passing here can still be defeated by the simulation/block-building divergence
///      documented in pocs/GATE-RESULTS.md.
contract MockTxIntrospection is ITxIntrospection {
    mapping(uint256 => mapping(uint256 => uint256)) public traceValue;   // param => in2 => value
    mapping(uint256 => mapping(address => mapping(uint256 => uint256))) public diffValue;
    bytes public eventPayload;

    function setTrace(uint256 param, uint256 in2, uint256 value) external {
        traceValue[param][in2] = value;
    }

    function setDiff(uint256 param, address account, uint256 in3, uint256 value) external {
        diffValue[param][account][in3] = value;
    }

    function setEventPayload(bytes calldata payload) external {
        eventPayload = payload;
    }

    function txtrace(uint256 param, uint256 in2) external view returns (uint256) {
        return traceValue[param][in2];
    }

    function txdiff(uint256 param, address account, uint256 in3) external view returns (uint256) {
        return diffValue[param][account][in3];
    }

    function eventData(uint256, uint256, uint256) external view returns (bytes memory) {
        return eventPayload;
    }
}
