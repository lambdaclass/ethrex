import { RLP } from "@ethereumjs/rlp";
import { keccak256, toHex } from "viem";
import type { Frame, FrameTxParams } from "./types.js";
import { FRAME_MODE_VERIFY } from "./types.js";

// EIP-8141 frame transaction type prefix
const TX_TYPE_FRAME = 0x06;

/**
 * Encode a bigint as the minimal big-endian byte representation (no leading zeros).
 * Zero encodes as empty bytes (RLP convention).
 */
function bigintToBytes(value: bigint): Uint8Array {
  if (value === 0n) return new Uint8Array(0);
  let hex = value.toString(16);
  if (hex.length % 2 !== 0) hex = "0" + hex;
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.substring(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

/**
 * Encode a single Frame as an RLP list: [mode, target, gasLimit, data]
 * target: 20-byte address for Call, empty bytes for Create (null target)
 */
function encodeFrame(frame: Frame): Uint8Array[] {
  return [
    bigintToBytes(BigInt(frame.mode)),
    frame.target.length === 0 ? new Uint8Array(0) : frame.target,
    bigintToBytes(frame.gasLimit),
    frame.data,
  ];
}

/**
 * Build the RLP input for the transaction fields.
 * Field order: [chainId, nonce, sender, [[mode, target, gasLimit, data], ...],
 *               maxPriorityFee, maxFee, maxBlobFee, blobHashes]
 */
function buildRlpFields(
  params: FrameTxParams,
  frames: Frame[]
): (Uint8Array | Uint8Array[] | (Uint8Array | Uint8Array[])[])[] {
  return [
    bigintToBytes(params.chainId),
    bigintToBytes(params.nonce),
    params.sender,
    frames.map(encodeFrame),
    bigintToBytes(params.maxPriorityFeePerGas),
    bigintToBytes(params.maxFeePerGas),
    bigintToBytes(params.maxFeePerBlobGas ?? 0n),
    (params.blobVersionedHashes ?? []) as Uint8Array[],
  ];
}

export class FrameTransaction {
  public params: FrameTxParams;

  constructor(params: FrameTxParams) {
    this.params = {
      ...params,
      frames: params.frames.map((f) => ({ ...f })),
    };
  }

  /**
   * Returns the canonical encoding: [0x06, ...rlp(fields)]
   */
  encodeCanonical(): Uint8Array {
    const fields = buildRlpFields(this.params, this.params.frames);
    const rlpEncoded = RLP.encode(fields as any);
    const result = new Uint8Array(1 + rlpEncoded.length);
    result[0] = TX_TYPE_FRAME;
    result.set(rlpEncoded, 1);
    return result;
  }

  /**
   * Compute sig_hash: keccak256 of canonical encoding with VERIFY frame data
   * replaced with empty bytes.
   */
  computeSigHash(): Uint8Array {
    const elidedFrames = this.params.frames.map((f) => {
      if (f.mode === FRAME_MODE_VERIFY) {
        return { ...f, data: new Uint8Array(0) };
      }
      return f;
    });
    const fields = buildRlpFields(this.params, elidedFrames);
    const rlpEncoded = RLP.encode(fields as any);
    const withPrefix = new Uint8Array(1 + rlpEncoded.length);
    withPrefix[0] = TX_TYPE_FRAME;
    withPrefix.set(rlpEncoded, 1);
    const hash = keccak256(toHex(withPrefix));
    return hexToBytes(hash);
  }

  /**
   * Replace the data of a VERIFY frame after signing.
   */
  setVerifyFrameData(frameIndex: number, data: Uint8Array): void {
    const frame = this.params.frames[frameIndex];
    if (frame === undefined) {
      throw new Error(`Frame index ${frameIndex} out of bounds`);
    }
    if (frame.mode !== FRAME_MODE_VERIFY) {
      throw new Error(
        `Frame ${frameIndex} is not a VERIFY frame (mode=${frame.mode})`
      );
    }
    frame.data = data;
  }

  /**
   * keccak256 of the full canonical encoding.
   */
  txHash(): Uint8Array {
    const canonical = this.encodeCanonical();
    const hash = keccak256(toHex(canonical));
    return hexToBytes(hash);
  }
}

// Utility: hex string to bytes
export function hexToBytes(hex: string): Uint8Array {
  const clean = hex.startsWith("0x") ? hex.slice(2) : hex;
  const bytes = new Uint8Array(clean.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(clean.substring(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

// Utility: bytes to 0x-prefixed hex string
export function bytesToHex(bytes: Uint8Array): `0x${string}` {
  return `0x${Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("")}` as `0x${string}`;
}
