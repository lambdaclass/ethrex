// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

/// @title Interface for the GuestProgramRegistry contract.
/// @author Tokamak Network
/// @notice Manages registration of guest programs for app-specific L2s.
/// Each guest program represents a custom circuit + contract combination
/// that can be used to run specialized L2s (DEX, payments, gaming, etc.).
interface IGuestProgramRegistry {
    /// @notice Information about a registered guest program.
    struct ProgramInfo {
        /// @notice The unique program type ID assigned on registration.
        uint8 programTypeId;
        /// @notice The address of the creator who registered this program.
        address creator;
        /// @notice Human-readable name of the program.
        string name;
        /// @notice Whether the program is currently active and usable.
        bool active;
        /// @notice Block number when the program was registered.
        uint256 registeredAt;
    }

    /// @notice Emitted when a new guest program is registered.
    /// @param programId The unique string identifier for the program.
    /// @param programTypeId The numeric type ID assigned to the program.
    /// @param creator The address that registered the program.
    event ProgramRegistered(
        string indexed programId,
        uint8 programTypeId,
        address indexed creator
    );

    /// @notice Emitted when a guest program is deactivated.
    /// @param programId The unique string identifier for the program.
    /// @param programTypeId The numeric type ID of the program.
    event ProgramDeactivated(
        string indexed programId,
        uint8 programTypeId
    );

    /// @notice Emitted when a guest program is reactivated.
    /// @param programId The unique string identifier for the program.
    /// @param programTypeId The numeric type ID of the program.
    event ProgramActivated(
        string indexed programId,
        uint8 programTypeId
    );

    /// @notice Register a new guest program.
    /// @dev Only callable by the contract owner (admin).
    /// @param programId Unique string identifier (e.g., "zk-dex", "tokamon").
    /// @param name Human-readable display name.
    /// @param creator The address of the program creator.
    /// @return programTypeId The assigned numeric type ID.
    function registerProgram(
        string calldata programId,
        string calldata name,
        address creator
    ) external returns (uint8 programTypeId);

    /// @notice Deactivate a registered program.
    /// @dev Only callable by the contract owner (admin).
    /// @param programId The program to deactivate.
    function deactivateProgram(string calldata programId) external;

    /// @notice Reactivate a previously deactivated program.
    /// @dev Only callable by the contract owner (admin).
    /// @param programId The program to reactivate.
    function activateProgram(string calldata programId) external;

    /// @notice Get program info by its string identifier.
    /// @param programId The unique string identifier.
    /// @return info The program information.
    function getProgram(string calldata programId) external view returns (ProgramInfo memory info);

    /// @notice Get program info by its numeric type ID.
    /// @param programTypeId The numeric type ID.
    /// @return info The program information.
    function getProgramByTypeId(uint8 programTypeId) external view returns (ProgramInfo memory info);

    /// @notice Check if a program is registered and active.
    /// @param programTypeId The numeric type ID to check.
    /// @return True if the program is registered and active.
    function isProgramActive(uint8 programTypeId) external view returns (bool);

    /// @notice Get the total number of registered programs.
    /// @return count The number of registered programs.
    function programCount() external view returns (uint8 count);
}
