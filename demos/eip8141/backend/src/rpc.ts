import { bytesToHex } from "./frame-tx.js";

const RPC_URL = process.env.RPC_URL ?? "http://localhost:8545";

let rpcId = 0;

export async function rpcCall(
  method: string,
  params: unknown[]
): Promise<unknown> {
  const id = ++rpcId;
  const res = await fetch(RPC_URL, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", method, params, id }),
  });
  const json = (await res.json()) as {
    result?: unknown;
    error?: { code: number; message: string };
  };
  if (json.error) {
    throw new Error(`RPC error ${json.error.code}: ${json.error.message}`);
  }
  return json.result;
}

export async function getChainId(): Promise<bigint> {
  const result = (await rpcCall("eth_chainId", [])) as string;
  return BigInt(result);
}

export async function getBaseFee(): Promise<bigint> {
  const block = (await rpcCall("eth_getBlockByNumber", [
    "latest",
    false,
  ])) as { baseFeePerGas: string };
  return BigInt(block.baseFeePerGas);
}

export async function getNonce(address: string): Promise<bigint> {
  const result = (await rpcCall("eth_getTransactionCount", [
    address,
    "latest",
  ])) as string;
  return BigInt(result);
}

export async function getBalance(address: string): Promise<bigint> {
  const result = (await rpcCall("eth_getBalance", [
    address,
    "latest",
  ])) as string;
  return BigInt(result);
}

export async function sendRawTransaction(rawTx: Uint8Array): Promise<string> {
  const hex = bytesToHex(rawTx);
  return (await rpcCall("eth_sendRawTransaction", [hex])) as string;
}

export async function getTransactionReceipt(
  txHash: string
): Promise<Record<string, unknown> | null> {
  return (await rpcCall("eth_getTransactionReceipt", [
    txHash,
  ])) as Record<string, unknown> | null;
}

export async function waitForReceipt(
  txHash: string,
  timeoutMs = 60_000
): Promise<Record<string, unknown>> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const receipt = await getTransactionReceipt(txHash);
    if (receipt !== null) return receipt;
    await new Promise((r) => setTimeout(r, 1000));
  }
  throw new Error(`Timed out waiting for receipt of ${txHash}`);
}

const FRAME_MODE_LABELS: Record<number, string> = {
  0: "DEFAULT",
  1: "VERIFY",
  2: "SENDER",
};

/**
 * Build the JSON response from a receipt, including per-frame status.
 * `success` is true only if ALL frames succeeded.
 */
export function buildTxResponse(
  receipt: Record<string, unknown>,
  submittedHash: string,
  frameModesInOrder: number[]
): Record<string, unknown> {
  const rawFrameReceipts = receipt.frameReceipts as
    | Array<{ status: string; gasUsed: string }>
    | undefined;

  const frameReceipts = rawFrameReceipts?.map((fr, i) => ({
    mode: FRAME_MODE_LABELS[frameModesInOrder[i]] ?? `FRAME ${i}`,
    status: fr.status === "0x1",
    gasUsed: fr.gasUsed,
  }));

  const allFramesSucceeded =
    frameReceipts?.every((fr) => fr.status) ?? receipt.status === "0x1";

  return {
    success: allFramesSucceeded,
    txHash: submittedHash,
    gasUsed: receipt.gasUsed,
    frameReceipts,
  };
}
