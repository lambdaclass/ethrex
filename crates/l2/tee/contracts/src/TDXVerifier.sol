// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";
import {ITimelock} from "../../../contracts/src/l1/interfaces/ITimelock.sol";

interface IAttestation {
    function verifyAndAttestOnChain(
        bytes calldata rawQuote
    ) external payable returns (bool success, bytes memory output);
}

contract TDXVerifier {
    IAttestation public quoteVerifier = IAttestation(address(0));
    ITimelock public timelock = ITimelock(address(0));

    address public authorizedSignature = address(0);
    bool public isDevMode = false;

    bytes public RTMR0 =
        hex"4f3d617a1c89bd9a89ea146c15b04383b7db7318f41a851802bba8eace5a6cf71050e65f65fd50176e4f006764a42643";
    bytes public RTMR1 =
        hex"53827a034d1e4c7f13fd2a12aee4497e7097f15a04794553e12fe73e2ffb8bd57585e771951115a13ec4d7e6bc193038";
    bytes public RTMR2 =
        hex"2ca1a728ff13c36195ad95e8f725bf00d7f9c5d6ed730fb8f50cccad692ab81aefc83d594819375649be934022573528";
    bytes public MRTD =
        hex"91eb2b44d141d4ece09f0c75c2c53d247a3c68edd7fafe8a3520c942a604a407de03ae6dc5f87f27428b2538873118b7";

    /// @notice Initializes the contract
    /// @param _dcap DCAP contract.
    /// @param _timelock Timelock contract, used for permission checks
    /// @param _isDevMode Disables quote verification
    constructor(address _dcap, address _timelock, bool _isDevMode) {
        require(_dcap != address(0), "TDXVerifier: DCAP address can't be null");
        require(
            _timelock != address(0),
            "TDXVerifier: Timelock address can't be null"
        );

        quoteVerifier = IAttestation(_dcap);
        timelock = ITimelock(_timelock);
        isDevMode = _isDevMode;
    }

    /// @notice Verifies a proof with given payload and signature
    /// @dev The signature should correspond to an address previously registered with the verifier
    /// @param payload The payload to be verified
    /// @param signature The associated signature
    function verify(
        bytes calldata payload,
        bytes memory signature
    ) external view {
        require(
            authorizedSignature != address(0),
            "TDXVerifier: authorized signer not registered"
        );
        bytes32 signedHash = MessageHashUtils.toEthSignedMessageHash(payload);
        require(
            ECDSA.recover(signedHash, signature) == authorizedSignature,
            "TDXVerifier: invalid signature"
        );
    }

    /// @notice Registers the quote
    /// @dev The data required to verify the quote must be loaded to the PCCS contracts beforehand
    /// @param quote The TDX quote, which includes the address being registered
    function register(bytes calldata quote) external {
        require(
            timelock.isSequencer(msg.sender),
            "TDXVerifier: only sequencer can update keys"
        );
        // TODO: only allow the owner to update the key, to avoid DoS
        if (isDevMode) {
            authorizedSignature = _getAddress(quote, 0);
            return;
        }
        (bool success, bytes memory report) = quoteVerifier
            .verifyAndAttestOnChain(quote);
        require(success, "TDXVerifier: quote verification failed");
        _validateReport(report);
        authorizedSignature = _getAddress(report, 533);
    }

    function _validateReport(bytes memory report) internal view {
        require(
            _rangeEquals(report, 0, hex"0004"),
            "TDXVerifier: Unsupported quote version"
        );
        require(report[2] == 0x81, "TDXVerifier: Quote is not of type TDX");
        require(report[6] == 0, "TDXVerifier: TCB_STATUS != OK");
        require(
            uint8(report[133]) & 15 == 0,
            "TDXVerifier: debug attributes are set"
        );
        require(_rangeEquals(report, 149, MRTD), "TDXVerifier: MRTD mismatch");
        require(
            _rangeEquals(report, 341, RTMR0),
            "TDXVerifier: RTMR0 mismatch"
        );
        require(
            _rangeEquals(report, 389, RTMR1),
            "TDXVerifier: RTMR1 mismatch"
        );
        require(
            _rangeEquals(report, 437, RTMR2),
            "TDXVerifier: RTMR2 mismatch"
        );
        // RTMR3 is ignored
    }

    function _getAddress(
        bytes memory report,
        uint256 offset
    ) public pure returns (address) {
        uint256 addr;
        for (uint8 i = 0; i < 20; i++) {
            addr = (addr << 8) | uint8(report[offset + i]);
        }
        return address(uint160(addr));
    }

    function _rangeEquals(
        bytes memory report,
        uint256 offset,
        bytes memory other
    ) internal pure returns (bool) {
        for (uint256 i; i < other.length; i++) {
            if (report[offset + i] != other[i]) return false;
        }
        return true;
    }
}
