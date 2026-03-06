// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "forge-std/Test.sol";

/// @title Public Inputs Encoding Test
/// @notice Verifies that the Solidity `abi.encodePacked` encoding of
///         ProgramOutput matches the Rust-side encoding stored in fixtures.
///         This catches Solidity/Rust encoding mismatches without needing
///         a real ZK proof.
contract PublicInputsEncodingTest is Test {
    /// @notice Reproduces `_getPublicInputsFromCommitment` logic from
    ///         OnChainProposer.sol and compares against fixture data.
    ///
    ///         Fixture: zk-dex batch 2 (deposit + withdrawal)
    function test_encoding_zk_dex_batch_2() public pure {
        // -- Fixture values from prover.json / committer.json --
        bytes32 initialStateHash   = 0x48f7b11fc87cbe873361a3ff5b40c91dc24cf42f2597c43b23e5d5ebd64fca94;
        bytes32 finalStateHash     = 0xb652083c49449bf64a2c2d75f9a933a206ccb75b718e74cebeff78b580aa7b3a;
        bytes32 withdrawalsMerkle  = 0xc55f9da905b0df29c9aaab516d596b67f57c6b60bc10c2a0dbac27744c6c9976;
        bytes32 privTxRollingHash  = 0x00015183d843661fbfd058d685348153652c98d9ec35fcc20f303daa31011239;
        bytes32 blobVersionedHash  = 0x01a3de24063c81ce5bb50727e9b66372583d43283d4f0429863bb4c2f61c0be5;
        bytes32 lastBlockHash      = 0xea82457426f80e6fe99776a37b4201f32813ff791800018f79be49dc75d3a9dd;
        uint256 chainId            = 65536999; // 0x3e803e7
        uint256 nonPrivilegedTxs   = 1;

        // Encode exactly like _getPublicInputsFromCommitment (8 fixed fields)
        bytes memory publicInputs = abi.encodePacked(
            initialStateHash,
            finalStateHash,
            withdrawalsMerkle,
            privTxRollingHash,
            blobVersionedHash,
            lastBlockHash,
            bytes32(chainId),
            bytes32(nonPrivilegedTxs)
        );

        // Expected: prover.encoded_public_values (no balance_diffs, no l2 rolling hashes)
        bytes memory expected = hex"48f7b11fc87cbe873361a3ff5b40c91dc24cf42f2597c43b23e5d5ebd64fca94b652083c49449bf64a2c2d75f9a933a206ccb75b718e74cebeff78b580aa7b3ac55f9da905b0df29c9aaab516d596b67f57c6b60bc10c2a0dbac27744c6c997600015183d843661fbfd058d685348153652c98d9ec35fcc20f303daa3101123901a3de24063c81ce5bb50727e9b66372583d43283d4f0429863bb4c2f61c0be5ea82457426f80e6fe99776a37b4201f32813ff791800018f79be49dc75d3a9dd0000000000000000000000000000000000000000000000000000000003e803e70000000000000000000000000000000000000000000000000000000000000001";

        assertEq(publicInputs, expected, "Public inputs encoding mismatch for zk-dex batch 2");
    }

    /// @notice Test encoding with balance_diffs (variable-size fields).
    ///         Verifies encoding length when balance_diffs are present.
    function test_encoding_with_balance_diffs() public pure {
        bytes32 initialStateHash   = bytes32(uint256(1));
        bytes32 finalStateHash     = bytes32(uint256(2));
        bytes32 withdrawalsMerkle  = bytes32(uint256(3));
        bytes32 privTxRollingHash  = bytes32(uint256(4));
        bytes32 blobVersionedHash  = bytes32(uint256(5));
        bytes32 lastBlockHash      = bytes32(uint256(6));
        uint256 chainId            = 7;
        uint256 nonPrivilegedTxs   = 8;

        // Simulate one balance_diff with one asset_diff
        uint256 bdChainId = 100;
        uint256 bdValue   = 200;
        address tokenL1   = address(0xAAAA);
        address tokenL2   = address(0xBBBB);
        address destTokenL2 = address(0xCCCC);
        uint256 adValue   = 300;

        bytes memory publicInputs = abi.encodePacked(
            initialStateHash,
            finalStateHash,
            withdrawalsMerkle,
            privTxRollingHash,
            blobVersionedHash,
            lastBlockHash,
            bytes32(chainId),
            bytes32(nonPrivilegedTxs),
            // balance_diff fields
            bytes32(bdChainId),
            bytes32(bdValue),
            // asset_diff fields (address = 20 bytes each)
            tokenL1,
            tokenL2,
            destTokenL2,
            bytes32(adValue)
        );

        // 8 * 32 (fixed) + 2 * 32 (bd header) + 3 * 20 (addresses) + 32 (value)
        // = 256 + 64 + 60 + 32 = 412
        assertEq(publicInputs.length, 412, "Encoding with balance_diff should be 412 bytes");
    }

    /// @notice Verify SHA-256 hash of public inputs matches fixture.
    ///         This is the value used for RISC0 verification (sha256(publicInputs)).
    function test_sha256_public_values_zk_dex_batch_2() public pure {
        bytes memory publicInputs = hex"48f7b11fc87cbe873361a3ff5b40c91dc24cf42f2597c43b23e5d5ebd64fca94b652083c49449bf64a2c2d75f9a933a206ccb75b718e74cebeff78b580aa7b3ac55f9da905b0df29c9aaab516d596b67f57c6b60bc10c2a0dbac27744c6c997600015183d843661fbfd058d685348153652c98d9ec35fcc20f303daa3101123901a3de24063c81ce5bb50727e9b66372583d43283d4f0429863bb4c2f61c0be5ea82457426f80e6fe99776a37b4201f32813ff791800018f79be49dc75d3a9dd0000000000000000000000000000000000000000000000000000000003e803e70000000000000000000000000000000000000000000000000000000000000001";

        bytes32 expectedHash = 0x11f0ad405278dfd9a8bb883a7ba632031b4b0af97c5d2603f6048ec37c984811;
        bytes32 actualHash = sha256(publicInputs);

        assertEq(actualHash, expectedHash, "SHA-256 of public inputs should match fixture");
    }

    /// @notice Verify encoding length is exactly 256 bytes for 8 fixed fields
    ///         with no variable-size data.
    function test_encoding_length_fixed_only() public pure {
        bytes memory publicInputs = abi.encodePacked(
            bytes32(0), bytes32(0), bytes32(0), bytes32(0),
            bytes32(0), bytes32(0), bytes32(0), bytes32(0)
        );
        assertEq(publicInputs.length, 256, "8 fixed fields = 256 bytes");
    }

    /// @notice Test encoding with L2 in message rolling hashes (variable-size).
    function test_encoding_with_l2_rolling_hashes() public pure {
        bytes memory fixedPart = abi.encodePacked(
            bytes32(uint256(1)), bytes32(uint256(2)), bytes32(uint256(3)), bytes32(uint256(4)),
            bytes32(uint256(5)), bytes32(uint256(6)), bytes32(uint256(7)), bytes32(uint256(8))
        );

        // One L2 rolling hash: chainId (uint256) + rollingHash (bytes32) = 64 bytes
        uint256 l2ChainId = 12345;
        bytes32 l2RollingHash = bytes32(uint256(99));

        bytes memory full = abi.encodePacked(
            fixedPart,
            bytes32(l2ChainId),
            l2RollingHash
        );

        // 256 (fixed) + 32 + 32 = 320
        assertEq(full.length, 320, "Fixed + 1 L2 rolling hash = 320 bytes");
    }
}
