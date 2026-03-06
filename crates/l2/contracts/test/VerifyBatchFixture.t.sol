// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "forge-std/Test.sol";
import "./SP1MockVerifier.sol";
import "../src/l1/interfaces/ISP1Verifier.sol";

/// @title Verify Batch Fixture Test (Phase 5b)
/// @notice Tests the on-chain verification flow using fixture data:
///         1. Deploy SP1MockVerifier
///         2. Load fixture public values + proof bytes + VK hash
///         3. Call verifyProof() — verifies the full integration path
///
/// @dev Mock proofs (from groth16_mock_fixtures test) have empty proof bytes.
///      Real Groth16 proofs (from groth16_real_prove test) have actual bytes.
///      SP1MockVerifier accepts both.
///
///      To test with real SP1 verification, deploy the actual SP1Verifier
///      and use real Groth16 fixtures from SP1_DEV=true proving.
contract VerifyBatchFixtureTest is Test {
    SP1MockVerifier public verifier;

    function setUp() public {
        verifier = new SP1MockVerifier();
    }

    /// @notice Test SP1 verifyProof call with zk-dex batch 2 fixture data.
    ///         Uses the same public values from the encoding test to ensure
    ///         the full path works: encode public inputs -> hash -> verify.
    function test_verifyProof_mock_zk_dex_batch_2() public view {
        // -- VK hash from fixture (will be overwritten by actual fixture if available) --
        // This is a placeholder; the mock verifier ignores it anyway.
        bytes32 programVKey = bytes32(0);

        // -- Public values: same as test_encoding_zk_dex_batch_2 --
        bytes32 initialStateHash   = 0x48f7b11fc87cbe873361a3ff5b40c91dc24cf42f2597c43b23e5d5ebd64fca94;
        bytes32 finalStateHash     = 0xb652083c49449bf64a2c2d75f9a933a206ccb75b718e74cebeff78b580aa7b3a;
        bytes32 withdrawalsMerkle  = 0xc55f9da905b0df29c9aaab516d596b67f57c6b60bc10c2a0dbac27744c6c9976;
        bytes32 privTxRollingHash  = 0x00015183d843661fbfd058d685348153652c98d9ec35fcc20f303daa31011239;
        bytes32 blobVersionedHash  = 0x01a3de24063c81ce5bb50727e9b66372583d43283d4f0429863bb4c2f61c0be5;
        bytes32 lastBlockHash      = 0xea82457426f80e6fe99776a37b4201f32813ff791800018f79be49dc75d3a9dd;
        uint256 chainId            = 65536999;
        uint256 nonPrivilegedTxs   = 1;

        bytes memory publicValues = abi.encodePacked(
            initialStateHash,
            finalStateHash,
            withdrawalsMerkle,
            privTxRollingHash,
            blobVersionedHash,
            lastBlockHash,
            bytes32(chainId),
            bytes32(nonPrivilegedTxs)
        );

        // Mock proof: empty bytes (matches SP1 mock proof format)
        bytes memory proofBytes = "";

        // This should NOT revert
        ISP1Verifier(address(verifier)).verifyProof(
            programVKey,
            publicValues,
            proofBytes
        );
    }

    /// @notice Test that verifyProof works with SHA-256 hash verification.
    ///         RISC0 uses sha256(publicInputs) for verification, while SP1
    ///         uses raw publicInputs. This test verifies the SHA-256 path.
    function test_verifyProof_with_sha256_check() public view {
        bytes32 programVKey = bytes32(0);

        // Build public values the same way as the encoding test
        bytes memory publicValues = abi.encodePacked(
            bytes32(0x48f7b11fc87cbe873361a3ff5b40c91dc24cf42f2597c43b23e5d5ebd64fca94),
            bytes32(0xb652083c49449bf64a2c2d75f9a933a206ccb75b718e74cebeff78b580aa7b3a),
            bytes32(0xc55f9da905b0df29c9aaab516d596b67f57c6b60bc10c2a0dbac27744c6c9976),
            bytes32(0x00015183d843661fbfd058d685348153652c98d9ec35fcc20f303daa31011239),
            bytes32(0x01a3de24063c81ce5bb50727e9b66372583d43283d4f0429863bb4c2f61c0be5),
            bytes32(0xea82457426f80e6fe99776a37b4201f32813ff791800018f79be49dc75d3a9dd),
            bytes32(uint256(65536999)),
            bytes32(uint256(1))
        );

        // Verify raw public values via mock verifier
        ISP1Verifier(address(verifier)).verifyProof(
            programVKey,
            publicValues,
            ""
        );

        // Also verify sha256(publicValues) matches expected (RISC0 path)
        bytes32 hash = sha256(publicValues);
        bytes32 expectedHash = 0x11f0ad405278dfd9a8bb883a7ba632031b4b0af97c5d2603f6048ec37c984811;
        assertEq(hash, expectedHash, "SHA-256 hash of public values mismatch");
    }

    /// @notice Test the full verifyBatch-like flow:
    ///         1. Encode public inputs from commitment fields (like OnChainProposer)
    ///         2. Call verifyProof with the encoded values
    ///         This validates the end-to-end encoding + verification path.
    function test_full_verify_flow_zk_dex() public view {
        // Simulate what OnChainProposer._getPublicInputsFromCommitment does
        bytes32 prevStateRoot      = 0x48f7b11fc87cbe873361a3ff5b40c91dc24cf42f2597c43b23e5d5ebd64fca94;
        bytes32 newStateRoot       = 0xb652083c49449bf64a2c2d75f9a933a206ccb75b718e74cebeff78b580aa7b3a;
        bytes32 withdrawalsMerkle  = 0xc55f9da905b0df29c9aaab516d596b67f57c6b60bc10c2a0dbac27744c6c9976;
        bytes32 privTxRollingHash  = 0x00015183d843661fbfd058d685348153652c98d9ec35fcc20f303daa31011239;
        bytes32 blobVersionedHash  = 0x01a3de24063c81ce5bb50727e9b66372583d43283d4f0429863bb4c2f61c0be5;
        bytes32 lastBlockHash      = 0xea82457426f80e6fe99776a37b4201f32813ff791800018f79be49dc75d3a9dd;
        uint256 chainId            = 65536999;
        uint256 nonPrivilegedTxs   = 1;

        // Reconstruct public inputs exactly like _getPublicInputsFromCommitment
        bytes memory publicInputs = abi.encodePacked(
            prevStateRoot,      // initialStateHash (from previous batch)
            newStateRoot,       // finalStateHash
            withdrawalsMerkle,  // l1_out_messages_merkle_root
            privTxRollingHash,  // l1_in_messages_rolling_hash
            blobVersionedHash,  // blob_versioned_hash
            lastBlockHash,      // last_block_hash
            bytes32(chainId),   // chain_id
            bytes32(nonPrivilegedTxs) // non_privileged_count
        );

        // Length check: 8 fields * 32 bytes = 256 bytes
        assertEq(publicInputs.length, 256, "Fixed-field public inputs should be 256 bytes");

        // Verify with mock verifier (simulates verifyBatch's SP1 verification path)
        bytes32 programVKey = bytes32(0);
        ISP1Verifier(address(verifier)).verifyProof(
            programVKey,
            publicInputs,
            ""
        );
    }

    /// @notice Test with non-empty proof bytes (simulates real Groth16 calldata).
    ///         Mock verifier should accept any proof bytes.
    function test_verifyProof_nonempty_proof() public view {
        bytes32 programVKey = bytes32(uint256(0x1234));

        bytes memory publicValues = hex"deadbeef";

        // Non-empty proof bytes (simulates Groth16 selector + proof)
        bytes memory proofBytes = hex"aabbccdd" // 4-byte vkey selector
            hex"0000000000000000000000000000000000000000000000000000000000000001"
            hex"0000000000000000000000000000000000000000000000000000000000000002";

        ISP1Verifier(address(verifier)).verifyProof(
            programVKey,
            publicValues,
            proofBytes
        );
    }
}
