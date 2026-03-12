/**
 * Nostr client for Tokamak Appchain Showroom social features.
 *
 * Authentication: EVM wallet signature → deterministic Nostr key derivation.
 * Same wallet always produces the same Nostr keypair.
 *
 * Custom Event Kinds:
 *   30100 — Appchain Review (replaceable, keyed by chainId)
 *   30101 — Appchain Comment (replaceable, keyed by chainId:timestamp)
 *       7 — Reaction (like/dislike on reviews)
 *
 * All events are namespaced with ["L", "tokamak-appchain"].
 */

import {
  SimplePool,
  finalizeEvent,
  getPublicKey,
  type Event as NostrEvent,
} from "nostr-tools";

const RELAY_URL =
  process.env.NEXT_PUBLIC_NOSTR_RELAY || "wss://relay.tokamak.network";

const NAMESPACE_TAG: [string, string] = ["L", "tokamak-appchain"];

const SIGN_MESSAGE =
  "Sign in to Tokamak Appchain Showroom\nThis signature links your wallet to your social identity.";

// Singleton pool
let _pool: SimplePool | null = null;
function getPool(): SimplePool {
  if (!_pool) _pool = new SimplePool();
  return _pool;
}

// ── Types ──

export interface Review {
  id: string;
  pubkey: string;
  walletAddress: string | null;
  rating: number;
  content: string;
  createdAt: number;
}

export interface Comment {
  id: string;
  pubkey: string;
  walletAddress: string | null;
  content: string;
  parentId: string | null;
  createdAt: number;
}

export interface WalletSession {
  sk: Uint8Array;
  pk: string;
  address: string;
}

// ── Wallet-based Key Management ──

const SESSION_KEY = "nostr_session";

interface EthereumProvider {
  request: (args: { method: string; params?: unknown[] }) => Promise<unknown>;
}

function getEthereum(): EthereumProvider | null {
  if (typeof window === "undefined") return null;
  return (window as unknown as { ethereum?: EthereumProvider }).ethereum ?? null;
}

function hexToBytes(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2) {
    bytes[i / 2] = parseInt(hex.substring(i, i + 2), 16);
  }
  return bytes;
}

/** Connect EVM wallet, sign message, derive Nostr keypair. */
export async function connectWallet(): Promise<WalletSession> {
  const ethereum = getEthereum();
  if (!ethereum) throw new Error("No wallet found. Please install MetaMask.");

  const accounts = (await ethereum.request({
    method: "eth_requestAccounts",
  })) as string[];
  const address = accounts[0];

  const signature = (await ethereum.request({
    method: "personal_sign",
    params: [SIGN_MESSAGE, address],
  })) as string;

  // Derive Nostr secret key from first 32 bytes of signature
  const sigBytes = hexToBytes(signature.slice(2)); // remove 0x prefix
  const sk = sigBytes.slice(0, 32);
  const pk = getPublicKey(sk);

  // Cache in sessionStorage (cleared when tab closes)
  const session: WalletSession = { sk, pk, address };
  sessionStorage.setItem(
    SESSION_KEY,
    JSON.stringify({ sk: Array.from(sk), pk, address })
  );

  return session;
}

/** Get cached wallet session (from sessionStorage). */
export function getWalletSession(): WalletSession | null {
  if (typeof window === "undefined") return null;
  const stored = sessionStorage.getItem(SESSION_KEY);
  if (!stored) return null;
  try {
    const parsed = JSON.parse(stored);
    return {
      sk: new Uint8Array(parsed.sk),
      pk: parsed.pk,
      address: parsed.address,
    };
  } catch {
    sessionStorage.removeItem(SESSION_KEY);
    return null;
  }
}

/** Disconnect wallet session. */
export function disconnectWallet(): void {
  if (typeof window !== "undefined") {
    sessionStorage.removeItem(SESSION_KEY);
  }
}

/** Check if browser has an EVM wallet available. */
export function hasWallet(): boolean {
  return getEthereum() !== null;
}

// ── Query Functions ──

/** Wrap a promise with a timeout. Clears timer on resolve to avoid leaks. */
function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout>;
  return Promise.race([
    promise.finally(() => clearTimeout(timer)),
    new Promise<T>((_, reject) => {
      timer = setTimeout(() => reject(new Error("Relay timeout")), ms);
    }),
  ]);
}

const QUERY_TIMEOUT = 10000;

/** Extract wallet address from event tags. */
function extractWallet(e: NostrEvent): string | null {
  return e.tags.find((t: string[]) => t[0] === "wallet")?.[1] || null;
}

