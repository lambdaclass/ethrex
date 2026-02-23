// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

import "./L1Anchor.sol";

/// @title L2Bridge — Unified L1 message processing and withdrawal bridge for Native Rollups PoC.
///
/// Deployed at 0x000000000000000000000000000000000000fffd (L2 predeploy).
/// Preminted with a large ETH balance in L2 genesis to cover all future L1
/// messages.
///
/// L1 Messages: the relayer calls processL1Message() for each pending L1
/// message, providing a Merkle proof against the L1 messages root anchored
/// by the EXECUTE precompile in the L1Anchor predeploy. The state root check
/// at the end of EXECUTE implicitly guarantees correct message processing.
///
/// Withdrawals: users call withdraw() to lock ETH, write the withdrawal hash
/// to `sentMessages` storage, and emit WithdrawalInitiated. The L1 contract
/// verifies withdrawals via MPT storage proofs against the L2 state root —
/// no custom Merkle tree in the EXECUTE precompile.
///
/// Storage layout:
///   Slot 0: relayer (address)
///   Slot 1: l1MessageNonce (uint256)
///   Slot 2: withdrawalNonce (uint256)
///   Slot 3: sentMessages (mapping(bytes32 => bool))
contract L2Bridge {
    address public relayer;
    uint256 public l1MessageNonce;
    uint256 public withdrawalNonce;
    mapping(bytes32 => bool) public sentMessages;

    /// @dev L1Anchor predeploy address (one above L2Bridge).
    address constant L1_ANCHOR_ADDRESS = 0x000000000000000000000000000000000000FFFE;

    event L1MessageProcessed(
        address indexed from,
        address indexed to,
        uint256 value,
        uint256 gasLimit,
        bytes32 dataHash,
        uint256 indexed nonce
    );

    event WithdrawalInitiated(
        address indexed from,
        address indexed receiver,
        uint256 amount,
        uint256 indexed messageId
    );

    /// @notice Process a single L1 message: verify Merkle proof, execute subcall, emit event.
    /// @dev If the subcall fails, the nonce is still incremented and the event
    ///      is still emitted so that L1/L2 nonces stay in sync. Assets stay in the
    ///      bridge. User recovery mechanism is TBD.
    /// @param from        Original L1 sender (msg.sender on L1).
    /// @param to          Target address on L2.
    /// @param value       Amount of ETH to send.
    /// @param gasLimit    Maximum gas for the L2 subcall.
    /// @param data        Calldata to execute on L2 (can be empty for simple ETH transfers).
    /// @param nonce       Nonce from the L1 message (must match current l1MessageNonce).
    /// @param merkleProof Merkle proof against the L1 messages root anchored in L1Anchor.
    function processL1Message(
        address from,
        address to,
        uint256 value,
        uint256 gasLimit,
        bytes calldata data,
        uint256 nonce,
        bytes32[] calldata merkleProof
    ) external {
        require(msg.sender == relayer, "L2Bridge: not relayer");
        require(nonce == l1MessageNonce, "L2Bridge: nonce mismatch");

        uint256 currentNonce = l1MessageNonce;
        l1MessageNonce = currentNonce + 1;

        // Compute message hash (same 168-byte preimage as L1's _recordL1Message)
        bytes32 messageHash = keccak256(abi.encodePacked(from, to, value, gasLimit, keccak256(data), currentNonce));

        // Verify Merkle proof against the L1 messages root anchored by EXECUTE precompile
        bytes32 root = L1Anchor(L1_ANCHOR_ADDRESS).l1MessagesRoot();
        require(_verifyMerkleProof(merkleProof, root, messageHash), "L2Bridge: invalid proof");

        // Execute the L2 subcall. Don't revert on failure — nonce stays in sync, assets stay in bridge.
        to.call{value: value, gas: gasLimit}(data);

        emit L1MessageProcessed(from, to, value, gasLimit, keccak256(data), currentNonce);
    }

    /// @notice Initiate a withdrawal by sending ETH with the L1 receiver address.
    /// @dev The ETH stays locked in the bridge contract (not burned). On L1,
    ///      claimWithdrawal releases the corresponding ETH from NativeRollup.
    /// @param _receiver Address on L1 that will receive the withdrawn ETH.
    function withdraw(address _receiver) external payable {
        require(msg.value > 0, "Withdrawal amount must be positive");
        require(_receiver != address(0), "Invalid receiver");

        uint256 msgId = withdrawalNonce;
        withdrawalNonce = msgId + 1;

        bytes32 withdrawalHash = keccak256(abi.encodePacked(msg.sender, _receiver, msg.value, msgId));
        sentMessages[withdrawalHash] = true;

        emit WithdrawalInitiated(msg.sender, _receiver, msg.value, msgId);
    }

    /// @dev Verify a Merkle proof using commutative Keccak256 hashing.
    /// Compatible with OpenZeppelin's MerkleProof.verify().
    function _verifyMerkleProof(
        bytes32[] calldata proof,
        bytes32 root,
        bytes32 leaf
    ) internal pure returns (bool) {
        bytes32 computedHash = leaf;
        for (uint256 i = 0; i < proof.length; i++) {
            computedHash = _hashPair(computedHash, proof[i]);
        }
        return computedHash == root;
    }

    /// @dev Commutative hash pair: H(a, b) == H(b, a).
    function _hashPair(bytes32 a, bytes32 b) private pure returns (bytes32) {
        if (a < b) {
            return keccak256(abi.encodePacked(a, b));
        } else {
            return keccak256(abi.encodePacked(b, a));
        }
    }
}
