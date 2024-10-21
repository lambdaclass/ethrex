// SPDX-License-Identifier: MIT
pragma solidity 0.8.27;

import "./interfaces/IOnChainProposer.sol";

/// @title OnChainProposer contract.
/// @author LambdaClass
contract OnChainProposer is IOnChainProposer {
    /// @inheritdoc IOnChainProposer
    function commit(bytes32 currentBlockCommitment) external override {
        emit BlockCommitted(currentBlockCommitment);
    }

    /// @inheritdoc IOnChainProposer
    function verify(bytes calldata blockProof) external override {
        bytes32 blockHash = keccak256(abi.encode(blockProof));
        emit BlockVerified(blockHash);
    }
}
