import { Hono } from "hono";
import {
  encodeFunctionData,
  parseEther,
  encodeAbiParameters,
  parseAbiParameters,
  toHex,
} from "viem";
import { sign } from "viem/accounts";
import { FrameTransaction, bytesToHex, hexToBytes } from "../frame-tx.js";
import { sendRawTransaction, waitForReceipt, buildTxResponse, getChainId, getBaseFee, getNonce } from "../rpc.js";
import {
  FRAME_MODE_VERIFY,
  FRAME_MODE_SENDER,
  SIGNER_REGISTRY_ADDRESS,
} from "../types.js";
import type { Frame } from "../types.js";
import { ephemeralAccounts, deriveKey } from "./ephemeral-register.js";

const DEFAULT_FRAME_GAS = 200_000n;

const app = new Hono();

// verifyAndPay(uint8,bytes32,bytes32) selector = 0x450beed2
const VERIFY_AND_PAY_SELECTOR = "0x450beed2";

function encodeVerifyCalldata(v: number, r: `0x${string}`, s: `0x${string}`): Uint8Array {
  const encoded = encodeAbiParameters(
    parseAbiParameters("uint8 v, bytes32 r, bytes32 s"),
    [v, r, s]
  );
  const selectorBytes = hexToBytes(VERIFY_AND_PAY_SELECTOR);
  const paramsBytes = hexToBytes(encoded);
  const result = new Uint8Array(4 + paramsBytes.length);
  result.set(selectorBytes, 0);
  result.set(paramsBytes, 4);
  return result;
}

function encodeRotateCalldata(nextSigner: string): Uint8Array {
  const encoded = encodeFunctionData({
    abi: [{
      type: "function",
      name: "rotate",
      inputs: [{ name: "nextSigner", type: "address" }],
      outputs: [],
      stateMutability: "nonpayable",
    }],
    functionName: "rotate",
    args: [nextSigner as `0x${string}`],
  });
  return hexToBytes(encoded);
}

function encodeExecuteCalldata(to: string, value: string, data: string): Uint8Array {
  const encoded = encodeAbiParameters(
    parseAbiParameters("address to, uint256 value, bytes data"),
    [to as `0x${string}`, parseEther(value), (data || "0x") as `0x${string}`]
  );
  const selector = hexToBytes("0xb61d27f6"); // execute(address,uint256,bytes)
  const params = hexToBytes(encoded);
  const result = new Uint8Array(4 + params.length);
  result.set(selector, 0);
  result.set(params, 4);
  return result;
}

function encodeTransferCalldata(to: string, amount: string): Uint8Array {
  const encoded = encodeAbiParameters(
    parseAbiParameters("address to, uint256 amount"),
    [to as `0x${string}`, parseEther(amount)]
  );
  const selector = hexToBytes("0xa9059cbb"); // transfer(address,uint256)
  const params = hexToBytes(encoded);
  const result = new Uint8Array(4 + params.length);
  result.set(selector, 0);
  result.set(params, 4);
  return result;
}

interface EphemeralSendRequest {
  address: string;
  to: string;
  amount: string;
}

app.post("/ephemeral-send", async (c) => {
  try {
    const body = (await c.req.json()) as EphemeralSendRequest;
    const address = body.address.toLowerCase();

    const state = ephemeralAccounts.get(address);
    if (!state) {
      return c.json({ error: `No ephemeral account registered for ${address}` }, 400);
    }

    // Derive current key and next key
    const currentKey = deriveKey(state.seed, state.keyIndex);
    const nextKey = deriveKey(state.seed, state.keyIndex + 1);

    console.log(`[ephemeral-send] Current signer: ${currentKey.address} (index ${state.keyIndex})`);
    console.log(`[ephemeral-send] Next signer: ${nextKey.address} (index ${state.keyIndex + 1})`);

    const sender = hexToBytes(address);

    // Fetch chain params
    const [chainId, baseFee, nonce] = await Promise.all([
      getChainId(),
      getBaseFee(),
      getNonce(address),
    ]);

    const maxPriorityFeePerGas = 1_000_000_000n;
    const maxFeePerGas = baseFee * 2n + maxPriorityFeePerGas;

    // Build rotate calldata: account.execute(registry, 0, rotate(nextKey))
    const rotateData = bytesToHex(encodeRotateCalldata(nextKey.address));
    const rotateExecuteCalldata = encodeExecuteCalldata(
      SIGNER_REGISTRY_ADDRESS,
      "0",
      rotateData
    );

    // Build transfer calldata: account.transfer(to, amount)
    const transferCalldata = encodeTransferCalldata(body.to, body.amount);

    // Frame 0: VERIFY → account (verifyAndPay, empty data for now)
    // Frame 1: SENDER → account (execute: rotate signer)
    // Frame 2: SENDER → account (transfer ETH)
    const frames: Frame[] = [
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
        data: rotateExecuteCalldata,
      },
      {
        mode: FRAME_MODE_SENDER,
        target: sender,
        gasLimit: DEFAULT_FRAME_GAS,
        data: transferCalldata,
      },
    ];

    const tx = new FrameTransaction({
      chainId,
      nonce,
      sender,
      frames,
      maxPriorityFeePerGas,
      maxFeePerGas,
      maxFeePerBlobGas: 0n,
      blobVersionedHashes: [],
    });

    // Compute sig_hash (VERIFY data elided)
    const sigHash = tx.computeSigHash();
    const sigHashHex = bytesToHex(sigHash);
    console.log(`[ephemeral-send] sigHash=${sigHashHex}`);

    // Sign with current ephemeral key (raw ECDSA, no Ethereum prefix)
    const signature = await sign({
      hash: sigHashHex,
      privateKey: currentKey.privateKey,
    });

    console.log(`[ephemeral-send] v=${signature.v}, r=${signature.r}, s=${signature.s}`);

    // Encode VERIFY frame data: verifyAndPay(v, r, s)
    const v = Number(signature.v);
    const verifyCalldata = encodeVerifyCalldata(
      v,
      signature.r as `0x${string}`,
      signature.s as `0x${string}`
    );

    // Fill in VERIFY frame
    tx.setVerifyFrameData(0, verifyCalldata);

    // Encode and send
    const rawTx = tx.encodeCanonical();
    const txHashBytes = tx.txHash();
    const txHashHex = bytesToHex(txHashBytes);
    console.log(`[ephemeral-send] Sending tx ${txHashHex}`);

    const submittedHash = await sendRawTransaction(rawTx);
    console.log(`[ephemeral-send] Submitted: ${submittedHash}`);

    const receipt = await waitForReceipt(submittedHash);
    console.log(`[ephemeral-send] Receipt: status=${receipt.status}, gasUsed=${receipt.gasUsed}`);

    // Increment key index
    state.keyIndex += 1;

    // Build response with key rotation info
    const baseResponse = buildTxResponse(receipt, submittedHash, [1, 2, 2]); // VERIFY, SENDER, SENDER
    return c.json({
      ...baseResponse,
      oldSigner: currentKey.address,
      newSigner: nextKey.address,
      keyIndex: state.keyIndex,
    });
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error(`[ephemeral-send] Error: ${msg}`);
    return c.json({ error: msg }, 500);
  }
});

export default app;
