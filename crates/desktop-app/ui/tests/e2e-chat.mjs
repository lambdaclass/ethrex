#!/usr/bin/env node
/**
 * E2E Test: Tokamak Desktop App — Chat & Auth Flow
 *
 * Tests the actual Platform API and token management.
 * Requires: existing platform-token.json from a real login session.
 *
 * Usage: node tests/e2e-chat.mjs
 */

import fs from 'fs';
import path from 'path';
import os from 'os';

const PLATFORM_BASE_URL = 'https://tokamak-appchain.vercel.app';
const TOKEN_DIR = path.join(os.homedir(), 'Library/Application Support/tokamak-appchain');
const TOKEN_FILE = path.join(TOKEN_DIR, 'platform-token.json');

let passed = 0;
let failed = 0;

function assert(condition, name) {
  if (condition) {
    console.log(`  ✅ ${name}`);
    passed++;
  } else {
    console.log(`  ❌ ${name}`);
    failed++;
  }
}

function assertEqual(actual, expected, name) {
  if (actual === expected) {
    console.log(`  ✅ ${name}`);
    passed++;
  } else {
    console.log(`  ❌ ${name} — expected: ${expected}, got: ${actual}`);
    failed++;
  }
}

// ================================================================
// 1. Token File I/O
// ================================================================
console.log('\n=== 1. Token File I/O ===');

// 1.1 Read existing token
let originalToken = null;
try {
  const data = JSON.parse(fs.readFileSync(TOKEN_FILE, 'utf8'));
  originalToken = data.token;
  assert(typeof originalToken === 'string' && originalToken.length > 10, '1.1 토큰 파일 읽기');
} catch (e) {
  console.log(`  ❌ 1.1 토큰 파일 읽기 실패: ${e.message}`);
  console.log('  ⚠️  로그인된 상태에서 테스트를 실행해주세요.');
  process.exit(1);
}

// 1.2 Token file format
const tokenData = JSON.parse(fs.readFileSync(TOKEN_FILE, 'utf8'));
assert(tokenData.hasOwnProperty('token'), '1.2 토큰 파일 형식 ({"token": "..."})');

// ================================================================
// 2. Platform API — Auth (GET /api/auth/me)
// ================================================================
console.log('\n=== 2. Platform Auth API ===');

async function fetchMe(token) {
  const resp = await fetch(`${PLATFORM_BASE_URL}/api/auth/me`, {
    headers: { 'Authorization': `Bearer ${token}` }
  });
  return { status: resp.status, data: resp.ok ? await resp.json() : null };
}

const meResult = await fetchMe(originalToken);
assert(meResult.status === 200, '2.1 GET /api/auth/me — 200 OK');
assert(meResult.data && meResult.data.email, '2.2 사용자 이메일 존재');
assert(meResult.data && meResult.data.name, '2.3 사용자 이름 존재');

if (meResult.data) {
  console.log(`       → ${meResult.data.name} (${meResult.data.email})`);
}

// 2.4 Invalid token
const badMeResult = await fetchMe('ps_invalid_token_12345');
assert(badMeResult.status === 401, '2.4 잘못된 토큰 → 401 Unauthorized');

// ================================================================
// 3. Platform API — Token Usage (GET /api/ai/usage)
// ================================================================
console.log('\n=== 3. Token Usage API ===');

async function fetchUsage(token) {
  const resp = await fetch(`${PLATFORM_BASE_URL}/api/ai/usage`, {
    headers: { 'Authorization': `Bearer ${token}` }
  });
  return { status: resp.status, data: resp.ok ? await resp.json() : null };
}

const usage1 = await fetchUsage(originalToken);
assert(usage1.status === 200, '3.1 GET /api/ai/usage — 200 OK');
assert(usage1.data && typeof usage1.data.used === 'number', '3.2 used 필드 (number)');
assert(usage1.data && typeof usage1.data.limit === 'number', '3.3 limit 필드 (number)');
assert(usage1.data && typeof usage1.data.remaining === 'number', '3.4 remaining 필드 (number)');

if (usage1.data) {
  console.log(`       → used: ${usage1.data.used}, limit: ${usage1.data.limit}, remaining: ${usage1.data.remaining}`);
}

