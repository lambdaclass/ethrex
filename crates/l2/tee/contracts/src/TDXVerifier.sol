// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "../lib/openzeppelin-contracts/contracts/utils/cryptography/ECDSA.sol";
import "../lib/openzeppelin-contracts/contracts/utils/cryptography/MessageHashUtils.sol";
import "../lib/openzeppelin-contracts/contracts/access/Ownable.sol";

interface IAttestation {
    function verifyAndAttestOnChain(bytes calldata rawQuote)
        external
        payable
        returns (bool success, bytes memory output);
}

interface IOnChainProposer {
    function authorizedSequencerAddresses(address addr) external returns (bool isAuthorized);
}

contract TDXVerifier is Ownable {
    IAttestation public quoteVerifier = IAttestation(address(0));
    address public onChainProposer = address(0);

    address public authorizedSignature = address(0);
    bool public isDevMode = false;

    bytes public RTMR0;
    bytes public RTMR1;
    bytes public RTMR2;
    bytes public MRTD;

    /// @notice Initializes the contract
    /// @param _dcap DCAP contract.
    /// @param _isDevMode Disables quote verification
    constructor(address _dcap, bytes memory _rtmr0, bytes memory _rtmr1, bytes memory _rtmr2, bytes memory _mrtd, bool _isDevMode) Ownable(msg.sender) {
        require(_dcap != address(0), "TDXVerifier: DCAP address can't be null");

        quoteVerifier = IAttestation(_dcap);
        isDevMode = _isDevMode;

        require(_rtmr0.length == 48, "RTMR0 must have 48 bytes");
        require(_rtmr1.length == 48, "RTMR1 must have 48 bytes");
        require(_rtmr2.length == 48, "RTMR2 must have 48 bytes");
        require(_mrtd.length == 48, "MRTD must have 48 bytes");

        RTMR0 = _rtmr0;
        RTMR1 = _rtmr1;
        RTMR2 = _rtmr2;
        MRTD = _mrtd;
    }

    /// @notice Initializes the OnChainProposer
    /// @param _ocp OnChainProposer contract address, used for permission checks
    function initializeOnChainProposer(address _ocp) public onlyOwner {
        require(onChainProposer == address(0), "TDXVerifier: OnChainProposer already initialized");
        require(_ocp != address(0), "TDXVerifier: OnChainPropser address can't be null");
        onChainProposer = _ocp;
    }

    /// @notice Verifies a proof with given payload and signature
    /// @dev The signature should correspond to an address previously registered with the verifier
    /// @param payload The payload to be verified
    /// @param signature The associated signature
    function verify(
        bytes calldata payload,
        bytes memory signature
    ) external view {
        require(authorizedSignature != address(0), "TDXVerifier: authorized signer not registered");
        bytes32 signedHash = MessageHashUtils.toEthSignedMessageHash(payload);
        require(ECDSA.recover(signedHash, signature) == authorizedSignature, "TDXVerifier: invalid signature");
    }

    /// @notice Registers the quote
    /// @dev The data required to verify the quote must be loaded to the PCCS contracts beforehand
    /// @param quote The TDX quote, which includes the address being registered
    function register(
        bytes calldata quote
    ) external {
        require(
            IOnChainProposer(onChainProposer).authorizedSequencerAddresses(msg.sender),
            "TDXVerifier: only sequencer can update keys"
        );
        // TODO: only allow the owner to update the key, to avoid DoS
        if (isDevMode) {
            authorizedSignature = _getAddress(quote, 0);
            return;
        }
        (bool success, bytes memory report) = quoteVerifier.verifyAndAttestOnChain(quote);
        require(success, "TDXVerifier: quote verification failed");
        _validateReport(report);
        authorizedSignature = _getAddress(report, 533);
    }

    function _validateReport(bytes memory report) view internal {
        require(_rangeEquals(report, 0, hex'0004'), "TDXVerifier: Unsupported quote version");
        require(report[2] == 0x81, "TDXVerifier: Quote is not of type TDX");
        require(report[6] == 0, "TDXVerifier: TCB_STATUS != OK");
        require(uint8(report[133]) & 15 == 0, "TDXVerifier: debug attributes are set");
        require(_rangeEquals(report, 149, MRTD), "TDXVerifier: MRTD mismatch");
        require(_rangeEquals(report, 341, RTMR0), "TDXVerifier: RTMR0 mismatch");
        require(_rangeEquals(report, 389, RTMR1), "TDXVerifier: RTMR1 mismatch");
        require(_rangeEquals(report, 437, RTMR2), "TDXVerifier: RTMR2 mismatch");
        // RTMR3 is ignored
    }

    function _getAddress(bytes memory report, uint256 offset) pure public returns (address) {
        uint256 addr;
        for (uint8 i = 0; i < 20; i++) {
            addr = (addr << 8) | uint8(report[offset + i]);
        }
        return address(uint160(addr));
    }

    function _rangeEquals(bytes memory report, uint256 offset, bytes memory other) pure internal returns (bool) {
        for (uint256 i; i < other.length; i++) {
            if (report[offset + i] != other[i]) return false;
        }
        return true;
    }
}
