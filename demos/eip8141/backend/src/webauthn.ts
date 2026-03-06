import { encodeAbiParameters, parseAbiParameters } from "viem";
import { hexToBytes } from "./frame-tx.js";

/**
 * The Solidity function signatures we need to encode calldata for:
 *
 * verify((uint256 r, uint256 s), (bytes authenticatorData, string clientDataJSON,
 *         uint16 challengeIndex, uint16 typeIndex, bool userVerificationRequired))
 *
 * verifyAndPay((uint256 r, uint256 s), (bytes authenticatorData, string clientDataJSON,
 *              uint16 challengeIndex, uint16 typeIndex, bool userVerificationRequired))
 */

// Function selectors (first 4 bytes of keccak256 of the signature)
// verify((uint256,uint256),(bytes,string,uint16,uint16,bool))
const VERIFY_SELECTOR = "0x182ffd20";
// verifyAndPay((uint256,uint256),(bytes,string,uint16,uint16,bool))
const VERIFY_AND_PAY_SELECTOR = "0x5a27d2e0";

const PARAMS_ABI = parseAbiParameters([
  "(uint256 r, uint256 s) sig",
  "(bytes authenticatorData, string clientDataJSON, uint16 challengeIndex, uint16 typeIndex, bool userVerificationRequired) metadata",
]);

interface WebAuthnSig {
  r: bigint;
  s: bigint;
}

interface WebAuthnMetadataInput {
  authenticatorData: Uint8Array;
  clientDataJSON: string;
  challengeIndex: number;
  typeIndex: number;
  userVerificationRequired: boolean;
}

function encodeParams(
  sig: WebAuthnSig,
  metadata: WebAuthnMetadataInput
): `0x${string}` {
  return encodeAbiParameters(PARAMS_ABI, [
    { r: sig.r, s: sig.s },
    {
      authenticatorData: `0x${Array.from(metadata.authenticatorData)
        .map((b) => b.toString(16).padStart(2, "0"))
        .join("")}` as `0x${string}`,
      clientDataJSON: metadata.clientDataJSON,
      challengeIndex: metadata.challengeIndex,
      typeIndex: metadata.typeIndex,
      userVerificationRequired: metadata.userVerificationRequired,
    },
  ]);
}

export function encodeVerifyCalldata(
  sig: WebAuthnSig,
  metadata: WebAuthnMetadataInput
): Uint8Array {
  const encoded = encodeParams(sig, metadata);
  const paramsBytes = hexToBytes(encoded);
  const selectorBytes = hexToBytes(VERIFY_SELECTOR);
  const result = new Uint8Array(4 + paramsBytes.length);
  result.set(selectorBytes, 0);
  result.set(paramsBytes, 4);
  return result;
}

export function encodeVerifyAndPayCalldata(
  sig: WebAuthnSig,
  metadata: WebAuthnMetadataInput
): Uint8Array {
  const encoded = encodeParams(sig, metadata);
  const paramsBytes = hexToBytes(encoded);
  const selectorBytes = hexToBytes(VERIFY_AND_PAY_SELECTOR);
  const result = new Uint8Array(4 + paramsBytes.length);
  result.set(selectorBytes, 0);
  result.set(paramsBytes, 4);
  return result;
}
