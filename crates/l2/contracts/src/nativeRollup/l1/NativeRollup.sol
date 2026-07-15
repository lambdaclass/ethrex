// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

import "./MPTProof.sol";

/// @title NativeRollup — L2 state manager using the EXECUTE precompile.
///
/// Aligned with the l2beat native rollups spec.
/// The `advance()` method forwards SSZ-encoded `StatelessInput` to the
/// EXECUTE precompile, which calls `verify_stateless_new_payload` and
/// returns SSZ-encoded `StatelessValidationResult`.
///
/// Storage layout:
///   Slot 0:  blockHash (bytes32)
///   Slot 1:  stateRoot (bytes32)
///   Slot 2:  blockNumber (uint256)
///   Slot 3:  l2GasLimit (uint256)
///   Slot 4:  chainId (uint256)
///   Slot 5:  pendingL1Messages (bytes32[])
///   Slot 6:  l1MessageIndex (uint256)
///   Slot 7:  totalDeposited (uint256)
///   Slot 8:  totalClaimed (uint256)
///   Slot 9:  stateRootHistory (mapping(uint256 => bytes32))
///   Slot 10: claimedWithdrawals (mapping(bytes32 => bool))
///   Slot 11: stateRootTimestamps (mapping(uint256 => uint256))
///   Slot 12: _locked (bool)
///   Slot 13: lastFetchedL1Block (uint256)
///   Slot 14: advancer (address)
///   Slot 15: pendingAdvancer (address)
///   (CHAIN_ID and FINALITY_DELAY are `immutable` — stored in code, not storage)

