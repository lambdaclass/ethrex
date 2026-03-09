import type { Page, CDPSession } from '@playwright/test';

// ── Constants ────────────────────────────────────────────────────────
export const RPC_URL = process.env.RPC_URL ?? 'http://localhost:8545';
export const BLOCKSCOUT_URL = process.env.BLOCKSCOUT_URL ?? 'http://localhost:8082';
export const ACCOUNT = '0x1000000000000000000000000000000000000003';
export const MOCK_ERC20 = '0x1000000000000000000000000000000000000002';
export const SPONSOR = '0x1000000000000000000000000000000000000001';
export const DEPLOYER_PROXY = '0x4e59b44847b379578588920ca78fbf26c0b4956c';
export const DEAD = '0x000000000000000000000000000000000000dEaD';
export const ADDR_0001 = '0x0000000000000000000000000000000000000001';

// ── RPC helpers ──────────────────────────────────────────────────────
async function rpc(method: string, params: unknown[] = []) {
  const res = await fetch(RPC_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', method, params, id: 1 }),
  });
  const data = await res.json();
  if (data.error) throw new Error(`RPC ${method}: ${data.error.message}`);
  return data.result;
}

export async function getEthBalance(address: string): Promise<bigint> {
  return BigInt(await rpc('eth_getBalance', [address, 'latest']));
}

export async function getNonce(address: string): Promise<bigint> {
  return BigInt(await rpc('eth_getTransactionCount', [address, 'latest']));
}

export async function getTokenBalance(address: string): Promise<bigint> {
  const paddedAddr = address.replace('0x', '').toLowerCase().padStart(64, '0');
  const data = '0x70a08231' + paddedAddr;
  const result = await rpc('eth_call', [{ to: MOCK_ERC20, data }, 'latest']);
  return BigInt(result);
}

export async function getTransactionByHash(txHash: string): Promise<any> {
  return await rpc('eth_getTransactionByHash', [txHash]);
}

export async function getTransactionReceipt(txHash: string): Promise<any> {
  return await rpc('eth_getTransactionReceipt', [txHash]);
}

export function formatEth(wei: bigint): string {
  return (Number(wei) / 1e18).toFixed(6);
}

export function formatTokens(raw: bigint): string {
  return (Number(raw) / 1e18).toFixed(2);
}

// ── Account state capture ────────────────────────────────────────────
export interface AccountState {
  ethBalance: bigint;
  tokenBalance: bigint;
  nonce: bigint;
}

export async function captureState(address: string): Promise<AccountState> {
  const [ethBalance, tokenBalance, nonce] = await Promise.all([
    getEthBalance(address),
    getTokenBalance(address),
    getNonce(address),
  ]);
  return { ethBalance, tokenBalance, nonce };
}

// ── WebAuthn Virtual Authenticator setup ─────────────────────────────
// Playwright + Chromium CDP allows creating a virtual authenticator
// that handles WebAuthn create/get requests without real biometrics.
export async function setupVirtualAuthenticator(page: Page): Promise<{
  cdp: CDPSession;
  authenticatorId: string;
}> {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('WebAuthn.enable', { enableUI: false });
  const { authenticatorId } = await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: {
      protocol: 'ctap2',
      transport: 'internal',
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  });
  return { cdp, authenticatorId };
}

// ── Blockscout API helpers ──────────────────────────────────────────
export interface BlockscoutTx {
  hash: string;
  type: number | string;
  status: string;
  block: number | null;
  tx_types?: string[];
}

export async function fetchBlockscoutTx(txHash: string, maxRetries = 20): Promise<BlockscoutTx | null> {
  for (let i = 0; i < maxRetries; i++) {
    try {
      const res = await fetch(`${BLOCKSCOUT_URL}/api/v2/transactions/${txHash}`);
      if (res.ok) {
        const data = await res.json();
        if (data.hash) return data as BlockscoutTx;
      }
    } catch { /* Blockscout not available or still indexing */ }
    await new Promise(r => setTimeout(r, 2000));
  }
  return null;
}

// ── UI helpers ───────────────────────────────────────────────────────
export async function waitForTxResult(page: Page): Promise<{
  success: boolean;
  txHash: string | null;
  frameResults: Array<{ mode: string; status: string; gas: string }>;
  deployedAddress: string | null;
}> {
  // Wait for the TxResult component to render "Transaction Successful" or "Transaction Failed"
  const resultText = page.getByText(/Transaction (Successful|Failed)/).first();
  await resultText.waitFor({ state: 'visible', timeout: 60_000 });

  // Navigate up to the TxResult container div
  const resultDiv = resultText.locator('xpath=ancestor::div[contains(@class,"rounded-lg") and contains(@class,"border")]').first();

  const successText = await resultText.textContent();
  const success = successText?.includes('Successful') ?? false;

  // Extract tx hash if present
  let txHash: string | null = null;
  const hashEl = resultDiv.locator('code.font-mono, a.font-mono').first();
  if (await hashEl.count() > 0) {
    txHash = await hashEl.textContent();
  }

  // Extract frame results
  const frameResults: Array<{ mode: string; status: string; gas: string }> = [];
  const frameRows = resultDiv.locator('.ml-2');
  const count = await frameRows.count();
  for (let i = 0; i < count; i++) {
    const row = frameRows.nth(i);
    const texts = await row.locator('span').allTextContents();
    // texts: [dot, mode, status, gas]
    if (texts.length >= 3) {
      frameResults.push({
        mode: texts[1]?.trim() ?? '',
        status: texts[2]?.trim() ?? '',
        gas: texts[3]?.replace(/[()gas: ]/g, '').trim() ?? '',
      });
    }
  }

  // Extract deployed address if present
  let deployedAddress: string | null = null;
  const deployedEl = resultDiv.locator('code.text-violet-400');
  if (await deployedEl.count() > 0) {
    deployedAddress = await deployedEl.textContent();
  }

  return { success, txHash, frameResults, deployedAddress };
}

export async function switchToTab(page: Page, tabLabel: string) {
  await page.getByRole('button', { name: tabLabel }).click();
}
