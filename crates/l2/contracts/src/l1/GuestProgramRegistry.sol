// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "@openzeppelin/contracts-upgradeable/proxy/utils/UUPSUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/Ownable2StepUpgradeable.sol";
import "./interfaces/IGuestProgramRegistry.sol";

/// @title GuestProgramRegistry contract.
/// @author Tokamak Network
/// @notice Registry for guest programs that enable app-specific L2s.
/// @dev Guest programs are custom circuit + contract combinations. Each registered
/// program receives a unique programTypeId that links to the OnChainProposer's
/// 3D verification key mapping: verificationKeys[commitHash][programTypeId][verifierId].
///
/// Program type ID allocation:
///   0       = Reserved (defaults to EVM-L2 in OnChainProposer)
///   1       = EVM-L2 (default, pre-registered)
///   2-9     = Official templates (reserved for core team)
///   10-255  = Store programs (community-registered)
contract GuestProgramRegistry is
    IGuestProgramRegistry,
    Initializable,
    UUPSUpgradeable,
    Ownable2StepUpgradeable
{
    /// @notice The first programTypeId available for community programs.
    uint8 public constant STORE_PROGRAM_START_ID = 10;

    /// @notice The next programTypeId to be assigned to a new program.
    /// @dev Starts at STORE_PROGRAM_START_ID and increments.
    uint8 public nextProgramTypeId;

    /// @notice Mapping from programId string hash to ProgramInfo.
    /// @dev Uses keccak256(programId) as key for efficient lookups.
    mapping(bytes32 => ProgramInfo) internal _programs;

    /// @notice Mapping from programId string hash to the original programId string.
    /// @dev Stored separately to allow string retrieval from hash.
    mapping(bytes32 => string) internal _programIds;

    /// @notice Mapping from programTypeId to programId hash.
    /// @dev Allows reverse lookup: typeId → programId → ProgramInfo.
    mapping(uint8 => bytes32) internal _typeIdToHash;

    /// @notice Total number of registered programs (including inactive).
    uint8 public programCount;

    /// @notice Initializes the GuestProgramRegistry contract.
    /// @dev Registers the default EVM-L2 program with typeId=1.
    /// @param owner The admin address (typically the Timelock contract).
    function initialize(address owner) public initializer {
        require(owner != address(0), "GuestProgramRegistry: Invalid owner");
        OwnableUpgradeable.__Ownable_init(owner);

        nextProgramTypeId = STORE_PROGRAM_START_ID;

        // Pre-register the default EVM-L2 program
        bytes32 evmHash = keccak256(bytes("evm-l2"));
        _programs[evmHash] = ProgramInfo({
            programTypeId: 1,
            creator: owner,
            name: "EVM L2",
            active: true,
            registeredAt: block.number
        });
        _programIds[evmHash] = "evm-l2";
        _typeIdToHash[1] = evmHash;
        programCount = 1;
    }

    /// @inheritdoc IGuestProgramRegistry
    function registerProgram(
        string calldata programId,
        string calldata name,
        address creator
    ) external override onlyOwner returns (uint8 programTypeId) {
        require(bytes(programId).length > 0, "GuestProgramRegistry: Empty programId");
        require(bytes(name).length > 0, "GuestProgramRegistry: Empty name");
        require(creator != address(0), "GuestProgramRegistry: Invalid creator");
        require(nextProgramTypeId > 0, "GuestProgramRegistry: Type ID overflow");

        bytes32 idHash = keccak256(bytes(programId));
        require(
            _programs[idHash].programTypeId == 0,
            "GuestProgramRegistry: Program already registered"
        );

        programTypeId = nextProgramTypeId;
        nextProgramTypeId++;

        _programs[idHash] = ProgramInfo({
            programTypeId: programTypeId,
            creator: creator,
            name: name,
            active: true,
            registeredAt: block.number
        });
        _programIds[idHash] = programId;
        _typeIdToHash[programTypeId] = idHash;
        programCount++;

        emit ProgramRegistered(programId, programTypeId, creator);
    }

    /// @inheritdoc IGuestProgramRegistry
    function deactivateProgram(string calldata programId) external override onlyOwner {
        bytes32 idHash = keccak256(bytes(programId));
        ProgramInfo storage info = _programs[idHash];
        require(info.programTypeId != 0, "GuestProgramRegistry: Program not found");
        require(info.active, "GuestProgramRegistry: Already inactive");
        require(info.programTypeId != 1, "GuestProgramRegistry: Cannot deactivate default program");

        info.active = false;

        emit ProgramDeactivated(programId, info.programTypeId);
    }

    /// @inheritdoc IGuestProgramRegistry
    function activateProgram(string calldata programId) external override onlyOwner {
        bytes32 idHash = keccak256(bytes(programId));
        ProgramInfo storage info = _programs[idHash];
        require(info.programTypeId != 0, "GuestProgramRegistry: Program not found");
        require(!info.active, "GuestProgramRegistry: Already active");

        info.active = true;

        emit ProgramActivated(programId, info.programTypeId);
    }

    /// @inheritdoc IGuestProgramRegistry
    function getProgram(string calldata programId) external view override returns (ProgramInfo memory info) {
        bytes32 idHash = keccak256(bytes(programId));
        info = _programs[idHash];
        require(info.programTypeId != 0, "GuestProgramRegistry: Program not found");
    }

    /// @inheritdoc IGuestProgramRegistry
    function getProgramByTypeId(uint8 programTypeId) external view override returns (ProgramInfo memory info) {
        bytes32 idHash = _typeIdToHash[programTypeId];
        require(idHash != bytes32(0), "GuestProgramRegistry: Type ID not found");
        info = _programs[idHash];
    }

    /// @inheritdoc IGuestProgramRegistry
    function isProgramActive(uint8 programTypeId) external view override returns (bool) {
        bytes32 idHash = _typeIdToHash[programTypeId];
        if (idHash == bytes32(0)) return false;
        return _programs[idHash].active;
    }

    /// @notice Register an official/reserved program with a specific typeId (2-9).
    /// @dev Only callable by the owner. Used for core team templates.
    /// @param programId Unique string identifier.
    /// @param name Human-readable display name.
    /// @param creator The address of the program creator.
    /// @param typeId The specific type ID to assign (must be 2-9).
    function registerOfficialProgram(
        string calldata programId,
        string calldata name,
        address creator,
        uint8 typeId
    ) external onlyOwner {
        require(typeId >= 2 && typeId < STORE_PROGRAM_START_ID, "GuestProgramRegistry: Invalid official typeId");
        require(bytes(programId).length > 0, "GuestProgramRegistry: Empty programId");
        require(bytes(name).length > 0, "GuestProgramRegistry: Empty name");
        require(creator != address(0), "GuestProgramRegistry: Invalid creator");

        bytes32 idHash = keccak256(bytes(programId));
        require(
            _programs[idHash].programTypeId == 0,
            "GuestProgramRegistry: Program already registered"
        );
        require(
            _typeIdToHash[typeId] == bytes32(0),
            "GuestProgramRegistry: Type ID already taken"
        );

        _programs[idHash] = ProgramInfo({
            programTypeId: typeId,
            creator: creator,
            name: name,
            active: true,
            registeredAt: block.number
        });
        _programIds[idHash] = programId;
        _typeIdToHash[typeId] = idHash;
        programCount++;

        emit ProgramRegistered(programId, typeId, creator);
    }

    /// @notice Get the programId string for a given type ID.
    /// @param programTypeId The numeric type ID.
    /// @return The programId string.
    function getProgramIdByTypeId(uint8 programTypeId) external view returns (string memory) {
        bytes32 idHash = _typeIdToHash[programTypeId];
        require(idHash != bytes32(0), "GuestProgramRegistry: Type ID not found");
        return _programIds[idHash];
    }

    /// @notice Allow owner to upgrade the contract.
    /// @param newImplementation the address of the new implementation
    function _authorizeUpgrade(
        address newImplementation
    ) internal virtual override onlyOwner {}
}
