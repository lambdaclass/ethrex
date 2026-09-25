// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/TDXVerifier.sol";

contract MockAttestation {}

contract MockTimelock {
    function isSequencer(address) external pure returns (bool) {
        return true;
    }
}

contract TDXVerifierTest is Test {
    TDXVerifier verifier;
    MockTimelock timelock;
    MockAttestation attestation;

    address sequencer = address(0x1234);
    address other = address(0x5678);

    function setUp() public {
        timelock = new MockTimelock();
        attestation = new MockAttestation();
        verifier = new TDXVerifier(
            address(attestation),
            address(timelock),
            sequencer,
            true
        );
    }

    function test_register_revertsForNonAuthorizedSequencer() public {
        bytes memory quote = abi.encodePacked(other, new bytes(12));
        vm.prank(other);
        vm.expectRevert("TDXVerifier: only authorized sequencer can update keys");
        verifier.register(quote);
    }

    function test_register_succeedsForAuthorizedSequencerInDevMode() public {
        bytes memory quote = abi.encodePacked(sequencer, new bytes(12));
        vm.prank(sequencer);
        verifier.register(quote);
        assertEq(verifier.authorizedSignature(), sequencer);
    }

    function test_setAuthorizedSequencer_revertsForNonTimelock() public {
        vm.prank(other);
        vm.expectRevert("TDXVerifier: only timelock can set sequencer");
        verifier.setAuthorizedSequencer(other);
    }

    function test_setAuthorizedSequencer_succeedsFromTimelock() public {
        address newSequencer = address(0x9999);
        vm.prank(address(timelock));
        verifier.setAuthorizedSequencer(newSequencer);
        assertEq(verifier.authorizedSequencer(), newSequencer);
    }
}
