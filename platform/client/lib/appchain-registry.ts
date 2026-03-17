/**
 * Appchain Registry — shared logic for metadata validation, signing, and on-chain verification.
 */
import { ethers } from "ethers";

// RPC URLs by L1 chain ID
const L1_RPC_URLS: Record<number, string> = {
  1: process.env.MAINNET_RPC_URL || "https://ethereum-rpc.publicnode.com",
  11155111: process.env.SEPOLIA_RPC_URL || "https://ethereum-sepolia-rpc.publicnode.com",
  17000: process.env.HOLESKY_RPC_URL || "https://ethereum-holesky-rpc.publicnode.com",
};

// Allow custom L1 RPCs via env: L1_RPC_9=http://54.180.160.159:8545
for (const [key, val] of Object.entries(process.env)) {
  const m = key.match(/^L1_RPC_(\d+)$/);
  if (m && val) L1_RPC_URLS[parseInt(m[1])] = val;
}

const TIMELOCK_ABI = ["function hasRole(bytes32 role, address account) view returns (bool)"];
const SECURITY_COUNCIL_ROLE = ethers.id("SECURITY_COUNCIL");

export const IDENTITY_CONTRACT_FIELD: Record<string, string> = {
  "tokamak-appchain": "Timelock",
};

// Rate limit (in-memory, per-instance — best-effort on serverless; each instance has its own map)
const submitRateLimit = new Map<string, { windowStart: number; count: number }>();
const SUBMIT_RATE_WINDOW = 60 * 60 * 1000;
const SUBMIT_RATE_MAX = 5;

export function checkSubmitRateLimit(ip: string): boolean {
  const now = Date.now();
  if (submitRateLimit.size > 100) {
    for (const [key, rec] of submitRateLimit) {
      if (now - rec.windowStart > SUBMIT_RATE_WINDOW) submitRateLimit.delete(key);
    }
  }
  const record = submitRateLimit.get(ip);
  if (!record || now - record.windowStart > SUBMIT_RATE_WINDOW) {
    submitRateLimit.set(ip, { windowStart: now, count: 1 });
    return true;
  }
  record.count++;
  return record.count <= SUBMIT_RATE_MAX;
}

export function buildSigningMessage(metadata: Record<string, unknown>, operation: string): string {
  const identityField = IDENTITY_CONTRACT_FIELD[metadata.stackType as string];
  if (!identityField) throw new Error(`Unknown stackType: ${metadata.stackType}`);

  const l1Contracts = metadata.l1Contracts as Record<string, string>;
  const identityAddress = l1Contracts[identityField];
  if (!identityAddress) throw new Error(`Missing l1Contracts.${identityField}`);

  const timestamp = operation === "register"
    ? Math.floor(new Date(metadata.createdAt as string).getTime() / 1000)
    : Math.floor(new Date(metadata.lastUpdated as string).getTime() / 1000);

  return [
    "Tokamak Appchain Registry",
    `L1 Chain ID: ${metadata.l1ChainId}`,
    `L2 Chain ID: ${metadata.l2ChainId}`,
    `Stack: ${metadata.stackType}`,
    `Operation: ${operation}`,
    `Contract: ${identityAddress.toLowerCase()}`,
    `Timestamp: ${timestamp}`,
  ].join("\n");
}

export function validateMetadataStructure(metadata: Record<string, unknown>): string[] {
  const errors: string[] = [];
  if (!metadata.l1ChainId) errors.push("Missing l1ChainId");
  if (!metadata.l2ChainId) errors.push("Missing l2ChainId");
  if (!metadata.name) errors.push("Missing name");
  if (!metadata.stackType) errors.push("Missing stackType");
  if (!metadata.rollupType) errors.push("Missing rollupType");
  if (!metadata.rpcUrl) errors.push("Missing rpcUrl");
  if (!metadata.status) errors.push("Missing status");
  if (!metadata.createdAt) errors.push("Missing createdAt");
  if (!metadata.lastUpdated) errors.push("Missing lastUpdated");
  if (!metadata.l1Contracts) errors.push("Missing l1Contracts");

  const operator = metadata.operator as Record<string, unknown> | undefined;
  if (!operator?.address) errors.push("Missing operator.address");

  const meta = metadata.metadata as Record<string, unknown> | undefined;
  if (!meta?.signature) errors.push("Missing metadata.signature");
  if (!meta?.signedBy) errors.push("Missing metadata.signedBy");

  const identityField = IDENTITY_CONTRACT_FIELD[metadata.stackType as string];
  const l1Contracts = metadata.l1Contracts as Record<string, string> | undefined;
  if (identityField && l1Contracts && !l1Contracts[identityField]) {
    errors.push(`Missing l1Contracts.${identityField}`);
  }
  if (l1Contracts && !l1Contracts.OnChainProposer) {
    errors.push("Missing l1Contracts.OnChainProposer");
  }

  if (meta?.signature && (typeof meta.signature !== "string" || !/^0x[a-fA-F0-9]{130}$/.test(meta.signature))) {
    errors.push("Invalid signature format");
  }
  if (meta?.signedBy && typeof meta.signedBy !== "string") {
    errors.push("metadata.signedBy must be a string");
  }

  return errors;
}

export function getRpcUrl(l1ChainId: number, metadataL1RpcUrl?: string): string | null {
  return L1_RPC_URLS[l1ChainId] || metadataL1RpcUrl || null;
}

export async function verifyOnChainOwnership(
  rpcUrl: string,
  timelockAddress: string,
  signerAddress: string,
): Promise<{ valid: boolean; error?: string; rpcError?: boolean }> {
  try {
    const provider = new ethers.JsonRpcProvider(rpcUrl);
    const timelock = new ethers.Contract(timelockAddress, TIMELOCK_ABI, provider);
    const hasRole = await timelock.hasRole(SECURITY_COUNCIL_ROLE, signerAddress);
    if (!hasRole) {
      return { valid: false, error: `Signer ${signerAddress} does not have SECURITY_COUNCIL role on Timelock ${timelockAddress}` };
    }
    return { valid: true };
  } catch (e) {
    return { valid: false, error: `On-chain verification failed: ${(e as Error).message}`, rpcError: true };
  }
}

export function buildPrTitleAndBody(metadata: Record<string, unknown>, operation: string, timelockAddress: string) {
  const prTag = operation === "update" ? "Update" : "Appchain";
  const prTitle = `[${prTag}] ${metadata.l1ChainId}/${metadata.stackType} ${timelockAddress.toLowerCase()} - ${metadata.name}`;
  const l1Contracts = metadata.l1Contracts as Record<string, string>;
  const operator = metadata.operator as Record<string, string>;
  const meta = metadata.metadata as Record<string, string>;
  const prBody = [
    `## ${operation === "register" ? "New" : "Update"} Appchain: ${metadata.name}`,
    "",
    `| Field | Value |`,
    `|---|---|`,
    `| L1 Chain ID | ${metadata.l1ChainId} |`,
    `| L2 Chain ID | ${metadata.l2ChainId} |`,
    `| Stack | ${metadata.stackType} |`,
    `| Rollup Type | ${metadata.rollupType} |`,
    `| Timelock | \`${timelockAddress}\` |`,
    `| OnChainProposer | \`${l1Contracts.OnChainProposer}\` |`,
    `| Operator | \`${operator.address}\` |`,
    `| Signed by | \`${meta.signedBy}\` |`,
    "",
    "---",
    "*Submitted via Tokamak Appchain Messenger*",
  ].join("\n");
  return { prTitle, prBody };
}
