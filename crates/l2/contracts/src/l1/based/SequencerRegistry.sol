// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/Ownable2StepUpgradeable.sol";
import "../interfaces/ISequencerRegistry.sol";
import "./interfaces/IOnChainProposer.sol";

/// @title SequencerRegistry contract
/// @author LambdaClass
contract SequencerRegistry is
    ISequencerRegistry,
    Initializable,
    UUPSUpgradeable,
    Ownable2StepUpgradeable
{
    /// @notice Minimum collateral required to register as a sequencer.
    /// @dev Any value sent to the register function must be greater than or equal to this value.
    /// @dev This value is set to 1 ether.
    uint256 public constant MIN_COLLATERAL = 1 ether;

    /// @notice Number of batches per sequencer that are allowed to commit.
    /// @dev This is to establish a fixed amount of batches per sequencer.
    /// @dev After completing this amount, the lead sequencer is changed to the next one.
    uint256 public constant BATCHES_PER_SEQUENCER = 32;

    /// @notice The address of the OnChainProposer contract.
    /// @dev This address is set during initialization and is used to get the last committed batch.
    address public ON_CHAIN_PROPOSER;

    /// @notice Mapping of sequencer addresses to their collateral amounts.
    /// @dev This mapping is used to check if a sequencer is registered and how much collateral they have put.
    /// @dev The collateral is in wei.
    mapping(address => uint256) public collateral;

    /// @notice List of registered sequencers.
    /// @dev This list is used to iterate over the registered sequencers.
    /// @dev The order of the sequencers in this list is important for determining the leader for future a batch.
    address[] public sequencers;

    /// @notice Mapping of batch numbers to the sequencer that committed them.
    /// @dev This mapping is used to keep track of which sequencer committed which batch.
    mapping(uint256 => address) public sequencerForBatch;

    /// @notice Modifier to restrict access to the OnChainProposer contract.
    /// @dev This modifier is used to ensure the mapping of the batch and the sequencer that committed it
    /// is only updated by the OnChainProposer contract.
    modifier onlyOnChainProposer() {
        require(
            msg.sender == ON_CHAIN_PROPOSER,
            "SequencerRegistry: Only onChainProposer can push sequencer"
        );
        _;
    }

    /// @notice Initializes the SequencerRegistry contract.
    /// @dev This function is called during the deployment of the contract.
    /// @dev It sets the address of the OnChainProposer contract and the owner of the contract.
    function initialize(
        address owner,
        address onChainProposer
    ) public initializer {
        require(
            onChainProposer != address(0),
            "SequencerRegistry: Invalid onChainProposer"
        );

        ON_CHAIN_PROPOSER = onChainProposer;

        require(owner != address(0), "SequencerRegistry: Invalid owner");

        OwnableUpgradeable.__Ownable_init(owner);
    }

    /// @inheritdoc ISequencerRegistry
    function pushSequencer(
        uint256 batchNumber,
        address sequencer
    ) external override onlyOnChainProposer {
        sequencerForBatch[batchNumber] = sequencer;
    }

    /// @notice Register a sequencer.
    /// @dev This function allows a sequencer to register itself by providing collateral.
    /// @dev The sequencer does not have to be previously registered.
    function register(address sequencer) public payable {
        require(
            collateral[sequencer] == 0,
            "SequencerRegistry: Already registered"
        );
        require(
            msg.value >= MIN_COLLATERAL,
            "SequencerRegistry: Insufficient collateral"
        );

        collateral[sequencer] = msg.value;
        sequencers.push(sequencer);

        emit SequencerRegistered(sequencer, msg.value);
    }

    /// @inheritdoc ISequencerRegistry
    function unregister(address sequencer) public {
        require(collateral[sequencer] > 0, "SequencerRegistry: Not registered");

        uint256 amount = collateral[sequencer];
        collateral[sequencer] = 0;
        for (uint256 i = 0; i < sequencers.length; i++) {
            if (sequencers[i] == sequencer) {
                sequencers[i] = sequencers[sequencers.length - 1];
                sequencers.pop();
                break;
            }
        }

        payable(sequencer).transfer(amount);

        emit SequencerUnregistered(sequencer);
    }

    /// @inheritdoc ISequencerRegistry
    function isRegistered(address sequencer) public view returns (bool) {
        return collateral[sequencer] >= MIN_COLLATERAL;
    }

    /// @inheritdoc ISequencerRegistry
    function leaderSequencer() public view returns (address) {
        uint256 _currentBatch = IOnChainProposer(ON_CHAIN_PROPOSER)
            .lastCommittedBatch() + 1;
        return leadSequencerForBatch(_currentBatch);
    }

    /// @inheritdoc ISequencerRegistry
    function leadSequencerForBatch(
        uint256 batchNumber
    ) public view returns (address) {
        uint256 _currentBatch = IOnChainProposer(ON_CHAIN_PROPOSER)
            .lastCommittedBatch() + 1;
        if (batchNumber < _currentBatch) {
            return sequencerForBatch[batchNumber];
        }
        uint256 _sequencersLength = sequencers.length;

        if (_sequencersLength == 0) {
            return address(0);
        }

        uint256 batchSlot = batchNumber / BATCHES_PER_SEQUENCER;

        address _leader = sequencers[batchSlot % _sequencersLength];

        return _leader;
    }

    /// @notice Allow owner to upgrade the contract.
    /// @param newImplementation the address of the new implementation
    function _authorizeUpgrade(
        address newImplementation
    ) internal virtual override onlyOwner {}
}
