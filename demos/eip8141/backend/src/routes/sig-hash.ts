import { Hono } from "hono";
import { FrameTransaction, bytesToHex, hexToBytes } from "../frame-tx.js";
import { getChainId, getBaseFee, getNonce } from "../rpc.js";
import {
  FRAME_MODE_VERIFY,
  FRAME_MODE_SENDER,
  FRAME_MODE_DEFAULT,
  SPONSOR_ADDRESS,
  MOCK_ERC20_ADDRESS,
  DEPLOYER_PROXY_ADDRESS,
} from "../types.js";
import type {
  SigHashRequest,
  SimpleSendParams,
  SponsoredSendParams,
  BatchOpsParams,
  DeployExecuteParams,
  Frame,
  AuthMethod,
} from "../types.js";
import {
  encodeAbiParameters,
  parseAbiParameters,
  parseEther,
  encodeFunctionData,
  getCreate2Address,
  numberToHex,
  keccak256,
  encodePacked,
} from "viem";
import { ephemeralAccounts, deriveKey } from "../ephemeral-state.js";

// Default gas limit per frame
const DEFAULT_FRAME_GAS = 200_000n;

const app = new Hono();

/**
 * Build rotation SENDER frame: account.execute(address(this), 0, rotate(nextKey))
 */
function buildRotationFrame(sender: Uint8Array, nextSignerAddress: string): Frame {
  const rotateCalldata = encodeFunctionData({
    abi: [{
      type: "function",
      name: "rotate",
      inputs: [{ name: "newSigner", type: "address" }],
      outputs: [],
      stateMutability: "nonpayable",
    }],
    functionName: "rotate",
    args: [nextSignerAddress as `0x${string}`],
  });
  const senderHex = bytesToHex(sender);
  const execCalldata = encodeExecuteCalldata(senderHex, "0", rotateCalldata);
  return {
    mode: FRAME_MODE_SENDER,
    flags: 0x00,
    target: sender,
    gasLimit: DEFAULT_FRAME_GAS,
    data: execCalldata,
  };
}

/**
 * Build a Frame TX skeleton (VERIFY data empty) based on demo type.
 * When authMethod is "ephemeral", a rotation SENDER frame is inserted
 * after VERIFY and before the operation frames.
 */
async function buildSkeleton(
  demoType: string,
  params: Record<string, unknown>,
  authMethod: AuthMethod = "passkey",
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

  // For ephemeral auth, pre-compute the rotation frame
  let rotationFrame: Frame | null = null;
  if (authMethod === "ephemeral") {
    const state = ephemeralAccounts.get(from);
    if (!state) throw new Error(`No ephemeral state for ${from}`);
    const nextKey = deriveKey(state.seed, state.keyIndex + 1);
    rotationFrame = buildRotationFrame(sender, nextKey.address);
  }

  let frames: Frame[];

  switch (demoType) {
    case "simple-send": {
      const p = params as unknown as SimpleSendParams;
      const transferCalldata = encodeTransferCalldata(p.to, p.amount);
      frames = [
        {
          mode: FRAME_MODE_VERIFY,
          flags: 0x03, // PAYMENT+EXECUTION (verifyAndPay / verifyEcdsaAndPay)
          target: sender,
          gasLimit: DEFAULT_FRAME_GAS,
          data: new Uint8Array(0),
        },
      ];
      if (rotationFrame) frames.push(rotationFrame);
      frames.push({
        mode: FRAME_MODE_SENDER,
        flags: 0x00,
        target: sender,
        gasLimit: DEFAULT_FRAME_GAS,
        data: transferCalldata,
      });
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
      frames = [
        {
          mode: FRAME_MODE_VERIFY,
          flags: 0x02, // EXECUTION only (verify / verifyEcdsa, sender-only)
          target: sender,
          gasLimit: DEFAULT_FRAME_GAS,
          data: new Uint8Array(0),
        },
        {
          mode: FRAME_MODE_VERIFY,
          flags: 0x01, // PAYMENT only (GasSponsor.verify, payer-only)
          target: hexToBytes(sponsorAddr),
          gasLimit: DEFAULT_FRAME_GAS,
          data: new Uint8Array(0),
        },
      ];
      if (rotationFrame) frames.push(rotationFrame);
      frames.push({
        mode: FRAME_MODE_SENDER,
        flags: 0x00,
        target: sender,
        gasLimit: DEFAULT_FRAME_GAS,
        data: executeCalldata,
      });
      break;
    }

    case "batch-ops": {
      const p = params as unknown as BatchOpsParams;
      frames = [
        {
          mode: FRAME_MODE_VERIFY,
          flags: 0x03, // PAYMENT+EXECUTION (verifyAndPay / verifyEcdsaAndPay)
          target: sender,
          gasLimit: DEFAULT_FRAME_GAS,
          data: new Uint8Array(0),
        },
      ];
      if (rotationFrame) frames.push(rotationFrame);
      for (const op of p.operations) {
        const executeCalldata = encodeExecuteCalldata(
          op.to,
          op.value,
          op.data
        );
        frames.push({
          mode: FRAME_MODE_SENDER,
          flags: 0x00,
          target: sender,
          gasLimit: DEFAULT_FRAME_GAS,
          data: executeCalldata,
        });
      }
      break;
    }

    case "deploy-execute": {
      const p = params as unknown as DeployExecuteParams;
      const initCodeHex = (
        p.bytecode.startsWith("0x") ? p.bytecode : `0x${p.bytecode}`
      ) as `0x${string}`;
      const initCode = hexToBytes(initCodeHex);

      // Salt = keccak256(sender || nonce) to avoid collisions across accounts
      const saltHex = keccak256(encodePacked(
        ['address', 'uint256'],
        [from as `0x${string}`, nonce],
      ));
      const salt = hexToBytes(saltHex);

      // Calldata for deployer proxy: 32-byte salt + init code
      const deployData = new Uint8Array(32 + initCode.length);
      deployData.set(salt, 0);
      deployData.set(initCode, 32);

      // Compute deterministic CREATE2 deployed address
      const deployedAddress = getCreate2Address({
        from: DEPLOYER_PROXY_ADDRESS as `0x${string}`,
        salt: saltHex,
        bytecode: initCodeHex,
      });

      const executeCalldata = encodeExecuteCalldata(
        deployedAddress,
        "0",
        p.calldata || "0x"
      );

      frames = [
        {
          mode: FRAME_MODE_VERIFY,
          flags: 0x03, // PAYMENT+EXECUTION (verifyAndPay / verifyEcdsaAndPay)
          target: sender,
          gasLimit: DEFAULT_FRAME_GAS,
          data: new Uint8Array(0),
        },
      ];
      if (rotationFrame) frames.push(rotationFrame);
      frames.push(
        {
          mode: FRAME_MODE_DEFAULT,
          flags: 0x00,
          target: hexToBytes(DEPLOYER_PROXY_ADDRESS),
          gasLimit: 1_000_000n,
          data: deployData,
        },
        {
          mode: FRAME_MODE_SENDER,
          flags: 0x00,
          target: sender,
          gasLimit: DEFAULT_FRAME_GAS,
          data: executeCalldata,
        },
      );

      // Store deployed address for the response
      pendingDeployedAddresses.set(from, deployedAddress);
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
      flags: f.flags,
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
      body.params as unknown as Record<string, unknown>,
      body.authMethod ?? "passkey",
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

// Cache deployed addresses for deploy-execute responses.
export const pendingDeployedAddresses = new Map<string, string>();

export { buildSkeleton, serializeTxSkeleton };
export default app;
