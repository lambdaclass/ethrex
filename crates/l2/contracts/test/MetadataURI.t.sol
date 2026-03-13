// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "forge-std/Test.sol";
import "../src/l1/OnChainProposer.sol";
import "../src/l1/interfaces/IOnChainProposer.sol";

/// @title MetadataURI Tests
/// @notice Tests setMetadataURI on OnChainProposer: storage, events, access control.
contract MetadataURITest is Test {
    OnChainProposer public proposer;
    address public owner;
    address public nonOwner;

    function setUp() public {
        owner = address(this);
        nonOwner = makeAddr("nonOwner");

        proposer = new OnChainProposer();

        // Minimal initialize: no proofs required, dummy bridge, owner = this contract
        address dummyBridge = makeAddr("bridge");
        bytes32 commitHash = bytes32(uint256(1));
        bytes32 genesisStateRoot = bytes32(uint256(0xdead));

        proposer.initialize(
            false,              // validium
            owner,              // timelock_owner
            false,              // requireRisc0Proof
            false,              // requireSp1Proof
            false,              // requireTdxProof
            false,              // aligned
            address(0),         // r0verifier
            address(0),         // sp1verifier
            address(0),         // tdxverifier
            address(0),         // alignedProofAggregator
            bytes32(0),         // sp1Vk
            bytes32(0),         // risc0Vk
            commitHash,         // commitHash
            genesisStateRoot,   // genesisStateRoot
            12345,              // chainId
            dummyBridge,        // bridge
            address(0)          // guestProgramRegistry
        );
    }

    /// @notice metadataURI is empty after initialization.
    function test_metadataURI_initially_empty() public view {
        assertEq(proposer.metadataURI(), "");
    }

    /// @notice Owner can set metadataURI and storage is updated.
    function test_setMetadataURI_updates_storage() public {
        string memory uri = "ipfs://QmTest123";
        proposer.setMetadataURI(uri);
        assertEq(proposer.metadataURI(), uri);
    }

    /// @notice setMetadataURI emits MetadataURIUpdated event.
    function test_setMetadataURI_emits_event() public {
        string memory uri = "ipfs://QmEventTest";
        vm.expectEmit(false, false, false, true);
        emit IOnChainProposer.MetadataURIUpdated(uri);
        proposer.setMetadataURI(uri);
    }

    /// @notice Multiple updates overwrite the previous value.
    function test_setMetadataURI_overwrites() public {
        proposer.setMetadataURI("ipfs://QmFirst");
        assertEq(proposer.metadataURI(), "ipfs://QmFirst");

        proposer.setMetadataURI("ipfs://QmSecond");
        assertEq(proposer.metadataURI(), "ipfs://QmSecond");
    }

    /// @notice Non-owner cannot call setMetadataURI.
    function test_setMetadataURI_reverts_nonOwner() public {
        vm.prank(nonOwner);
        vm.expectRevert();
        proposer.setMetadataURI("ipfs://QmUnauthorized");
    }

    /// @notice Setting an empty string is allowed.
    function test_setMetadataURI_empty_string() public {
        proposer.setMetadataURI("ipfs://QmSomething");
        assertEq(proposer.metadataURI(), "ipfs://QmSomething");

        proposer.setMetadataURI("");
        assertEq(proposer.metadataURI(), "");
    }
}
