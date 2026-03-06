import { Hono } from "hono";
import { FrameTransaction, bytesToHex, hexToBytes } from "../frame-tx.js";
import { getChainId, getBaseFee, getNonce } from "../rpc.js";
import {
  FRAME_MODE_VERIFY,
  FRAME_MODE_SENDER,
  FRAME_MODE_DEFAULT,
  SPONSOR_ADDRESS,
  MOCK_ERC20_ADDRESS,
} from "../types.js";
import type {
  SigHashRequest,
  SimpleSendParams,
  SponsoredSendParams,
  BatchOpsParams,
  DeployExecuteParams,
  Frame,
} from "../types.js";
import { encodeAbiParameters, parseAbiParameters, parseEther } from "viem";

// Default gas limit per frame
const DEFAULT_FRAME_GAS = 200_000n;

const app = new Hono();

/**
 * Build a Frame TX skeleton (VERIFY data empty) based on demo type.
 */
async function buildSkeleton(
  demoType: string,
  params: Record<string, unknown>
): Promise<FrameTransaction> {
  const from = (params.from as string).toLowerCase();
  const sender = hexToBytes(from);

  const [chainId, baseFee, nonce] = await Promise.all([
    getChainId(),
    getBaseFee(),
    getNonce(from),
  ]);

  const maxPriorityFeePerGas = 1_000_000_000n; // 1 gwei
  const maxFeePerGas = baseFee * 2n + maxPriorityFeePerGas;

  let frames: Frame[];

  switch (demoType) {
    case "simple-send": {
      const p = params as unknown as SimpleSendParams;
      // Frame 0: VERIFY (empty data, filled after signing)
      // Frame 1: SENDER - transfer call on the account
      const transferCalldata = encodeTransferCalldata(p.to, p.amount);
      frames = [
        {
          mode: FRAME_MODE_VERIFY,
          target: sender,
          gasLimit: DEFAULT_FRAME_GAS,
          data: new Uint8Array(0),
        },
        {
          mode: FRAME_MODE_SENDER,
          target: sender,
          gasLimit: DEFAULT_FRAME_GAS,
          data: transferCalldata,
        },
      ];
      break;
    }

    case "sponsored-send": {
      const p = params as unknown as SponsoredSendParams;
      const sponsorAddr = p.sponsorAddress?.toLowerCase() ?? SPONSOR_ADDRESS;
      // ERC20 transfer: account calls execute(erc20, 0, transfer(to, amount))
      const erc20TransferData = encodeTransferCalldata(p.to, p.amount);
      const erc20Hex = "0x" + Array.from(erc20TransferData).map(b => b.toString(16).padStart(2, "0")).join("");
      const executeCalldata = encodeExecuteCalldata(
        MOCK_ERC20_ADDRESS,
        "0",
        erc20Hex
      );
      // Frame 0: VERIFY sender (scope=0, account proves identity)
      // Frame 1: VERIFY payer (scope=1, sponsor approves gas payment)
      // Frame 2: SENDER execute (ERC20 transfer)
      frames = [
        {
          mode: FRAME_MODE_VERIFY,
          target: sender,
          gasLimit: DEFAULT_FRAME_GAS,
          data: new Uint8Array(0),
        },
        {
          mode: FRAME_MODE_VERIFY,
          target: hexToBytes(sponsorAddr),
          gasLimit: DEFAULT_FRAME_GAS,
          data: new Uint8Array(0),
        },
        {
          mode: FRAME_MODE_SENDER,
          target: sender,
          gasLimit: DEFAULT_FRAME_GAS,
          data: executeCalldata,
        },
      ];
      break;
    }

    case "batch-ops": {
      const p = params as unknown as BatchOpsParams;
      // Frame 0: VERIFY
      // Frame 1..N: SENDER for each operation
      frames = [
        {
          mode: FRAME_MODE_VERIFY,
          target: sender,
          gasLimit: DEFAULT_FRAME_GAS,
          data: new Uint8Array(0),
        },
      ];
      for (const op of p.operations) {
        const executeCalldata = encodeExecuteCalldata(
          op.to,
          op.value,
          op.data
        );
        frames.push({
          mode: FRAME_MODE_SENDER,
          target: sender,
          gasLimit: DEFAULT_FRAME_GAS,
          data: executeCalldata,
        });
      }
      break;
    }

    case "deploy-execute": {
      const p = params as unknown as DeployExecuteParams;
      // Frame 0: VERIFY
      // Frame 1: DEFAULT (mode=0, null target = CREATE) with bytecode
      // Frame 2: SENDER (CALL the deployed contract - address TBD by caller)
      frames = [
        {
          mode: FRAME_MODE_VERIFY,
          target: sender,
          gasLimit: DEFAULT_FRAME_GAS,
          data: new Uint8Array(0),
        },
        {
          mode: FRAME_MODE_DEFAULT,
          target: new Uint8Array(0), // null target = CREATE
          gasLimit: 1_000_000n,
          data: hexToBytes(p.bytecode),
        },
        {
          mode: FRAME_MODE_SENDER,
          target: sender,
          gasLimit: DEFAULT_FRAME_GAS,
          data: hexToBytes(p.calldata),
        },
      ];
      break;
    }

    default:
      throw new Error(`Unknown demo type: ${demoType}`);
  }

  return new FrameTransaction({
    chainId,
    nonce,
    sender,
    frames,
    maxPriorityFeePerGas,
    maxFeePerGas,
    maxFeePerBlobGas: 0n,
    blobVersionedHashes: [],
  });
}