contract NativeRollup {
    // ===== L2 chain state (spec-aligned) =====
    bytes32 public blockHash;
    bytes32 public stateRoot;
    uint256 public blockNumber;
    uint256 public l2GasLimit;
    uint256 public chainId;

    // ===== L1→L2 messaging =====
    bytes32[] public pendingL1Messages;
    uint256 public l1MessageIndex;

    // ===== Escrow solvency (I7) =====
    /// Total ETH deposited via value-bearing L1 messages. Claims may never
    /// exceed this, so the L1 escrow can only ever pay out what was actually
    /// bridged in — an over-withdrawal on L2 (whose bridge is preminted) cannot
    /// drain L1 funds it never received.
    uint256 public totalDeposited;
    /// Total ETH paid out by `claimWithdrawal`.
    uint256 public totalClaimed;

    // ===== L2→L1 messaging (state proof-based) =====
    mapping(uint256 => bytes32) public stateRootHistory;
    mapping(bytes32 => bool) public claimedWithdrawals;
    mapping(uint256 => uint256) public stateRootTimestamps;

    // ===== Reentrancy guard =====
    bool private _locked;

    // ===== L1 watcher cursor =====
    /// Deploy block. Watcher seeds its cursor from this on first poll.
    uint256 public lastFetchedL1Block;

    // ===== Access control =====
    address public advancer;
    /// Nominated next advancer, pending acceptance (two-step handoff). Set by
    /// `setAdvancer`, cleared once the nominee calls `acceptAdvancer`.
    address public pendingAdvancer;

    // ===== Immutables =====
    uint256 public immutable CHAIN_ID;
    uint256 public immutable FINALITY_DELAY;

    address constant EXECUTE_PRECOMPILE = address(0x0101);
    address constant L2_BRIDGE_ADDRESS = address(0x000000000000000000000000000000000000FfFD);
    // Storage slot of `sentMessages` in L2Bridge.sol. Must match that layout —
    // reordering L2Bridge storage silently breaks withdrawal proofs here.
    uint256 constant SENT_MESSAGES_SLOT = 3;

    // Fixed gas the L2 block producer reserves around each L1 message, on top of
    // the 63/64 call-gas rule, when it builds the relayer tx (must match
    // `build_relayer_transactions` in block_producer.rs). A message whose
    // gasLimit leaves no room for this within one L2 block would be un-includable,
    // so `sendL1Message` rejects it up front (see the cap there).
    uint256 constant RELAYER_GAS_BODY_ALLOWANCE = 300_000;

    // SSZ `StatelessValidationResult`: root(32) + successful_validation(1, @32) +
    // chain_config(variable → 4-byte offset @33). chain_config's first field is
    // chain_id (uint64 LE), read at the dereferenced offset.
    uint256 constant RESULT_FIXED_LEN = 37; // root(32) + bool(1) + chain_config offset(4)
    uint256 constant RESULT_SUCCESS_OFFSET = 32;
    uint256 constant RESULT_CHAIN_CONFIG_OFFSET_POS = 33;

    // Byte offsets inside the SSZ `ExecutionPayload` fixed prefix.
    uint256 constant EP_PARENT_HASH_OFFSET = 0;
    uint256 constant EP_STATE_ROOT_OFFSET = 52;
    uint256 constant EP_BLOCK_NUMBER_OFFSET = 404;
    uint256 constant EP_GAS_LIMIT_OFFSET = 412;
    uint256 constant EP_BLOCK_HASH_OFFSET = 472;
    // EIP-7843 slot_number is a trailing fixed u64 in the payload, right after the
    // block_access_list offset slot (528..532). All reads above are unaffected.
    uint256 constant EP_SLOT_NUMBER_OFFSET = 532;
    uint256 constant EP_FIXED_PREFIX_LEN = 540; // 528 fixed fields + block_access_list offset(4) + slot_number(8)

    // ===== Events =====
    event StateAdvanced(uint256 indexed newBlockNumber, bytes32 indexed newStateRoot);
    event L1MessageRecorded(address indexed sender, address indexed to, uint256 value, uint256 gasLimit, bytes data, uint256 indexed nonce);
    event WithdrawalClaimed(address indexed receiver, uint256 amount, uint256 blockNumber, uint256 messageId);
    event AdvancerTransferStarted(address indexed currentAdvancer, address indexed pendingAdvancer);
    event AdvancerChanged(address indexed oldAdvancer, address indexed newAdvancer);

    modifier nonReentrant() {
        require(!_locked, "ReentrancyGuard: reentrant call");
        _locked = true;
        _;
        _locked = false;
    }

    modifier onlyAdvancer() {
        require(msg.sender == advancer, "NativeRollup: not advancer");
        _;
    }

    constructor(
        bytes32 _initialStateRoot,
        bytes32 _initialBlockHash,
        uint256 _blockGasLimit,
        uint256 _chainId,
        address _advancer,
        uint256 _finalityDelay
    ) {
        require(_advancer != address(0), "NativeRollup: advancer is zero");
        stateRoot = _initialStateRoot;
        blockHash = _initialBlockHash;
        blockNumber = 0;
        l2GasLimit = _blockGasLimit;
        chainId = _chainId;
        CHAIN_ID = _chainId;
        // Configurable exit window (was hardcoded 0). A local demo passes 0 for
        // instant finality; production should set a real reorg-safe delay.
        FINALITY_DELAY = _finalityDelay;
        advancer = _advancer;
        lastFetchedL1Block = block.number;
    }

    /// @notice Begin rotating the authorized advancer (step 1 of 2). Callable
    /// only by the current advancer. The nominee does not gain control until it
    /// calls `acceptAdvancer`, so a typo or wrong address cannot brick this
    /// (immutable, non-upgradeable) contract.
    /// @param _newAdvancer Address nominated to become the next advancer.
    function setAdvancer(address _newAdvancer) external onlyAdvancer {
        require(_newAdvancer != address(0), "NativeRollup: advancer is zero");
        pendingAdvancer = _newAdvancer;
        emit AdvancerTransferStarted(advancer, _newAdvancer);
    }

    /// @notice Complete the advancer rotation (step 2 of 2). Callable only by
    /// the address nominated in `setAdvancer`, proving it can transact before it
    /// gains control.
    function acceptAdvancer() external {
        require(msg.sender == pendingAdvancer, "NativeRollup: not pending advancer");
        emit AdvancerChanged(advancer, pendingAdvancer);
        advancer = pendingAdvancer;
        pendingAdvancer = address(0);
    }

    // ===== L1 Messaging =====

    /// @notice Enqueue an L1→L2 message (optionally value-bearing) for the L2 to
    /// consume. Permissionless. Burns `_gasLimit` gas on L1 to price the L2
    /// execution the message will trigger.
    /// @param _to L2 recipient of the message.
    /// @param _gasLimit Gas the message may consume on L2. Capped below
    ///        `l2GasLimit` so the producer's relayer tx (which adds the 63/64
    ///        call-gas overhead plus `RELAYER_GAS_BODY_ALLOWANCE`) still fits in
    ///        one L2 block.
    /// @param _data Calldata delivered to `_to` on L2.
    function sendL1Message(address _to, uint256 _gasLimit, bytes calldata _data) external payable {
        require(_gasLimit > 0, "NativeRollup: gasLimit must be > 0");
        // Reject messages that can never fit in a single L2 block: the producer
        // builds a relayer tx of `_gasLimit * 64/63 + RELAYER_GAS_BODY_ALLOWANCE`
        // (block_producer.rs::build_relayer_transactions). Requiring that to be
        // <= l2GasLimit means every accepted message is includable, so a valid
        // deposit can never get permanently stuck.
        require(
            _gasLimit <= ((l2GasLimit - RELAYER_GAS_BODY_ALLOWANCE) * 63) / 64,
            "NativeRollup: gasLimit exceeds includable limit"
        );
        _burnGas(_gasLimit);
        // I7: account bridged-in ETH so claims can never exceed deposits.
        totalDeposited += msg.value;
        _recordL1Message(msg.sender, _to, msg.value, _gasLimit, _data);
    }

    /// @notice Reject plain ETH transfers. All value must enter via
    /// `sendL1Message` so escrow accounting (`totalDeposited`) stays consistent;
    /// ETH sent directly would be untracked and unrecoverable.
    receive() external payable {
        revert("NativeRollup: use sendL1Message to deposit");
    }

    function _recordL1Message(
        address _from,
        address _to,
        uint256 _value,
        uint256 _gasLimit,
        bytes memory _data
    ) internal {
        uint256 nonce = pendingL1Messages.length;
        bytes32 dataHash = keccak256(_data);
        bytes32 messageHash = keccak256(abi.encodePacked(_from, _to, _value, _gasLimit, dataHash, nonce));
        pendingL1Messages.push(messageHash);
        emit L1MessageRecorded(_from, _to, _value, _gasLimit, _data, nonce);
    }

    function _burnGas(uint256 amount) private view {
        uint256 startingGas = gasleft();
        while (startingGas - gasleft() < amount) {}
    }

    // ===== Block Advancement =====

    /// @notice Advance the L2 state by one block.
    /// @param _l1MessagesCount Number of L1 messages consumed in this block.
    /// @param _sszStatelessInput SSZ-encoded `StatelessInput` for the block.
    /// @dev The L1 messages Merkle root in `parent_beacon_block_root` is
    ///      recomputed over `pendingL1Messages[l1MessageIndex .. +count]`
    ///      and must match — same shape as OCP's
    ///      `processedPrivilegedTransactionsRollingHash` check.
    function advance(
        uint16 _l1MessagesCount,
        bytes calldata _sszStatelessInput
    ) external onlyAdvancer {
        uint256 startIdx = l1MessageIndex;
        require(startIdx + _l1MessagesCount <= pendingL1Messages.length, "NativeRollup: not enough L1 messages");

        _runExecutePrecompile(_sszStatelessInput);

        (uint256 newBlockNumber, bytes32 newBlockHash, bytes32 newStateRoot) =
            _checkAndDecodeProvenFields(_sszStatelessInput, startIdx, _l1MessagesCount);

        l1MessageIndex = startIdx + _l1MessagesCount;
        blockHash = newBlockHash;
        stateRoot = newStateRoot;
        blockNumber = newBlockNumber;
        stateRootHistory[newBlockNumber] = newStateRoot;
        stateRootTimestamps[newBlockNumber] = block.timestamp;

        emit StateAdvanced(newBlockNumber, newStateRoot);
    }

    function _runExecutePrecompile(bytes calldata sszInput) internal view {
        (bool success, bytes memory result) = EXECUTE_PRECOMPILE.staticcall(sszInput);
        require(success, "NativeRollup: EXECUTE precompile failed");

        require(result.length >= RESULT_FIXED_LEN, "NativeRollup: invalid result length");
        require(uint8(result[RESULT_SUCCESS_OFFSET]) == 1, "NativeRollup: L2 validation failed");

        // chain_config is variable-size (contains active_fork); it is offset-encoded.
        // chain_id is its first field.
        uint256 ccOffset = _decodeSszUint32LE(result, RESULT_CHAIN_CONFIG_OFFSET_POS);
        uint64 provenChainId = _decodeSszUint64LE(result, ccOffset);
        require(provenChainId == chainId, "NativeRollup: chain_id mismatch");
    }

    function _checkAndDecodeProvenFields(
        bytes calldata sszInput,
        uint256 startIdx,
        uint16 l1MessagesCount
    )
        internal
        view
        returns (uint256 newBlockNumber, bytes32 newBlockHash, bytes32 newStateRoot)
    {
        (
            uint64 provenBlockNumber,
            bytes32 provenParentHash,
            bytes32 provenBlockHash,
            bytes32 provenStateRoot,
            uint64 provenGasLimit,
            bytes32 provenL1MessagesRoot
        ) = _decodeProvenPayloadFields(sszInput);

        newBlockNumber = blockNumber + 1;
        require(uint256(provenBlockNumber) == newBlockNumber, "NativeRollup: block_number mismatch");
        require(provenParentHash == blockHash, "NativeRollup: parent_hash mismatch");
        require(uint256(provenGasLimit) == l2GasLimit, "NativeRollup: gas_limit mismatch");

        require(
            provenL1MessagesRoot == _computeL1MessagesRoot(startIdx, l1MessagesCount),
            "NativeRollup: L1 messages root mismatch"
        );

        newBlockHash = provenBlockHash;
        newStateRoot = provenStateRoot;
    }

    /// @notice Merkle root over the next `number` unconsumed L1 messages.
    ///         Mirrors `CommonBridge.getPendingTransactionsVersionedHash`.
    function getPendingL1MessagesRoot(uint16 number) public view returns (bytes32) {
        require(
            uint256(number) <= pendingL1Messages.length - l1MessageIndex,
            "NativeRollup: number exceeds pending L1 messages"
        );
        return _computeL1MessagesRoot(l1MessageIndex, number);
    }

    /// @dev NPR fixed prefix: ep_off(4) | vh_off(4) | parent_beacon_block_root(32) | er_off(4).
    function _decodeProvenPayloadFields(bytes calldata sszInput)
        internal
        pure
        returns (
            uint64 blockNumber_,
            bytes32 parentHash,
            bytes32 blockHash_,
            bytes32 stateRoot_,
            uint64 gasLimit_,
            bytes32 parentBeaconBlockRoot
        )
    {
        require(sszInput.length >= 20, "SSZ: input too short");
        uint256 nprAbs = _readU32LECalldata(sszInput, 0);
        // 44 = NPR fixed prefix: 3 var-field offsets (12) + parent_beacon_block_root (32).
        require(sszInput.length >= nprAbs + 44, "SSZ: NPR offset out of range");
        parentBeaconBlockRoot = _readBytes32Calldata(sszInput, nprAbs + 8);
        uint256 epAbs = nprAbs + _readU32LECalldata(sszInput, nprAbs);
        require(
            sszInput.length >= epAbs + EP_FIXED_PREFIX_LEN,
            "SSZ: EP offset out of range"
        );

        parentHash = _readBytes32Calldata(sszInput, epAbs + EP_PARENT_HASH_OFFSET);
        stateRoot_ = _readBytes32Calldata(sszInput, epAbs + EP_STATE_ROOT_OFFSET);
        blockNumber_ = _readU64LECalldata(sszInput, epAbs + EP_BLOCK_NUMBER_OFFSET);
        gasLimit_ = _readU64LECalldata(sszInput, epAbs + EP_GAS_LIMIT_OFFSET);
        blockHash_ = _readBytes32Calldata(sszInput, epAbs + EP_BLOCK_HASH_OFFSET);
    }

    /// @dev Must match `compute_merkle_root` in `crates/l2/common/src/merkle_tree.rs`
    ///      — diverging silently breaks L2Bridge proofs. Single-leaf trees
    ///      are NOT padded (lambdaworks treats `len==1` as already power-of-two).
    function _computeL1MessagesRoot(uint256 startIdx, uint16 count)
        internal
        view
        returns (bytes32)
    {
        if (count == 0) return bytes32(0);

        uint256 len = _nextPowerOfTwo(count);
        bytes32[] memory layer = new bytes32[](len);
        bytes32 last;
        for (uint256 i = 0; i < count; i++) {
            last = pendingL1Messages[startIdx + i];
            layer[i] = last;
        }
        for (uint256 i = count; i < len; i++) {
            layer[i] = last;
        }

        while (len > 1) {
            uint256 newLen = len / 2;
            for (uint256 i = 0; i < newLen; i++) {
                bytes32 a = layer[2 * i];
                bytes32 b = layer[2 * i + 1];
                layer[i] = a < b
                    ? keccak256(abi.encodePacked(a, b))
                    : keccak256(abi.encodePacked(b, a));
            }
            len = newLen;
        }
        return layer[0];
    }

    function _nextPowerOfTwo(uint256 n) internal pure returns (uint256) {
        if (n <= 1) return 1;
        uint256 p = 1;
        while (p < n) p <<= 1;
        return p;
    }

    // ===== Withdrawal Claiming (MPT proof-based) =====

    /// @notice Claim an L2→L1 withdrawal by proving its inclusion in a
    /// finalized L2 state root. Permissionless; pays `_amount` to `_receiver`
    /// from the L1 escrow, bounded by escrow solvency (I7).
    /// @param _from L2 sender that initiated the withdrawal.
    /// @param _receiver L1 recipient of the withdrawn ETH.
    /// @param _amount Amount to withdraw, in wei.
    /// @param _messageId L2Bridge message id of the withdrawal.
    /// @param _atBlockNumber L2 block whose committed state root proves the withdrawal.
    /// @param _accountProof MPT proof of the L2Bridge account against the state root.
    /// @param _storageProof MPT proof of the withdrawal slot against the storage root.
    function claimWithdrawal(
        address _from,
        address payable _receiver,
        uint256 _amount,
        uint256 _messageId,
        uint256 _atBlockNumber,
        bytes[] calldata _accountProof,
        bytes[] calldata _storageProof
    ) external nonReentrant {
        bytes32 historicStateRoot = stateRootHistory[_atBlockNumber];
        require(historicStateRoot != bytes32(0), "NativeRollup: state root not found");

        uint256 stateRootTime = stateRootTimestamps[_atBlockNumber];
        require(block.timestamp >= stateRootTime + FINALITY_DELAY, "NativeRollup: withdrawal not yet final");

        bytes32 withdrawalHash = keccak256(abi.encodePacked(_from, _receiver, _amount, _messageId));
        require(!claimedWithdrawals[withdrawalHash], "NativeRollup: withdrawal already claimed");

        _verifyWithdrawalProof(historicStateRoot, withdrawalHash, _accountProof, _storageProof);

        // I7: escrow solvency — never pay out more than was deposited. Checked
        // and updated before the transfer (checks-effects-interactions).
        require(totalClaimed + _amount <= totalDeposited, "NativeRollup: escrow insolvent");
        totalClaimed += _amount;

        claimedWithdrawals[withdrawalHash] = true;

        (bool sent, ) = _receiver.call{value: _amount}("");
        require(sent, "NativeRollup: ETH transfer failed");

        emit WithdrawalClaimed(_receiver, _amount, _atBlockNumber, _messageId);
    }

    function _verifyWithdrawalProof(
        bytes32 _stateRoot,
        bytes32 _withdrawalHash,
        bytes[] calldata _accountProof,
        bytes[] calldata _storageProof
    ) internal pure {
        bytes32 l2BridgeAddressHash = keccak256(abi.encodePacked(L2_BRIDGE_ADDRESS));
        bytes memory accountRLP = MPTProof.verifyMptProof(
            _stateRoot,
            MPTProof.toNibbles(abi.encodePacked(l2BridgeAddressHash)),
            _accountProof
        );
        bytes32 storageRoot = MPTProof.decodeAccountStorageRoot(accountRLP);

        bytes32 slot = keccak256(abi.encode(_withdrawalHash, SENT_MESSAGES_SLOT));
        bytes32 slotHash = keccak256(abi.encodePacked(slot));
        bytes memory valueRLP = MPTProof.verifyMptProof(
            storageRoot,
            MPTProof.toNibbles(abi.encodePacked(slotHash)),
            _storageProof
        );
        uint256 value = MPTProof.decodeRlpUint(valueRLP);
        require(value == 1, "NativeRollup: withdrawal not found in L2Bridge storage");
    }

    /// @dev Decode an SSZ `uint32` (4 little-endian bytes) at `offset` into `data` (memory).
    function _decodeSszUint32LE(bytes memory data, uint256 offset) internal pure returns (uint256) {
        require(data.length >= offset + 4, "SSZ: u32 out of bounds");
        return uint256(uint8(data[offset]))
            | (uint256(uint8(data[offset + 1])) << 8)
            | (uint256(uint8(data[offset + 2])) << 16)
            | (uint256(uint8(data[offset + 3])) << 24);
    }

    /// @dev Decode an SSZ `uint64` (8 little-endian bytes) at `offset` into `data` (memory).
    function _decodeSszUint64LE(bytes memory data, uint256 offset) internal pure returns (uint64) {
        require(data.length >= offset + 8, "SSZ: out of bounds");
        uint64 value = 0;
        for (uint256 i = 0; i < 8; i++) {
            value |= uint64(uint8(data[offset + i])) << uint64(8 * i);
        }
        return value;
    }

    /// @dev Decode an SSZ `uint32` (4 little-endian bytes) at `offset` into `data` (calldata).
    function _readU32LECalldata(bytes calldata data, uint256 offset) internal pure returns (uint256) {
        require(data.length >= offset + 4, "SSZ: u32 out of bounds");
        return uint256(uint8(data[offset]))
            | (uint256(uint8(data[offset + 1])) << 8)
            | (uint256(uint8(data[offset + 2])) << 16)
            | (uint256(uint8(data[offset + 3])) << 24);
    }

    /// @dev Decode an SSZ `uint64` (8 little-endian bytes) at `offset` into `data` (calldata).
    function _readU64LECalldata(bytes calldata data, uint256 offset) internal pure returns (uint64 v) {
        require(data.length >= offset + 8, "SSZ: u64 out of bounds");
        for (uint256 i = 0; i < 8; i++) {
            v |= uint64(uint8(data[offset + i])) << uint64(8 * i);
        }
    }

    /// @dev Read 32 contiguous bytes at `offset` from calldata as a `bytes32`.
    function _readBytes32Calldata(bytes calldata data, uint256 offset) internal pure returns (bytes32 out) {
        require(data.length >= offset + 32, "SSZ: bytes32 out of bounds");
        // solhint-disable-next-line no-inline-assembly
        assembly {
            out := calldataload(add(data.offset, offset))
        }
    }
}
