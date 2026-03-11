// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title L1Anchor — L2 predeploy for anchoring L1 data on L2.
///
/// Deployed at 0x000000000000000000000000000000000000fffe (L2 predeploy).
/// The EXECUTE precompile writes the L1 messages Merkle root directly to
/// storage slot 0 before executing regular transactions (system transaction).
/// L2 contracts (e.g., L2Bridge) read the anchored root to verify L1 message
/// inclusion via Merkle proofs.
///
/// Storage layout:
///   Slot 0: l1MessagesRoot (bytes32)
contract L1Anchor {
    bytes32 public l1MessagesRoot;
}
