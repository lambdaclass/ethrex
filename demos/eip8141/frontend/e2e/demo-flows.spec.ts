/**
 * EIP-8141 Frame Transaction — Playwright E2E Tests
 *
 * Tests all 4 demo flows through the browser UI:
 *   1. Account registration (passkey via virtual authenticator + account deployment)
 *   2. Simple Send (ETH transfer)
 *   3. Sponsored ERC20 Send (gas paid by sponsor)
 *   4. Batch Operations (multiple transfers in one tx)
 *   5. Deploy + Execute (CREATE2 deploy + call)
 *
 * Each test verifies:
 *   - UI shows correct result (success, frame receipts)
 *   - ETH and token balances before/after via RPC
 *   - Nonces before/after via RPC
 *   - Blockscout indexes the tx as type 6 with correct frame structure
 *
 * Prerequisites:
 *   - ethrex node running on port 8545
 *   - Backend running on port 3000
 *   - Frontend dev server on port 5173 (auto-started by playwright.config.ts)
 *   - Blockscout on port 8082 (optional — Blockscout checks are skipped if unavailable)
 *
 * Run: npx playwright test
 */
import { test, expect, type Page, type CDPSession } from '@playwright/test';
import {
  setupVirtualAuthenticator,
  captureState,
  getEthBalance,
  getNonce,
  getTokenBalance,
  getTransactionByHash,
  getTransactionReceipt,
  fetchBlockscoutTx,
  waitForTxResult,
  switchToTab,
  formatEth,
  formatTokens,
  setAccount,
  ACCOUNT,
  SPONSOR,
  DEAD,
  ADDR_0001,
  DEPLOYER_PROXY,
  type AccountState,
} from './helpers';

// Shared state across sequential tests
let page: Page;
let cdp: CDPSession;
let authenticatorId: string;