/** Fetch reviews for an appchain by chainId. */
export async function getAppchainReviews(
  chainId: string
): Promise<Review[]> {
  const pool = getPool();
  const events = await withTimeout(pool.querySync([RELAY_URL], {
    kinds: [30100],
    "#d": [chainId],
    "#L": ["tokamak-appchain"],
  }), QUERY_TIMEOUT);

  return events
    .map((e: NostrEvent) => ({
      id: e.id,
      pubkey: e.pubkey,
      walletAddress: extractWallet(e),
      rating: parseInt(
        e.tags.find((t: string[]) => t[0] === "rating")?.[1] || "0",
        10
      ),
      content: e.content,
      createdAt: e.created_at,
    }))
    .sort((a: Review, b: Review) => b.createdAt - a.createdAt);
}

/** Fetch comments for an appchain by chainId. */
export async function getAppchainComments(
  chainId: string
): Promise<Comment[]> {
  const pool = getPool();
  const events = await withTimeout(pool.querySync([RELAY_URL], {
    kinds: [30101],
    "#chain": [chainId],
    "#L": ["tokamak-appchain"],
  }), QUERY_TIMEOUT);

  return events
    .map((e: NostrEvent) => ({
      id: e.id,
      pubkey: e.pubkey,
      walletAddress: extractWallet(e),
      content: e.content,
      parentId: e.tags.find((t: string[]) => t[0] === "e")?.[1] || null,
      createdAt: e.created_at,
    }))
    .sort((a: Comment, b: Comment) => b.createdAt - a.createdAt);
}

/** Batch-fetch reaction counts for multiple event IDs. */
export async function getReactionCounts(
  eventIds: string[]
): Promise<Record<string, number>> {
  if (eventIds.length === 0) return {};
  const pool = getPool();
  const events = await withTimeout(pool.querySync([RELAY_URL], {
    kinds: [7],
    "#e": eventIds,
  }), QUERY_TIMEOUT);

  const counts: Record<string, number> = {};
  for (const id of eventIds) counts[id] = 0;
  for (const e of events) {
    if (e.content !== "+") continue;
    const targetId = e.tags.find((t: string[]) => t[0] === "e")?.[1];
    if (targetId && targetId in counts) counts[targetId]++;
  }
  return counts;
}

// ── Publish Functions ──

/** Publish a review for an appchain. Includes wallet address tag. */
export async function publishReview(
  session: WalletSession,
  chainId: string,
  rating: number,
  content: string
): Promise<NostrEvent> {
  const pool = getPool();
  const event = finalizeEvent(
    {
      kind: 30100,
      created_at: Math.floor(Date.now() / 1000),
      tags: [
        ["d", chainId],
        ["rating", Math.min(5, Math.max(1, rating)).toString()],
        NAMESPACE_TAG,
        ["wallet", session.address],
      ],
      content,
    },
    session.sk
  );
  await pool.publish([RELAY_URL], event);
  return event;
}

/** Publish a comment on an appchain or reply to another event. */
export async function publishComment(
  session: WalletSession,
  chainId: string,
  content: string,
  parentEventId?: string
): Promise<NostrEvent> {
  const pool = getPool();
  const uniqueId = `${chainId}:${Date.now()}`;
  const tags: string[][] = [
    ["d", uniqueId],
    ["chain", chainId],
    NAMESPACE_TAG,
    ["wallet", session.address],
  ];
  if (parentEventId) {
    tags.push(["e", parentEventId]);
  }

  const event = finalizeEvent(
    {
      kind: 30101,
      created_at: Math.floor(Date.now() / 1000),
      tags,
      content,
    },
    session.sk
  );
  await pool.publish([RELAY_URL], event);
  return event;
}

/** Publish a like reaction on a review or comment. */
export async function publishReaction(
  session: WalletSession,
  targetEventId: string
): Promise<NostrEvent> {
  const pool = getPool();
  const event = finalizeEvent(
    {
      kind: 7,
      created_at: Math.floor(Date.now() / 1000),
      tags: [["e", targetEventId], NAMESPACE_TAG, ["wallet", session.address]],
      content: "+",
    },
    session.sk
  );
  await pool.publish([RELAY_URL], event);
  return event;
}

// ── Helpers ──

/** Shorten an EVM address for display. */
export function shortenAddress(addr: string): string {
  return `${addr.slice(0, 6)}...${addr.slice(-4)}`;
}

