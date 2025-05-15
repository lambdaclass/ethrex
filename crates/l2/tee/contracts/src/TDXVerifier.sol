// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "../lib/openzeppelin-contracts/contracts/utils/cryptography/ECDSA.sol";
import "../lib/openzeppelin-contracts/contracts/utils/cryptography/MessageHashUtils.sol";

interface IAttestation {
    function verifyAndAttestOnChain(bytes calldata rawQuote)
        external
        payable
        returns (bool success, bytes memory output);
}

contract TDXVerifier {
    IAttestation public quoteVerifier = IAttestation(address(0));
    address public authorizedSignature = address(0);

    bytes public RTMR0 = hex'4f3d617a1c89bd9a89ea146c15b04383b7db7318f41a851802bba8eace5a6cf71050e65f65fd50176e4f006764a42643';
    bytes public RTMR1 = hex'ae6d959ed05ad39dea8b03c61c761612d40091c7d5082beaeb54b96231603c65ae7437dcb6794dbc55cfe76f79797532';
    bytes public RTMR2 = hex'0bcc11304c49a603385c460e63e250e212e5992856bb2f69b116a850133401f0e6613a5e62562cd4a55df757c7124045';
    bytes public MRTD = hex'91eb2b44d141d4ece09f0c75c2c53d247a3c68edd7fafe8a3520c942a604a407de03ae6dc5f87f27428b2538873118b7';

    constructor(address _dcap) {
        quoteVerifier = IAttestation(_dcap);
    }

    /// @notice Verifies a proof with given payload and signature
    /// @dev The signature should correspond to an address previously registered with the verifier
    /// @param payload The payload to be verified
    /// @param signature The associated signature
    function verify(
        bytes calldata payload,
        bytes memory signature
    ) external view {
        require(authorizedSignature != address(0), "TDX authorized signer not registered");
        bytes32 signedHash = MessageHashUtils.toEthSignedMessageHash(payload);
        require(ECDSA.recover(signedHash, signature) == authorizedSignature, "invalid signature");
    }

    /// @notice Registers the quote
    /// @dev The data required to verify the quote must be loaded to the PCCS contracts beforehand
    /// @param quote The TDX quote, which includes the address being registered
    function register(
        bytes calldata quote
    ) external {
        // TODO: only allow the owner to update the key, to avoid DoS
        (bool success, bytes memory report) = quoteVerifier.verifyAndAttestOnChain(quote);
        require(success, "quote verification failed");
        _validateReport(report);
        authorizedSignature = _getAddress(report);
    }

    function _validateReport(bytes memory report) view internal {
        
        require(_rangeEquals(report, 0, hex'0004'), "Unsupported quote version");
        require(report[2] == 0x81, "Quote is not of type TDX");
        require(report[6] == 0, "TCB_STATUS != OK");
        require(uint8(report[133]) & 15 == 0, "debug attributes are set");
        require(_rangeEquals(report, 149, MRTD), "MRTD mismatch");
        require(_rangeEquals(report, 341, RTMR0), "RTMR0 mismatch");
        require(_rangeEquals(report, 389, RTMR1), "RTMR1 mismatch");
        require(_rangeEquals(report, 437, RTMR2), "RTMR2 mismatch");
        // RTMR3 is ignored
    }

    function _getAddress(bytes memory report) view internal returns (address) {
        bytes20 addr;
        for (uint8 i = 0; i < 20; i++) {
            addr |= report[533 + i] >> (i * 8);
        }
        return address(addr);
    }

    function _rangeEquals(bytes memory report, uint256 offset, bytes memory other) pure internal returns (bool) {
        for (uint256 i; i < other.length; i++) {
            if (report[offset + i] != other[i]) return false;
        }
        return true;
    }
}
