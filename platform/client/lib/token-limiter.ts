/**
 * Daily token limiter for Tokamak AI proxy.
 * Tracks per-user usage using Upstash Redis.
 *
 * Key format: "ai:usage:{userId}:{YYYY-MM-DD}"
 * Value: cumulative token count (number)
 * TTL: 48 hours (auto-cleanup)
 *
 * Env:
 *   UPSTASH_REDIS_REST_URL, UPSTASH_REDIS_REST_TOKEN
 *   TOKAMAK_AI_DAILY_LIMIT — default daily token limit (default: 50000)
 */

const TTL_SECONDS = 48 * 60 * 60; // 48h (covers timezone edge cases)

export function getDefaultDailyLimit(): number {
  return parseInt(process.env.TOKAMAK_AI_DAILY_LIMIT || "50000", 10);
}

function todayKey(userId: string): string {
  const date = new Date().toISOString().slice(0, 10); // YYYY-MM-DD UTC
  return `ai:usage:${userId}:${date}`;
}

export interface TokenUsage {
  used: number;
  limit: number;
  remaining: number;
}

export async function getUsage(userId: string, limit: number): Promise<TokenUsage> {
  const kv = await getKV();
  const key = todayKey(userId);
  const used = ((await kv.get(key)) as number) || 0;
  return {
    used,
    limit,
    remaining: Math.max(0, limit - used),
  };
}

export async function checkLimit(userId: string, limit: number): Promise<void> {
  const usage = await getUsage(userId, limit);
  if (usage.remaining <= 0) {
    throw new LimitExceededError(usage);
  }
}

export async function recordUsage(
  userId: string,
  tokens: number,
  limit: number
): Promise<TokenUsage> {
  const kv = await getKV();
  const key = todayKey(userId);
  const newTotal = await kv.incrby(key, tokens);
  await kv.expire(key, TTL_SECONDS);
  return {
    used: newTotal,
    limit,
    remaining: Math.max(0, limit - newTotal),
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
  incrby(key: string, value: number): Promise<number>;
  expire(key: string, seconds: number): Promise<unknown>;
}

async function getKV(): Promise<KVLike> {
  if (kvInstance) return kvInstance;

  // Try Upstash Redis first
  if (process.env.UPSTASH_REDIS_REST_URL && process.env.UPSTASH_REDIS_REST_TOKEN) {
    const { Redis } = await import("@upstash/redis");
    const redis = new Redis({
      url: process.env.UPSTASH_REDIS_REST_URL,
      token: process.env.UPSTASH_REDIS_REST_TOKEN,
    });
    kvInstance = {
      get: (key) => redis.get(key),
      set: (key, value, opts) => redis.set(key, value, opts?.ex ? { ex: opts.ex } : undefined),
      incrby: (key, value) => redis.incrby(key, value),
      expire: (key, seconds) => redis.expire(key, seconds),
    };
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
    async incrby(key: string, value: number): Promise<number> {
      const entry = store.get(key);
      const current = (entry && Date.now() <= entry.expiry ? entry.value as number : 0);
      const newTotal = current + value;
      const expiry = entry?.expiry || Date.now() + TTL_SECONDS * 1000;
      store.set(key, { value: newTotal, expiry });
      return newTotal;
    },
    async expire(key: string, seconds: number) {
      const entry = store.get(key);
      if (entry) {
        entry.expiry = Date.now() + seconds * 1000;
      }
    },
  };
  return kvInstance;
}
