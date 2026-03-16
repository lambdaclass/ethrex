/**
 * Server-side wallet signature verification for social features.
 * Uses ethers.verifyMessage for EIP-191 signature recovery.
 */
import { NextRequest } from "next/server";

const CHALLENGE_MESSAGE =
  "Sign in to Tokamak Appchain Showroom\n\nDomain: platform.tokamak.network\nPurpose: Social interaction authentication\n\nThis signature proves you own this wallet.";

// LRU cache for verified signatures (avoid repeated ecrecover)
const verifiedCache = new Map<string, string>();
const CACHE_MAX = 1000;

let _verifyMessage: ((message: string, signature: string) => string) | null = null;

function getVerifyMessage() {
  if (!_verifyMessage) {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    _verifyMessage = require("ethers").verifyMessage as (message: string, signature: string) => string;
  }
  return _verifyMessage;
}

export function verifyWalletSignature(signature: string, claimedAddress: string): string {
  const cacheKey = `${signature}:${claimedAddress.toLowerCase()}`;
  if (verifiedCache.has(cacheKey)) return verifiedCache.get(cacheKey)!;

  const recovered = getVerifyMessage()(CHALLENGE_MESSAGE, signature);
  if (recovered.toLowerCase() !== claimedAddress.toLowerCase()) {
    throw new Error("Signature does not match claimed address");
  }

  const addr = recovered.toLowerCase();

  // Evict oldest if cache full
  if (verifiedCache.size >= CACHE_MAX) {
    const first = verifiedCache.keys().next().value;
    if (first) verifiedCache.delete(first);
  }
  verifiedCache.set(cacheKey, addr);
  return addr;
}

/**
 * Extract and verify wallet from request headers.
 * Returns lowercase wallet address or null.
 */
export function getWalletAddress(req: NextRequest): string | null {
  const address = req.headers.get("x-wallet-address");
  const signature = req.headers.get("x-wallet-signature");
  if (!address || !signature) return null;
  return verifyWalletSignature(signature, address);
}

/**
 * Require wallet authentication. Returns wallet address or throws a Response.
 */
export function requireWallet(req: NextRequest): string {
  const address = req.headers.get("x-wallet-address");
  const signature = req.headers.get("x-wallet-signature");
  if (!address || !signature) {
    throw new Response(JSON.stringify({ error: "Wallet authentication required" }), {
      status: 401,
      headers: { "Content-Type": "application/json" },
    });
  }
  try {
    return verifyWalletSignature(signature, address);
  } catch {
    throw new Response(JSON.stringify({ error: "Invalid wallet signature" }), {
      status: 401,
      headers: { "Content-Type": "application/json" },
    });
  }
}
