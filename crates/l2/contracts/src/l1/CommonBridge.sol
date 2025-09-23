// SPDX-License-Identifier: MIT
pragma solidity =0.8.29;

import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/Ownable2StepUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/utils/PausableUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/utils/ReentrancyGuardUpgradeable.sol";
import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import "@openzeppelin/contracts/proxy/transparent/TransparentUpgradeableProxy.sol";
import {MerkleProof} from "@openzeppelin/contracts/utils/cryptography/MerkleProof.sol";

import "./interfaces/ICommonBridge.sol";
import "./interfaces/IOnChainProposer.sol";
import "../l2/interfaces/ICommonBridgeL2.sol";

/// @title CommonBridge contract.
/// @author LambdaClass
contract CommonBridge is
    ICommonBridge,
    Initializable,
    UUPSUpgradeable,
    Ownable2StepUpgradeable,
    ReentrancyGuardUpgradeable,
    PausableUpgradeable
{
    using SafeERC20 for IERC20;

    /// @notice Mapping of unclaimed withdrawals. A withdrawal is claimed if
    /// there is a non-zero value in the mapping (a merkle root) for the hash
    /// of the L2 transaction that requested the withdrawal.
    /// @dev The key is the hash of the L2 transaction that requested the
    /// withdrawal.
    /// @dev Deprecated.
    mapping(bytes32 => bool) public claimedWithdrawals;

    /// @notice Mapping of merkle roots to the L2 withdrawal transaction logs.
    /// @dev The key is the L2 batch number where the logs were emitted.
    /// @dev The value is the merkle root of the logs.
    /// @dev If there exist a merkle root for a given batch number it means
    /// that the logs were published on L1, and that that batch was committed.
    mapping(uint256 => bytes32) public batchWithdrawalLogsMerkleRoots;

    /// @notice Array of hashed pending privileged transactions
    bytes32[] public pendingTxHashes;

    address public ON_CHAIN_PROPOSER;

    /// @notice Block in which the CommonBridge was initialized.
    /// @dev Used by the L1Watcher to fetch logs starting from this block.
    uint256 public lastFetchedL1Block;

    /// @notice Global privileged transaction identifier, it is incremented each time a new privileged transaction is made.
    /// @dev It is used as the nonce of the mint transaction created by the L1Watcher.
    uint256 public transactionId;

    /// @notice Address of the bridge on the L2
    /// @dev It's used to validate withdrawals
    address public constant L2_BRIDGE_ADDRESS = address(0xffff);

    /// @notice How much of each L1 token was deposited to each L2 token.
    /// @dev Stored as L1 -> L2 -> amount
    /// @dev Prevents L2 tokens from faking their L1 address and stealing tokens
    /// @dev The token can take the value {ETH_TOKEN} to represent ETH
    mapping(address => mapping(address => uint256)) public deposits;

    /// @notice Token address used to represent ETH
    address public constant ETH_TOKEN =
        0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE;

    /// @notice Owner of the L2 system contract proxies
    address public constant L2_PROXY_ADMIN =  0x000000000000000000000000000000000000f000;

    /// @notice Mapping of unclaimed withdrawals. A withdrawal is claimed if
    /// there is a non-zero value in the mapping for the message id
    /// of the L2 transaction that requested the withdrawal.
    /// @dev The key is the message id of the L1Message of the transaction.
    /// @dev The value is a boolean indicating if the withdrawal was claimed or not.
    mapping(uint256 => bool) public claimedWithdrawalIDs;

    /// @notice Maximum time the sequencer is allowed to take without processing a privileged transaction
    /// @notice Specified in seconds.
    uint256 public PRIVILEGED_TX_MAX_WAIT_BEFORE_INCLUSION;

    /// @notice Deadline for the sequencer to include the transaction.
    mapping(bytes32 => uint256) public privilegedTxDeadline;

    address public NATIVE_TOKEN_L1_ADDRESS;

    modifier onlyOnChainProposer() {
        require(
            msg.sender == ON_CHAIN_PROPOSER,
            "CommonBridge: caller is not the OnChainProposer"
        );
        _;
    }

    /// @notice Initializes the contract.
    /// @dev This method is called only once after the contract is deployed.
    /// @dev It sets the OnChainProposer address.
    /// @param owner the address of the owner who can perform upgrades.
    /// @param onChainProposer the address of the OnChainProposer contract.
    function initialize(
        address owner,
        address onChainProposer,
        uint256 inclusionMaxWait,
        address _nativeTokenL1Address
    ) public initializer {
        require(
            onChainProposer != address(0),
            "CommonBridge: onChainProposer is the zero address"
        );
        ON_CHAIN_PROPOSER = onChainProposer;

        lastFetchedL1Block = block.number;
        transactionId = 0;

        PRIVILEGED_TX_MAX_WAIT_BEFORE_INCLUSION = inclusionMaxWait;

        NATIVE_TOKEN_L1_ADDRESS = _nativeTokenL1Address;

        OwnableUpgradeable.__Ownable_init(owner);
        ReentrancyGuardUpgradeable.__ReentrancyGuard_init();
    }

    /// @inheritdoc ICommonBridge
    function getPendingTransactionHashes()
        public
        view
        returns (bytes32[] memory)
    {
        return pendingTxHashes;
    }

    /// Burns at least {amount} gas
    function _burnGas(uint256 amount) private view {
        uint256 startingGas = gasleft();
        while (startingGas - gasleft() < amount) {}
    }


    /// EIP-7702 delegated accounts have code beginning with this.
    bytes3 internal constant EIP7702_PREFIX = 0xef0100;
    /// Code size in bytes of an EIP-7702 delegated account
    /// = len(EIP7702_PREFIX) + len(account)
    uint256 internal constant EIP7702_CODE_LENGTH = 23;

    /// This is intentionally different from the constant Optimism uses, but arbitrary.
    uint256 internal constant ADDRESS_ALIASING = uint256(uint160(0xEe110000000000000000000000000000000011Ff));

    /// @notice This implements address aliasing, inspired by [Optimism](https://docs.optimism.io/stack/differences#address-aliasing)
    /// @dev The purpose of this is to prevent L2 contracts from being impersonated by malicious L1 contracts at the same address
    /// @dev We don't want this to affect users, so we need to detect if the caller is an EOA
    /// @dev We still want L2 contracts to be able to know who called in on L1
    /// @dev So we modify the calling address by with a constant
    function _getSenderAlias() private view returns (address) {
        // If sender is origin, the account is an EOA
        if (msg.sender == tx.origin) return msg.sender;
        // Check for an EIP7702 delegate it account
        if (msg.sender.code.length == EIP7702_CODE_LENGTH) {
            if (bytes3(msg.sender.code) == EIP7702_PREFIX) {
                // And treat it as an EOA
                return msg.sender;
            }
        }
        return address(uint160(uint256(uint160(msg.sender)) + ADDRESS_ALIASING));
    }

    function _sendToL2(address from, SendValues memory sendValues) private {
        _burnGas(sendValues.gasLimit);

        bytes32 l2MintTxHash = keccak256(
            bytes.concat(
                bytes20(from),
                bytes20(sendValues.to),
                bytes32(transactionId),
                bytes32(sendValues.value),
                bytes32(sendValues.gasLimit),
                bytes32(keccak256(sendValues.data))
            )
        );

        pendingTxHashes.push(l2MintTxHash);

        emit PrivilegedTxSent(
            msg.sender,
            from,
            sendValues.to,
            transactionId,
            sendValues.value,
            sendValues.gasLimit,
            sendValues.data
        );
        transactionId += 1;
        privilegedTxDeadline[l2MintTxHash] =
            block.timestamp +
            PRIVILEGED_TX_MAX_WAIT_BEFORE_INCLUSION;
    }

    /// @inheritdoc ICommonBridge
    function sendToL2(SendValues calldata sendValues) public override whenNotPaused {
        _sendToL2(_getSenderAlias(), sendValues);
    }

    /// @inheritdoc ICommonBridge
    function deposit(
        uint256 _amount,
        address _token,
        address l2Recipient
    ) public payable override whenNotPaused {
        uint256 value;

        if (_token == address(0)) {
            require(
                msg.value > 0,
                "CommonBridge: the native token is ETH, msg.value must be greater than zero"
            );

            require(
                _amount == 0,
                "CommonBridge: the native token is ETH, _amount must be zero"
            );

            value = msg.value;
        } else {
            require(
                msg.value == 0,
                "CommonBridge: the native token is an ERC20, msg.value must be zero"
            );

            require(
                _token == NATIVE_TOKEN_L1_ADDRESS,
                "CommonBridge: token address is not the native token address. Use depositERC20() instead"
            );

            require(
                _amount > 0,
                "CommonBridge: the native token is an ERC20, _amount must be greater than zero"
            );

            value = _amount;

            IERC20(_token).safeTransferFrom(msg.sender, address(this), value);
        }

        deposits[ETH_TOKEN][ETH_TOKEN] += value;

        bytes memory callData = abi.encodeCall(
            ICommonBridgeL2.mintETH,
            (l2Recipient)
        );

        SendValues memory sendValues = SendValues({
            to: L2_BRIDGE_ADDRESS,
            gasLimit: 21000 * 5,
            value: value,
            data: callData
        });

        _sendToL2(L2_BRIDGE_ADDRESS, sendValues);
    }

    receive() external payable whenNotPaused {
        revert("CommonBridge: use deposit() for native token deposits");
    }

    function depositERC20(
        address tokenL1,
        address tokenL2,
        address destination,
        uint256 amount
    ) external whenNotPaused {
        require(amount > 0, "CommonBridge: amount to deposit is zero");
        deposits[tokenL1][tokenL2] += amount;
        IERC20(tokenL1).safeTransferFrom(msg.sender, address(this), amount);

        bytes memory callData = abi.encodeCall(
            ICommonBridgeL2.mintERC20,
            (tokenL1, tokenL2, destination, amount)
        );
        SendValues memory sendValues = SendValues({
            to: L2_BRIDGE_ADDRESS,
            gasLimit: 21000 * 5,
            value: 0,
            data: callData
        });
        _sendToL2(L2_BRIDGE_ADDRESS, sendValues);
    }

    /// @inheritdoc ICommonBridge
    function getPendingTransactionsVersionedHash(
        uint16 number
    ) public view returns (bytes32) {
        require(number > 0, "CommonBridge: number is zero (get)");
        require(
            uint256(number) <= pendingTxHashes.length,
            "CommonBridge: number is greater than the length of pendingTxHashes (get)"
        );

        bytes memory hashes;
        for (uint i = 0; i < number; i++) {
            hashes = bytes.concat(hashes, pendingTxHashes[i]);
        }

        return
            bytes32(bytes2(number)) |
            bytes32(uint256(uint240(uint256(keccak256(hashes)))));
    }

    /// @inheritdoc ICommonBridge
    function removePendingTransactionHashes(
        uint16 number
    ) public onlyOnChainProposer {
        require(
            number <= pendingTxHashes.length,
            "CommonBridge: number is greater than the length of pendingTxHashes (remove)"
        );

        for (uint i = 0; i < pendingTxHashes.length - number; i++) {
            pendingTxHashes[i] = pendingTxHashes[i + number];
        }

        for (uint _i = 0; _i < number; _i++) {
            pendingTxHashes.pop();
        }
    }

    /// @inheritdoc ICommonBridge
    function hasExpiredPrivilegedTransactions() public view returns (bool) {
        if (pendingTxHashes.length == 0) {
            return false;
        }
        return block.timestamp > privilegedTxDeadline[pendingTxHashes[0]];
    }

    /// @inheritdoc ICommonBridge
    function getWithdrawalLogsMerkleRoot(
        uint256 blockNumber
    ) public view returns (bytes32) {
        return batchWithdrawalLogsMerkleRoots[blockNumber];
    }

    /// @inheritdoc ICommonBridge
    function publishWithdrawals(
        uint256 withdrawalLogsBatchNumber,
        bytes32 withdrawalsLogsMerkleRoot
    ) public onlyOnChainProposer {
        require(
            batchWithdrawalLogsMerkleRoots[withdrawalLogsBatchNumber] ==
                bytes32(0),
            "CommonBridge: withdrawal logs already published"
        );
        batchWithdrawalLogsMerkleRoots[
            withdrawalLogsBatchNumber
        ] = withdrawalsLogsMerkleRoot;
        emit WithdrawalsPublished(
            withdrawalLogsBatchNumber,
            withdrawalsLogsMerkleRoot
        );
    }

    /// @inheritdoc ICommonBridge
    function claimWithdrawal(
        uint256 claimedAmount,
        uint256 withdrawalBatchNumber,
        uint256 withdrawalMessageId,
        bytes32[] calldata withdrawalProof
    ) public override whenNotPaused {
        _claimWithdrawal(
            ETH_TOKEN,
            ETH_TOKEN,
            claimedAmount,
            withdrawalBatchNumber,
            withdrawalMessageId,
            withdrawalProof
        );
        (bool success, ) = payable(msg.sender).call{value: claimedAmount}("");
        require(success, "CommonBridge: failed to send the claimed amount");
    }

    /// @inheritdoc ICommonBridge
    function claimWithdrawalERC20(
        address tokenL1,
        address tokenL2,
        uint256 claimedAmount,
        uint256 withdrawalBatchNumber,
        uint256 withdrawalMessageId,
        bytes32[] calldata withdrawalProof
    ) public nonReentrant override whenNotPaused {
        _claimWithdrawal(
            tokenL1,
            tokenL2,
            claimedAmount,
            withdrawalBatchNumber,
            withdrawalMessageId,
            withdrawalProof
        );
        require(
            tokenL1 != ETH_TOKEN,
            "CommonBridge: attempted to withdraw ETH as if it were ERC20, use claimWithdrawal()"
        );
        IERC20(tokenL1).safeTransfer(msg.sender, claimedAmount);
    }

    function _claimWithdrawal(
        address tokenL1,
        address tokenL2,
        uint256 claimedAmount,
        uint256 withdrawalBatchNumber,
        uint256 withdrawalMessageId,
        bytes32[] calldata withdrawalProof
    ) private {
        require(
            deposits[tokenL1][tokenL2] >= claimedAmount,
            "CommonBridge: trying to withdraw more tokens/ETH than were deposited"
        );
        deposits[tokenL1][tokenL2] -= claimedAmount;
        bytes32 msgHash = keccak256(
            abi.encodePacked(tokenL1, tokenL2, msg.sender, claimedAmount)
        );
        require(
            batchWithdrawalLogsMerkleRoots[withdrawalBatchNumber] != bytes32(0),
            "CommonBridge: the batch that emitted the withdrawal logs was not committed"
        );
        require(
            withdrawalBatchNumber <=
                IOnChainProposer(ON_CHAIN_PROPOSER).lastVerifiedBatch(),
            "CommonBridge: the batch that emitted the withdrawal logs was not verified"
        );
        require(
            claimedWithdrawalIDs[withdrawalMessageId] == false,
            "CommonBridge: the withdrawal was already claimed"
        );
        claimedWithdrawalIDs[withdrawalMessageId] = true;
        emit WithdrawalClaimed(withdrawalMessageId);
        require(
            _verifyMessageProof(
                msgHash,
                withdrawalBatchNumber,
                withdrawalMessageId,
                withdrawalProof
            ),
            "CommonBridge: Invalid proof"
        );
    }

    function _verifyMessageProof(
        bytes32 msgHash,
        uint256 withdrawalBatchNumber,
        uint256 withdrawalMessageId,
        bytes32[] calldata withdrawalProof
    ) internal view returns (bool) {
        bytes32 withdrawalLeaf = keccak256(
            abi.encodePacked(
                L2_BRIDGE_ADDRESS,
                msgHash,
                withdrawalMessageId
            )
        );
        return
            MerkleProof.verify(
                withdrawalProof,
                batchWithdrawalLogsMerkleRoots[withdrawalBatchNumber],
                withdrawalLeaf
            );
    }

    function upgradeL2Contract(address l2Contract, address newImplementation, uint256 gasLimit, bytes calldata data) public onlyOwner {
        bytes memory callData = abi.encodeCall(ITransparentUpgradeableProxy.upgradeToAndCall, (newImplementation, data));
        SendValues memory sendValues = SendValues({
            to: l2Contract,
            gasLimit: gasLimit,
            value: 0,
            data: callData
        });
        _sendToL2(L2_PROXY_ADMIN, sendValues);
    }

    /// @notice Allow owner to upgrade the contract.
    /// @param newImplementation the address of the new implementation
    function _authorizeUpgrade(
        address newImplementation
    ) internal virtual override onlyOwner {}

    /// @inheritdoc ICommonBridge
    function pause() external override onlyOwner {
        _pause();
    }

    /// @inheritdoc ICommonBridge
    function unpause() external override onlyOwner {
        _unpause();
    }
}
