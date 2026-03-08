/**
 * Unit tests for lib modules.
 * Run: npx tsx tests/unit.test.ts
 *
 * These tests run without DB/Redis — they test pure logic only.
 */

let passed = 0;
let failed = 0;

function test(name: string, fn: () => void) {
  try {
    fn();
    passed++;
    console.log(`  PASS: ${name}`);
  } catch (e) {
    failed++;
    console.error(`  FAIL: ${name} — ${e}`);
  }
}

function assert(condition: boolean, msg: string) {
  if (!condition) throw new Error(msg);
}

// ---- validate.ts ----
console.log("\n=== lib/validate.ts ===");

import { isValidEmail, isValidPassword, isValidProgramId, isValidName, isValidCategory, sanitizeString } from "../lib/validate";

test("isValidEmail — valid", () => {
  assert(isValidEmail("user@example.com"), "should accept valid email");
  assert(isValidEmail("a.b+c@d.co"), "should accept email with dots and plus");
});

test("isValidEmail — invalid", () => {
  assert(!isValidEmail(""), "should reject empty");
  assert(!isValidEmail("noatsign"), "should reject no @");
  assert(!isValidEmail("@no-local.com"), "should reject no local part");
});

test("isValidPassword — valid", () => {
  assert(isValidPassword("12345678"), "should accept 8 chars");
  assert(isValidPassword("a".repeat(128)), "should accept 128 chars");
});

test("isValidPassword — invalid", () => {
  assert(!isValidPassword("short"), "should reject < 8 chars");
  assert(!isValidPassword("a".repeat(129)), "should reject > 128 chars");
  assert(!isValidPassword(""), "should reject empty");
});

test("isValidProgramId — valid", () => {
  assert(isValidProgramId("my-program-1"), "should accept lowercase with hyphens");
  assert(isValidProgramId("abc"), "should accept 3 chars");
});

test("isValidProgramId — invalid", () => {
  assert(!isValidProgramId("ab"), "should reject < 3 chars");
  assert(!isValidProgramId("My-Program"), "should reject uppercase");
  assert(!isValidProgramId("a".repeat(65)), "should reject > 64 chars");
  assert(!isValidProgramId("has space"), "should reject spaces");
});

test("isValidName — valid", () => {
  assert(isValidName("Test"), "should accept simple name");
  assert(isValidName("a"), "should accept 1 char");
  assert(isValidName("a".repeat(100)), "should accept 100 chars");
});

test("isValidName — invalid", () => {
  assert(!isValidName(""), "should reject empty");
  assert(!isValidName("a".repeat(101)), "should reject > 100 chars");
  assert(!isValidName(123 as unknown as string), "should reject non-string");
});

test("isValidCategory — valid categories", () => {
  for (const cat of ["general", "defi", "gaming", "social", "infrastructure"]) {
    assert(isValidCategory(cat), `should accept '${cat}'`);
  }
});

test("isValidCategory — invalid", () => {
  assert(!isValidCategory("unknown"), "should reject unknown category");
  assert(!isValidCategory(""), "should reject empty");
});

test("sanitizeString — truncates and trims", () => {
  assert(sanitizeString("  hello  ", 100) === "hello", "should trim");
  assert(sanitizeString("abcdef", 3) === "abc", "should truncate to maxLen");
  assert(sanitizeString("", 100) === "", "should handle empty");
});

// ---- token-limiter.ts (in-memory mode) ----
console.log("\n=== lib/token-limiter.ts (in-memory) ===");

// Ensure no Redis env vars so it uses in-memory fallback
delete process.env.UPSTASH_REDIS_REST_URL;
delete process.env.UPSTASH_REDIS_REST_TOKEN;

import { getUsage, checkLimit, recordUsage, LimitExceededError } from "../lib/token-limiter";

test("getUsage — new user has 0 used", async () => {
  const usage = await getUsage("test-user-unit-001", 50000);
  assert(usage.used === 0, `expected 0 used, got ${usage.used}`);
  assert(usage.limit === 50000, `expected 50000 limit, got ${usage.limit}`);
  assert(usage.remaining === 50000, `expected 50000 remaining, got ${usage.remaining}`);
});

test("recordUsage — records and returns updated usage", async () => {
  const usage = await recordUsage("test-user-unit-002", 1000, 50000);
  assert(usage.used === 1000, `expected 1000 used, got ${usage.used}`);
  assert(usage.remaining === 49000, `expected 49000 remaining, got ${usage.remaining}`);
});

test("recordUsage — accumulates", async () => {
  await recordUsage("test-user-unit-003", 25000, 50000);
  const usage = await recordUsage("test-user-unit-003", 25000, 50000);
  assert(usage.used === 50000, `expected 50000, got ${usage.used}`);
  assert(usage.remaining === 0, `expected 0 remaining, got ${usage.remaining}`);
});

test("checkLimit — throws when limit exceeded", async () => {
  await recordUsage("test-user-unit-004", 50001, 50000);
  let threw = false;
  try {
    await checkLimit("test-user-unit-004", 50000);
  } catch (e) {
    threw = e instanceof LimitExceededError;
  }
  assert(threw, "should throw LimitExceededError");
});

test("checkLimit — passes when under limit", async () => {
  let threw = false;
  try {
    await checkLimit("test-user-unit-new", 50000);
  } catch {
    threw = true;
  }
  assert(!threw, "should not throw");
});

// ---- oauth.ts (config checks) ----
console.log("\n=== lib/oauth.ts (config checks) ===");

// Clear OAuth env vars
delete process.env.GOOGLE_CLIENT_ID;
delete process.env.NAVER_CLIENT_ID;
delete process.env.NAVER_CLIENT_SECRET;
delete process.env.KAKAO_REST_API_KEY;

import { isGoogleAuthConfigured, isNaverAuthConfigured, isKakaoAuthConfigured } from "../lib/oauth";

test("isGoogleAuthConfigured — false when no env", () => {
  assert(!isGoogleAuthConfigured(), "should be false");
});

test("isNaverAuthConfigured — false when no env", () => {
  assert(!isNaverAuthConfigured(), "should be false");
});

test("isKakaoAuthConfigured — false when no env", () => {
  assert(!isKakaoAuthConfigured(), "should be false");
});

// ---- Summary ----
console.log(`\n=== Results: ${passed} passed, ${failed} failed ===\n`);
process.exit(failed > 0 ? 1 : 0);
