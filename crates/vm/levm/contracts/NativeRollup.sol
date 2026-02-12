// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

/// @title NativeRollup — PoC L2 state manager using the EXECUTE precompile (EIP-8079).
///
/// Maintains the current L2 state root and block number. The `advance` method
/// calls the EXECUTE precompile at 0x0101, which re-executes the L2 block and
/// verifies the state transition. On success, the contract updates its state.
contract NativeRollup {
    bytes32 public stateRoot;
    uint256 public blockNumber;

    address constant EXECUTE_PRECOMPILE = address(0x0101);

    event StateAdvanced(uint256 indexed newBlockNumber, bytes32 newStateRoot);

    constructor(bytes32 _initialStateRoot) {
        stateRoot = _initialStateRoot;
    }

    /// @notice Advance the L2 by one block.
    /// @param _newStateRoot Expected post-state root (verified by the precompile).
    /// @param _newBlockNumber The L2 block number being applied.
    /// @param _precompileInput Full JSON-serialized ExecutePrecompileInput for 0x0101.
    function advance(
        bytes32 _newStateRoot,
        uint256 _newBlockNumber,
        bytes calldata _precompileInput
    ) external {
        (bool success, bytes memory result) = EXECUTE_PRECOMPILE.call(
            _precompileInput
        );
        require(
            success && result.length == 1 && uint8(result[0]) == 0x01,
            "EXECUTE precompile verification failed"
        );

        stateRoot = _newStateRoot;
        blockNumber = _newBlockNumber;

        emit StateAdvanced(_newBlockNumber, _newStateRoot);
    }
}
