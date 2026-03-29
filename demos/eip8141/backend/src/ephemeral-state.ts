import { keccak256, encodePacked } from "viem";
import { privateKeyToAccount } from "viem/accounts";

// ── Key derivation ──

export function deriveKey(seed: `0x${string}`, index: number): { privateKey: `0x${string}`; address: `0x${string}` } {
  const privKey = keccak256(
    encodePacked(["bytes32", "uint256"], [seed, BigInt(index)])
  );
  const account = privateKeyToAccount(privKey);
  return { privateKey: privKey, address: account.address.toLowerCase() as `0x${string}` };
}

// ── Per-account ephemeral state ──

export interface EphemeralState {
  seed: `0x${string}`;
  keyIndex: number;
  address: `0x${string}`;
}

export const ephemeralAccounts = new Map<string, EphemeralState>();