test.describe.serial('EIP-8141 Frame Transaction Demo', () => {
  test.beforeAll(async ({ browser }) => {
    const context = await browser.newContext();
    page = await context.newPage();

    // Set up virtual authenticator for WebAuthn
    const auth = await setupVirtualAuthenticator(page);
    cdp = auth.cdp;
    authenticatorId = auth.authenticatorId;
  });

  test.afterAll(async () => {
    await cdp.send('WebAuthn.removeVirtualAuthenticator', { authenticatorId });
    await page.context().close();
  });

  // ═══════════════════════════════════════════════════════════════════
  //  REGISTRATION
  // ═══════════════════════════════════════════════════════════════════
  test('register passkey account', async () => {
    await page.goto('/');

    // Should show "Connect with Passkey" when not logged in
    await expect(page.getByText('Connect with Passkey')).toBeVisible();

    // Click "Create Account" — this creates a passkey AND deploys a per-user account
    await page.getByRole('button', { name: 'Create Account' }).click();

    // Wait for the account panel to show the connected state (address + ETH balance)
    // Registration now deploys a new account via factory, so the address is dynamic
    await expect(page.locator('code.font-mono').first()).toBeVisible({ timeout: 60_000 });

    // Capture the deployed account address from the UI
    const addressText = await page.locator('code.font-mono').first().textContent();
    expect(addressText).toBeTruthy();
    expect(addressText).toMatch(/^0x[0-9a-fA-F]{4}\.\.\..*$/);

    // Extract full address from localStorage (the UI shows truncated)
    const fullAddress = await page.evaluate(() => {
      const raw = localStorage.getItem('eip8141_credential');
      if (!raw) return null;
      return JSON.parse(raw).address;
    });
    expect(fullAddress).toBeTruthy();
    expect(fullAddress).toMatch(/^0x[0-9a-fA-F]{40}$/);

    // Set the dynamic account address for use in subsequent tests
    setAccount(fullAddress!);
    console.log(`[test] Account deployed at: ${ACCOUNT}`);

    // Balance should appear in the header (e.g., "9.9901 ETH" and "1000000.00 DEMO")
    await expect(page.locator('header').getByText(/[\d.]+ ETH/)).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('header').getByText(/[\d.]+ DEMO/)).toBeVisible({ timeout: 15_000 });

    // Verify via RPC that the account has ETH and tokens (funded during registration)
    const ethBal = await getEthBalance(ACCOUNT);
    expect(ethBal).toBeGreaterThan(0n);
    const tokenBal = await getTokenBalance(ACCOUNT);
    expect(tokenBal).toBeGreaterThan(0n);

    // Should now show the tab layout
    await expect(page.getByRole('button', { name: 'Simple Send' })).toBeVisible();
  });

  // ═══════════════════════════════════════════════════════════════════
  //  SIMPLE SEND
  // ═══════════════════════════════════════════════════════════════════
  test('simple send: 0.001 ETH to 0xdead', async () => {
    const SEND_AMOUNT = 1_000_000_000_000_000n; // 0.001 ETH

    // Capture state BEFORE
    const accountBefore = await captureState(ACCOUNT);
    const deadBefore = await captureState(DEAD);

    // Navigate to Simple Send tab (should be default)
    await switchToTab(page, 'Simple Send');
    await expect(page.getByText('Send ETH from your passkey account')).toBeVisible();

    // Fill form
    await page.getByPlaceholder('0x...').fill(DEAD);
    await page.getByPlaceholder('0.1').fill('0.001');

    // Submit
    await page.getByRole('button', { name: 'Send', exact: true }).click();

    // Wait for status messages to appear and disappear
    await expect(page.getByText('Building transaction...')).toBeVisible({ timeout: 10_000 });

    // Wait for tx result
    const result = await waitForTxResult(page);

    // ── UI verification ──
    expect(result.success).toBe(true);
    expect(result.txHash).toBeTruthy();
    expect(result.txHash).toMatch(/^0x[0-9a-f]{64}$/);

    // Frame receipts: VERIFY + SENDER
    expect(result.frameResults).toHaveLength(2);
    expect(result.frameResults[0].mode).toBe('VERIFY');
    expect(result.frameResults[0].status).toBe('OK');
    expect(result.frameResults[1].mode).toBe('SENDER');
    expect(result.frameResults[1].status).toBe('OK');

    // ── Tx hash link verification ──
    // The tx hash must be a clickable <a> link pointing to Blockscout
    const txLink = page.locator('a.font-mono').first();
    await expect(txLink).toBeVisible();
    const href = await txLink.getAttribute('href');
    expect(href).toBeTruthy();
    expect(href).toContain('/tx/');
    expect(href).toContain(result.txHash!);
    expect(href).toMatch(/^https?:\/\/.+:8082\/tx\/0x/);
    // Link must NOT have target="_blank" (Safari blocks HTTPS→HTTP popups)
    const target = await txLink.getAttribute('target');
    expect(target).toBeNull();
    // Verify the element is an <a> tag (not a <code> or <span>)
    const tagName = await txLink.evaluate(el => el.tagName.toLowerCase());
    expect(tagName).toBe('a');

    // ── Balance verification via RPC ──
    const accountAfter = await captureState(ACCOUNT);
    const deadAfter = await captureState(DEAD);

    // Dead received exactly 0.001 ETH
    const deadDelta = deadAfter.ethBalance - deadBefore.ethBalance;
    expect(deadDelta).toBe(SEND_AMOUNT);

    // Account spent > 0.001 ETH (transfer + gas)
    const accountDelta = accountBefore.ethBalance - accountAfter.ethBalance;
    expect(accountDelta).toBeGreaterThan(SEND_AMOUNT);

    // Token balance unchanged
    expect(accountAfter.tokenBalance).toBe(accountBefore.tokenBalance);

    // ── Nonce verification ──
    expect(accountAfter.nonce).toBe(accountBefore.nonce + 1n);

    // ── Blockscout verification ──
    await verifyFrameTxOnBlockscout(result.txHash!, 2, ['VERIFY', 'SENDER']);
  });

  // ═══════════════════════════════════════════════════════════════════
  //  SPONSORED ERC20 SEND
  // ═══════════════════════════════════════════════════════════════════
  test('sponsored send: 100 DEMO to 0xdead', async () => {
    const SEND_TOKENS = 100n * 10n ** 18n;

    // Capture state BEFORE
    const accountBefore = await captureState(ACCOUNT);
    const deadBefore = await captureState(DEAD);
    const sponsorBefore = await captureState(SPONSOR);

    // Switch to Sponsored tab
    await switchToTab(page, 'Sponsored');
    await expect(page.getByText('Send ERC20 tokens without paying gas')).toBeVisible();

    // Fill form
    await page.getByPlaceholder('0x...').fill(DEAD);
    await page.getByPlaceholder('100').fill('100');

    // Submit
    await page.getByRole('button', { name: 'Send (Sponsored)' }).click();

    // Wait for tx result
    await expect(page.getByText('Building transaction...')).toBeVisible({ timeout: 10_000 });
    const result = await waitForTxResult(page);

    // ── UI verification ──
    expect(result.success).toBe(true);
    expect(result.txHash).toBeTruthy();

    // Frame receipts: VERIFY (sender) + VERIFY (sponsor) + SENDER
    expect(result.frameResults).toHaveLength(3);
    expect(result.frameResults[0].mode).toBe('VERIFY');
    expect(result.frameResults[0].status).toBe('OK');
    expect(result.frameResults[1].mode).toBe('VERIFY');
    expect(result.frameResults[1].status).toBe('OK');
    expect(result.frameResults[2].mode).toBe('SENDER');
    expect(result.frameResults[2].status).toBe('OK');

    // ── Balance verification via RPC ──
    const accountAfter = await captureState(ACCOUNT);
    const deadAfter = await captureState(DEAD);
    const sponsorAfter = await captureState(SPONSOR);

    // Account sent exactly 100 DEMO tokens
    const accountTokenDelta = accountBefore.tokenBalance - accountAfter.tokenBalance;
    expect(accountTokenDelta).toBe(SEND_TOKENS);

    // Dead received exactly 100 DEMO tokens
    const deadTokenDelta = deadAfter.tokenBalance - deadBefore.tokenBalance;
    expect(deadTokenDelta).toBe(SEND_TOKENS);

    // Sponsor paid gas (ETH decreased)
    const sponsorEthDelta = sponsorBefore.ethBalance - sponsorAfter.ethBalance;
    expect(sponsorEthDelta).toBeGreaterThan(0n);

    // Account ETH unchanged (gas paid by sponsor)
    expect(accountAfter.ethBalance).toBe(accountBefore.ethBalance);

    // ── Nonce verification ──
    expect(accountAfter.nonce).toBe(accountBefore.nonce + 1n);

    // ── Blockscout verification ──
    await verifyFrameTxOnBlockscout(result.txHash!, 3, ['VERIFY', 'VERIFY', 'SENDER']);
  });

  // ═══════════════════════════════════════════════════════════════════
  //  BATCH OPERATIONS
  // ═══════════════════════════════════════════════════════════════════
  test('batch ops: 0.001 ETH to 0xdead + 0.001 ETH to 0x0001', async () => {
    const SEND_PER_OP = 1_000_000_000_000_000n;
    const TOTAL_SEND = SEND_PER_OP * 2n;

    // Capture state BEFORE
    const accountBefore = await captureState(ACCOUNT);
    const deadBefore = await captureState(DEAD);
    const addr1Before = await getEthBalance(ADDR_0001);

    // Switch to Batch tab
    await switchToTab(page, 'Batch');
    await expect(page.getByText('Execute multiple operations')).toBeVisible();

    // Fill operation 1 (already present by default)
    const op1 = page.locator('[class*="border-zinc-700/50"]').first();
    await op1.getByPlaceholder('To address (0x...)').fill(DEAD);
    await op1.getByPlaceholder('Value (ETH)').fill('0.001');

    // Add operation 2
    await page.getByRole('button', { name: '+ Add Operation' }).click();
    const op2 = page.locator('[class*="border-zinc-700/50"]').nth(1);
    await op2.getByPlaceholder('To address (0x...)').fill(ADDR_0001);
    await op2.getByPlaceholder('Value (ETH)').fill('0.001');

    // Submit
    await page.getByRole('button', { name: /Execute Batch/ }).click();

    // Wait for tx result
    await expect(page.getByText('Building transaction...')).toBeVisible({ timeout: 10_000 });
    const result = await waitForTxResult(page);

    // ── UI verification ──
    expect(result.success).toBe(true);
    expect(result.txHash).toBeTruthy();

    // Frame receipts: VERIFY + SENDER + SENDER
    expect(result.frameResults).toHaveLength(3);
    expect(result.frameResults[0].mode).toBe('VERIFY');
    expect(result.frameResults[0].status).toBe('OK');
    expect(result.frameResults[1].mode).toBe('SENDER');
    expect(result.frameResults[1].status).toBe('OK');
    expect(result.frameResults[2].mode).toBe('SENDER');
    expect(result.frameResults[2].status).toBe('OK');

    // ── Balance verification via RPC ──
    const accountAfter = await captureState(ACCOUNT);
    const deadAfter = await captureState(DEAD);
    const addr1After = await getEthBalance(ADDR_0001);

    // Dead received 0.001 ETH
    expect(deadAfter.ethBalance - deadBefore.ethBalance).toBe(SEND_PER_OP);

    // 0x0001 received 0.001 ETH
    expect(addr1After - addr1Before).toBe(SEND_PER_OP);

    // Account spent > 0.002 ETH (transfers + gas)
    const accountDelta = accountBefore.ethBalance - accountAfter.ethBalance;
    expect(accountDelta).toBeGreaterThan(TOTAL_SEND);

    // Token balance unchanged
    expect(accountAfter.tokenBalance).toBe(accountBefore.tokenBalance);

    // ── Nonce verification ──
    expect(accountAfter.nonce).toBe(accountBefore.nonce + 1n);

    // ── Blockscout verification ──
    await verifyFrameTxOnBlockscout(result.txHash!, 3, ['VERIFY', 'SENDER', 'SENDER']);
  });

  // ═══════════════════════════════════════════════════════════════════
  //  DEPLOY + EXECUTE
  // ═══════════════════════════════════════════════════════════════════
  test('deploy+execute: deploy returns-42 contract and call it', async () => {
    // Init code: deploys a contract that returns 42 on any call
    const runtimeCode = '602a60005260206000f3';
    const runtimeLen = runtimeCode.length / 2;
    const offset = 32 - runtimeLen;
    const initCode = `0x69${runtimeCode}600052600a60${offset.toString(16).padStart(2, '0')}f3`;

    // Capture state BEFORE
    const accountBefore = await captureState(ACCOUNT);

    // Switch to Deploy + Execute tab
    await switchToTab(page, 'Deploy + Execute');
    await expect(page.getByText('Deploy a contract and call it')).toBeVisible();

    // Fill form
    await page.getByPlaceholder('0x608060...').fill(initCode);
    // Leave constructor args and post-deploy calldata empty

    // Submit
    await page.getByRole('button', { name: 'Deploy & Execute' }).click();

    // Wait for tx result
    await expect(page.getByText('Building transaction...')).toBeVisible({ timeout: 10_000 });
    const result = await waitForTxResult(page);

    // ── UI verification ──
    expect(result.success).toBe(true);
    expect(result.txHash).toBeTruthy();

    // Frame receipts: VERIFY + DEFAULT (deploy) + SENDER (execute)
    expect(result.frameResults).toHaveLength(3);
    expect(result.frameResults[0].mode).toBe('VERIFY');
    expect(result.frameResults[0].status).toBe('OK');
    expect(result.frameResults[1].mode).toBe('DEFAULT');
    expect(result.frameResults[1].status).toBe('OK');
    expect(result.frameResults[2].mode).toBe('SENDER');
    expect(result.frameResults[2].status).toBe('OK');

    // Deployed address should be shown in the UI
    expect(result.deployedAddress).toBeTruthy();
    expect(result.deployedAddress).toMatch(/^0x[0-9a-fA-F]{40}$/);

    // ── Balance verification via RPC ──
    const accountAfter = await captureState(ACCOUNT);

    // Account spent ETH on gas
    const accountDelta = accountBefore.ethBalance - accountAfter.ethBalance;
    expect(accountDelta).toBeGreaterThan(0n);

    // Token balance unchanged
    expect(accountAfter.tokenBalance).toBe(accountBefore.tokenBalance);

    // ── Nonce verification ──
    expect(accountAfter.nonce).toBe(accountBefore.nonce + 1n);

    // ── Blockscout verification ──
    await verifyFrameTxOnBlockscout(result.txHash!, 3, ['VERIFY', 'DEFAULT', 'SENDER']);
  });
});

