/**
 * Daily token limiter for Tokamak AI proxy.
 * Tracks per-device usage using Upstash Redis.
 *
 * Key format: "ai:usage:{deviceId}:{YYYY-MM-DD}"
 * Value: cumulative token count (number)
 * TTL: 48 hours (auto-cleanup)
 *
 * Env: UPSTASH_REDIS_REST_URL, UPSTASH_REDIS_REST_TOKEN
 */

const DAILY_TOKEN_LIMIT = 50_000;
const TTL_SECONDS = 48 * 60 * 60; // 48h (covers timezone edge cases)

function todayKey(deviceId: string): string {
  const date = new Date().toISOString().slice(0, 10); // YYYY-MM-DD UTC
  return `ai:usage:${deviceId}:${date}`;
}

export interface TokenUsage {
  used: number;
  limit: number;
  remaining: number;
}

export async function getUsage(deviceId: string): Promise<TokenUsage> {
  const kv = await getKV();
  const key = todayKey(deviceId);
  const used = ((await kv.get(key)) as number) || 0;
  return {
    used,
    limit: DAILY_TOKEN_LIMIT,
    remaining: Math.max(0, DAILY_TOKEN_LIMIT - used),
  };
}

export async function checkLimit(deviceId: string): Promise<void> {
  const usage = await getUsage(deviceId);
  if (usage.remaining <= 0) {
    throw new LimitExceededError(usage);
  }
}

export async function recordUsage(
  deviceId: string,
  tokens: number
): Promise<TokenUsage> {
  const kv = await getKV();
  const key = todayKey(deviceId);
  const current = ((await kv.get(key)) as number) || 0;
  const newTotal = current + tokens;
  await kv.set(key, newTotal, { ex: TTL_SECONDS });
  return {
    used: newTotal,
    limit: DAILY_TOKEN_LIMIT,
    remaining: Math.max(0, DAILY_TOKEN_LIMIT - newTotal),
  };
}

export class LimitExceededError extends Error {
  usage: TokenUsage;
  constructor(usage: TokenUsage) {
    super("daily_limit_exceeded");
    this.usage = usage;
  }
}

// ---- Redis connection ----

let kvInstance: KVLike | null = null;

interface KVLike {
  get(key: string): Promise<unknown>;
  set(key: string, value: unknown, opts?: { ex?: number }): Promise<unknown>;
}

async function getKV(): Promise<KVLike> {
  if (kvInstance) return kvInstance;

  // Try Upstash Redis first
  if (process.env.UPSTASH_REDIS_REST_URL && process.env.UPSTASH_REDIS_REST_TOKEN) {
    const { Redis } = await import("@upstash/redis");
    kvInstance = new Redis({
      url: process.env.UPSTASH_REDIS_REST_URL,
      token: process.env.UPSTASH_REDIS_REST_TOKEN,
    });
    return kvInstance;
  }

  // Fallback: in-memory store for local development
  console.warn("[token-limiter] Upstash Redis not configured, using in-memory store");
  const store = new Map<string, { value: unknown; expiry: number }>();
  kvInstance = {
    async get(key: string) {
      const entry = store.get(key);
      if (!entry) return null;
      if (Date.now() > entry.expiry) {
        store.delete(key);
        return null;
      }
      return entry.value;
    },
    async set(key: string, value: unknown, opts?: { ex?: number }) {
      const expiry = Date.now() + (opts?.ex || TTL_SECONDS) * 1000;
      store.set(key, { value, expiry });
    },
  };
  return kvInstance;
}
