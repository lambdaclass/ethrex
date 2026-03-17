/**
 * Daily token limiter for Tokamak AI proxy.
 * Tracks per-user usage with persistent storage:
 *   1. Upstash Redis (if configured)
 *   2. Neon Postgres (fallback — always available on Vercel)
 *
 * Env:
 *   UPSTASH_REDIS_REST_URL, UPSTASH_REDIS_REST_TOKEN — optional
 *   DATABASE_URL — Postgres (required, already used by auth/db)
 *   TOKAMAK_AI_DAILY_LIMIT — default daily token limit (default: 50000)
 */

import { sql } from "./db";

export function getDefaultDailyLimit(): number {
  return parseInt(process.env.TOKAMAK_AI_DAILY_LIMIT || "50000", 10);
}

function todayDate(): string {
  return new Date().toISOString().slice(0, 10); // YYYY-MM-DD UTC
}

export interface TokenUsage {
  used: number;
  limit: number;
  remaining: number;
}

export async function getUsage(
  userId: string,
  limit: number
): Promise<TokenUsage> {
  const kv = await getKV();
  const key = `ai:usage:${userId}:${todayDate()}`;
  const used = ((await kv.get(key)) as number) || 0;
  return {
    used,
    limit,
    remaining: Math.max(0, limit - used),
  };
}

export async function checkLimit(
  userId: string,
  limit: number
): Promise<void> {
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
  const key = `ai:usage:${userId}:${todayDate()}`;
  const newTotal = await kv.incrby(key, tokens);
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

// ---- KV abstraction ----

let kvInstance: KVLike | null = null;

interface KVLike {
  get(key: string): Promise<unknown>;
  incrby(key: string, value: number): Promise<number>;
}

async function getKV(): Promise<KVLike> {
  if (kvInstance) return kvInstance;

  // Option 1: Upstash Redis (fastest)
  if (
    process.env.UPSTASH_REDIS_REST_URL &&
    process.env.UPSTASH_REDIS_REST_TOKEN
  ) {
    const { Redis } = await import("@upstash/redis");
    const redis = new Redis({
      url: process.env.UPSTASH_REDIS_REST_URL,
      token: process.env.UPSTASH_REDIS_REST_TOKEN,
    });
    kvInstance = {
      get: (key) => redis.get(key),
      incrby: (key, value) => redis.incrby(key, value),
    };
    return kvInstance;
  }

  // Option 2: Postgres (always available on Vercel via Neon)
  await ensureUsageTable();
  kvInstance = {
    async get(key: string): Promise<unknown> {
      const { rows } = await sql`
        SELECT value FROM ai_usage WHERE key = ${key}
      `;
      return rows.length > 0 ? Number(rows[0].value) : null;
    },
    async incrby(key: string, value: number): Promise<number> {
      const { rows } = await sql`
        INSERT INTO ai_usage (key, value) VALUES (${key}, ${value})
        ON CONFLICT (key) DO UPDATE SET value = ai_usage.value + ${value}
        RETURNING value
      `;
      return Number(rows[0].value);
    },
  };
  return kvInstance;
}

let usageTableReady = false;

async function ensureUsageTable() {
  if (usageTableReady) return;
  await sql`
    CREATE TABLE IF NOT EXISTS ai_usage (
      key TEXT PRIMARY KEY,
      value INTEGER NOT NULL DEFAULT 0
    )
  `;
  // Cleanup old records (keys older than 7 days)
  const cutoff = new Date(Date.now() - 7 * 24 * 60 * 60 * 1000)
    .toISOString()
    .slice(0, 10);
  await sql`
    DELETE FROM ai_usage WHERE key < ${"ai:usage:0:" + cutoff}
  `;
  usageTableReady = true;
}
