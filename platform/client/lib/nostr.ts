/**
 * Nostr client for Tokamak Appchain Showroom social features.
 *
 * Custom Event Kinds:
 *   30100 — Appchain Review (replaceable, keyed by chainId)
 *   30101 — Appchain Comment (replaceable, keyed by chainId)
 *       7 — Reaction (like/dislike on reviews)
 *
 * All events are namespaced with ["L", "tokamak-appchain"].
 */

import {
  SimplePool,
  finalizeEvent,
  generateSecretKey,
  getPublicKey,
  type Event as NostrEvent,
} from "nostr-tools";

const RELAY_URL =
  process.env.NEXT_PUBLIC_NOSTR_RELAY || "wss://relay.tokamak.network";

const NAMESPACE_TAG: [string, string] = ["L", "tokamak-appchain"];

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
  rating: number;
  content: string;
  createdAt: number;
}

export interface Comment {
  id: string;
  pubkey: string;
  content: string;
  parentId: string | null;
  createdAt: number;
}

export interface NostrKeys {
  sk: Uint8Array;
  pk: string;
}

// ── Key Management ──

const STORAGE_KEY = "nostr_sk";

/** Get or create Nostr keypair. Stored in localStorage. */
export function getOrCreateNostrKeys(): NostrKeys {
  if (typeof window === "undefined") {
    // SSR fallback — generate ephemeral
    const sk = generateSecretKey();
    return { sk, pk: getPublicKey(sk) };
  }

  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored) {
    try {
      const sk = new Uint8Array(JSON.parse(stored));
      return { sk, pk: getPublicKey(sk) };
    } catch {
      // Corrupted — regenerate
      localStorage.removeItem(STORAGE_KEY);
    }
  }

  const sk = generateSecretKey();
  localStorage.setItem(STORAGE_KEY, JSON.stringify(Array.from(sk)));
  return { sk, pk: getPublicKey(sk) };
}

/** Check if user has Nostr keys configured. */
export function hasNostrKeys(): boolean {
  if (typeof window === "undefined") return false;
  return !!localStorage.getItem(STORAGE_KEY);
}

// ── Query Functions ──

/** Fetch reviews for an appchain by chainId. */
export async function getAppchainReviews(
  chainId: string
): Promise<Review[]> {
  const pool = getPool();
  const events = await pool.querySync([RELAY_URL], {
    kinds: [30100],
    "#d": [chainId],
    "#L": ["tokamak-appchain"],
  });

  return events
    .map((e: NostrEvent) => ({
      id: e.id,
      pubkey: e.pubkey,
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
  const events = await pool.querySync([RELAY_URL], {
    kinds: [30101],
    "#d": [chainId],
    "#L": ["tokamak-appchain"],
  });

  return events
    .map((e: NostrEvent) => ({
      id: e.id,
      pubkey: e.pubkey,
      content: e.content,
      parentId: e.tags.find((t: string[]) => t[0] === "e")?.[1] || null,
      createdAt: e.created_at,
    }))
    .sort((a: Comment, b: Comment) => b.createdAt - a.createdAt);
}

/** Count reactions (likes) for a specific event. */
export async function getReactionCount(eventId: string): Promise<number> {
  const pool = getPool();
  const events = await pool.querySync([RELAY_URL], {
    kinds: [7],
    "#e": [eventId],
  });
  return events.filter((e: NostrEvent) => e.content === "+").length;
}

/** Batch-fetch reaction counts for multiple event IDs. */
export async function getReactionCounts(
  eventIds: string[]
): Promise<Record<string, number>> {
  if (eventIds.length === 0) return {};
  const pool = getPool();
  const events = await pool.querySync([RELAY_URL], {
    kinds: [7],
    "#e": eventIds,
  });

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

/** Publish a review for an appchain. */
export async function publishReview(
  secretKey: Uint8Array,
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
      ],
      content,
    },
    secretKey
  );
  await pool.publish([RELAY_URL], event);
  return event;
}

/** Publish a comment on an appchain or reply to another event. */
export async function publishComment(
  secretKey: Uint8Array,
  chainId: string,
  content: string,
  parentEventId?: string
): Promise<NostrEvent> {
  const pool = getPool();
  const tags: string[][] = [["d", chainId], NAMESPACE_TAG];
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
    secretKey
  );
  await pool.publish([RELAY_URL], event);
  return event;
}

/** Publish a like reaction on a review or comment. */
export async function publishReaction(
  secretKey: Uint8Array,
  targetEventId: string
): Promise<NostrEvent> {
  const pool = getPool();
  const event = finalizeEvent(
    {
      kind: 7,
      created_at: Math.floor(Date.now() / 1000),
      tags: [["e", targetEventId], NAMESPACE_TAG],
      content: "+",
    },
    secretKey
  );
  await pool.publish([RELAY_URL], event);
  return event;
}

// ── Helpers ──

/** Shorten a Nostr public key for display. */
export function shortenPubkey(pk: string): string {
  return `${pk.slice(0, 8)}...${pk.slice(-4)}`;
}

/** Get relay URL (for debugging/display). */
export function getRelayUrl(): string {
  return RELAY_URL;
}