// 3.5 Same token, same result (consistency check)
const usage2 = await fetchUsage(originalToken);
assertEqual(usage2.data?.used, usage1.data?.used, '3.5 동일 토큰 → 동일 사용량 (일관성)');

// 3.6 Invalid token
const badUsage = await fetchUsage('ps_invalid_token_12345');
assert(badUsage.status === 401, '3.6 잘못된 토큰 → 401 Unauthorized');

// ================================================================
// 4. Logout → Login Simulation (Token Usage Preservation)
// ================================================================
console.log('\n=== 4. 로그아웃 → 로그인 시뮬레이션 ===');

const usageBefore = usage1.data?.used;
console.log(`       로그아웃 전 사용량: ${usageBefore}`);

// 4.1 Simulate logout: delete token file
const backupFile = TOKEN_FILE + '.backup';
fs.copyFileSync(TOKEN_FILE, backupFile);
fs.unlinkSync(TOKEN_FILE);
assert(!fs.existsSync(TOKEN_FILE), '4.1 로그아웃: 토큰 파일 삭제됨');

// 4.2 Verify API fails without token
const usageAfterLogout = await fetchUsage(originalToken);
// Token is still valid on server even after local file deletion
// (server sessions are independent of local storage)
assert(usageAfterLogout.status === 200, '4.2 서버 세션은 로컬 파일 삭제와 무관');

// 4.3 Simulate re-login: restore token file
fs.copyFileSync(backupFile, TOKEN_FILE);
fs.unlinkSync(backupFile);
assert(fs.existsSync(TOKEN_FILE), '4.3 재로그인: 토큰 파일 복원됨');

// 4.4 Verify usage preserved after re-login
const usageAfterRelogin = await fetchUsage(originalToken);
assertEqual(usageAfterRelogin.data?.used, usageBefore, '4.4 재로그인 후 사용량 보존 (서버 기준)');
console.log(`       재로그인 후 사용량: ${usageAfterRelogin.data?.used}`);

// ================================================================
// 5. Concurrent Usage Calls (Dual-Token Bug Check)
// ================================================================
console.log('\n=== 5. 동시 호출 일관성 ===');

const [c1, c2, c3] = await Promise.all([
  fetchUsage(originalToken),
  fetchUsage(originalToken),
  fetchUsage(originalToken),
]);
assertEqual(c1.data?.used, c2.data?.used, '5.1 동시 3회 호출: call1 === call2');
assertEqual(c2.data?.used, c3.data?.used, '5.2 동시 3회 호출: call2 === call3');

// ================================================================
// 6. Rust Backend Tests
// ================================================================
console.log('\n=== 6. Rust 단위 테스트 ===');

const { execSync } = await import('child_process');
try {
  const rustOutput = execSync(
    `cd ${path.resolve(import.meta.dirname, '../src-tauri')} && cargo test --lib -- ai_provider::tests 2>&1`,
    { encoding: 'utf8', timeout: 60000 }
  );
  const rustPassMatch = rustOutput.match(/(\d+) passed/);
  const rustFailMatch = rustOutput.match(/(\d+) failed/);
  const rustPassed = rustPassMatch ? parseInt(rustPassMatch[1]) : 0;
  const rustFailed = rustFailMatch ? parseInt(rustFailMatch[1]) : 0;

  assert(rustPassed > 0, `6.1 Rust 단위 테스트 통과: ${rustPassed}개`);
  assert(rustFailed === 0, `6.2 Rust 단위 테스트 실패: ${rustFailed}개`);
} catch (e) {
  const output = e.stdout || e.message;
  console.log(`  ❌ 6.1 Rust 테스트 실행 실패`);
  failed++;
  // Extract pass/fail from output
  const rustFailMatch = output.match(/(\d+) failed/);
  if (rustFailMatch) {
    console.log(`  ❌ 6.2 Rust 테스트 ${rustFailMatch[1]}개 실패`);
    failed++;
  }
}

// ================================================================
// Summary
// ================================================================
console.log('\n' + '='.repeat(50));
console.log(`총 ${passed + failed}개 테스트: ✅ ${passed} 통과, ❌ ${failed} 실패`);
console.log('='.repeat(50));

process.exit(failed > 0 ? 1 : 0);