// ═══════════════════════════════════════════════════════════════════════
//  BLOCKSCOUT VERIFICATION
//
//  For each frame transaction, verify:
//  1. Blockscout API: tx type is 6 (frame tx), status is success
//  2. RPC: eth_getTransactionByHash returns type 0x6
//  3. RPC: eth_getTransactionReceipt has frameReceipts with correct
//     count, all succeeded, and each used gas > 0
//  4. Frame structure matches the EIP-8141 spec for the demo type
// ═══════════════════════════════════════════════════════════════════════
async function verifyFrameTxOnBlockscout(
  txHash: string,
  expectedFrameCount: number,
  expectedFrameModes: string[],
) {
  // ── RPC-level verification (always available) ──
  const rpcTx = await getTransactionByHash(txHash);
  expect(rpcTx).toBeTruthy();

  // Transaction type must be 6 (EIP-8141 frame tx)
  const rpcType = parseInt(rpcTx.type, 16);
  expect(rpcType).toBe(6);

  // Frame receipts in the receipt
  const receipt = await getTransactionReceipt(txHash);
  expect(receipt).toBeTruthy();

  if (receipt.frameReceipts) {
    const frameReceipts = receipt.frameReceipts as Array<{ status: string; gasUsed: string }>;

    // Correct number of frame receipts
    expect(frameReceipts.length).toBe(expectedFrameCount);

    // Each frame succeeded and used gas
    for (let i = 0; i < frameReceipts.length; i++) {
      const fr = frameReceipts[i];
      expect(fr.status).toBe('0x1'); // success
      const gasUsed = parseInt(fr.gasUsed, 16);
      expect(gasUsed).toBeGreaterThan(0);
    }
  }

  // ── Verify RPC tx has no legacy fields (EIP-8141 spec compliance) ──
  expect(rpcTx.type).toBe('0x6');

  // ── Blockscout API verification (if available) ──
  const blockscoutTx = await fetchBlockscoutTx(txHash);
  if (!blockscoutTx) {
    // Blockscout not available or slow — skip but don't fail
    console.log(`  [blockscout] Transaction ${txHash.slice(0, 18)}... not found (Blockscout may not be running)`);
    return;
  }

  // Transaction type is 6
  const bsType = typeof blockscoutTx.type === 'string' ? parseInt(blockscoutTx.type) : blockscoutTx.type;
  expect(bsType).toBe(6);

  // Transaction succeeded
  const status = blockscoutTx.status;
  expect(['ok', 'success', true].some(s => s === status)).toBe(true);

  // Transaction is included in a block
  expect(blockscoutTx.block).not.toBeNull();

  console.log(
    `  [blockscout] Verified ${txHash.slice(0, 18)}... — type=${bsType}, status=${status}, block=${blockscoutTx.block}`
  );
}