function encodeTransferCalldata(to: string, amount: string): Uint8Array {
  const encoded = encodeAbiParameters(
    parseAbiParameters("address to, uint256 amount"),
    [to as `0x${string}`, parseEther(amount)]
  );
  // transfer(address,uint256) selector: 0xa9059cbb
  const selector = hexToBytes("0xa9059cbb");
  const params = hexToBytes(encoded);
  const result = new Uint8Array(4 + params.length);
  result.set(selector, 0);
  result.set(params, 4);
  return result;
}

function encodeExecuteCalldata(
  to: string,
  value: string,
  data: string
): Uint8Array {
  const encoded = encodeAbiParameters(
    parseAbiParameters("address to, uint256 value, bytes data"),
    [to as `0x${string}`, parseEther(value), (data || "0x") as `0x${string}`]
  );
  // execute(address,uint256,bytes) selector: 0xb61d27f6
  const selector = hexToBytes("0xb61d27f6");
  const params = hexToBytes(encoded);
  const result = new Uint8Array(4 + params.length);
  result.set(selector, 0);
  result.set(params, 4);
  return result;
}

// Serialize a FrameTransaction to a JSON-friendly format for the frontend
function serializeTxSkeleton(tx: FrameTransaction): Record<string, unknown> {
  return {
    chainId: tx.params.chainId.toString(),
    nonce: tx.params.nonce.toString(),
    sender: bytesToHex(tx.params.sender),
    frames: tx.params.frames.map((f) => ({
      mode: f.mode,
      target: bytesToHex(f.target),
      gasLimit: f.gasLimit.toString(),
      data: bytesToHex(f.data),
    })),
    maxPriorityFeePerGas: tx.params.maxPriorityFeePerGas.toString(),
    maxFeePerGas: tx.params.maxFeePerGas.toString(),
    maxFeePerBlobGas: (tx.params.maxFeePerBlobGas ?? 0n).toString(),
    blobVersionedHashes: (tx.params.blobVersionedHashes ?? []).map(bytesToHex),
  };
}

app.post("/sig-hash", async (c) => {
  try {
    const body = (await c.req.json()) as SigHashRequest;
    const tx = await buildSkeleton(
      body.demoType,
      body.params as unknown as Record<string, unknown>
    );

    const sigHash = tx.computeSigHash();
    const from = ((body.params as any).from as string).toLowerCase();
    pendingSkeletons.set(from, tx);
    console.log(
      `[sig-hash] Built ${body.demoType} skeleton, sigHash=${bytesToHex(sigHash)}`
    );

    return c.json({
      sigHash: bytesToHex(sigHash),
      txSkeleton: serializeTxSkeleton(tx),
    });
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error(`[sig-hash] Error: ${msg}`);
    return c.json({ error: msg }, 500);
  }
});

// Cache skeletons by address so submit endpoints reuse the exact same tx
// (avoids nonce/baseFee drift between sig-hash and submit calls).
export const pendingSkeletons = new Map<string, FrameTransaction>();

export { buildSkeleton, serializeTxSkeleton };
export default app;
