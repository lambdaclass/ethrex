// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title WebAuthnP256AccountFactory
/// @notice Deploys WebAuthnP256Account instances via CREATE2.
///         Each account gets a deterministic address derived from its P256 public key.
/// @dev The factory stores the account initcode (constructor + runtime, without pubkey args).
///      On deploy(), it appends the pubkey as constructor args and uses CREATE2.
///      Salt = keccak256(abi.encode(pubKeyX, pubKeyY)).
contract WebAuthnP256AccountFactory {
    /// @notice The WebAuthnP256Account initcode (constructor + runtime).
    ///         Constructor args (pubKeyX, pubKeyY) are appended at deploy time.
    bytes private _accountInitcode;

    /// @notice Whether the factory has been initialized with the account initcode.
    bool public initialized;

    event AccountDeployed(address indexed account, uint256 pubKeyX, uint256 pubKeyY);

    /// @notice Initialize the factory with the account initcode. Can only be called once.
    /// @param initcode_ The compiled WebAuthnP256Account initcode (constructor + runtime).
    function initialize(bytes calldata initcode_) external {
        require(!initialized, "already initialized");
        _accountInitcode = initcode_;
        initialized = true;
    }

    /// @notice Deploy a new WebAuthnP256Account with the given P256 public key.
    /// @param pubKeyX The X coordinate of the P256 public key.
    /// @param pubKeyY The Y coordinate of the P256 public key.
    /// @return account The address of the newly deployed account.
    function deploy(uint256 pubKeyX, uint256 pubKeyY) external returns (address account) {
        require(initialized, "not initialized");
        bytes32 salt = keccak256(abi.encode(pubKeyX, pubKeyY));
        bytes memory initcode = abi.encodePacked(_accountInitcode, pubKeyX, pubKeyY);
        assembly {
            account := create2(0, add(initcode, 32), mload(initcode), salt)
            if iszero(account) { revert(0, 0) }
        }
        emit AccountDeployed(account, pubKeyX, pubKeyY);
    }

    /// @notice Compute the deterministic address for a given public key without deploying.
    /// @param pubKeyX The X coordinate of the P256 public key.
    /// @param pubKeyY The Y coordinate of the P256 public key.
    /// @return The address where the account would be deployed.
    function getAddress(uint256 pubKeyX, uint256 pubKeyY) external view returns (address) {
        require(initialized, "not initialized");
        bytes32 salt = keccak256(abi.encode(pubKeyX, pubKeyY));
        bytes32 initHash = keccak256(abi.encodePacked(_accountInitcode, pubKeyX, pubKeyY));
        return address(uint160(uint256(keccak256(abi.encodePacked(
            bytes1(0xff), address(this), salt, initHash
        )))));
    }
}
