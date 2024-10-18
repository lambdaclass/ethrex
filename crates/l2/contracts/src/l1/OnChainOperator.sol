// SPDX-License-Identifier: MIT
pragma solidity 0.8.27;

import "./interfaces/IOnChainOperator.sol";

/// @title OnChainOperator contract.
/// @author LambdaClass
contract OnChainOperator is IOnChainOperator {
    /// @inheritdoc IOnChainOperator
    function commit(bytes32 currentBlockCommitment) external override {
        emit BlockCommitted(currentBlockCommitment);
    }

    /// @inheritdoc IOnChainOperator
    function verify(bytes calldata blockProof) external override {
        bytes32 blockHash = keccak256(abi.encode(blockProof));
        emit BlockVerified(blockHash);
    }
}
