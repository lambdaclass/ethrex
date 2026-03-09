import type { StoredCredential, SignResult } from './passkey';

const API_BASE = '/api';

export interface FrameReceipt {
  mode: string;
  status: boolean;
  gasUsed: string;
}

export interface TxResult {
  success: boolean;
  txHash?: string;
  gasUsed?: string;
  frameReceipts?: FrameReceipt[];
  deployedAddress?: string;
  error?: string;
}

export interface SimpleSendRequest {
  address: string;
  to: string;
  amount: string;
  signature: SignResult['signature'];
  webauthn: SignResult['webauthn'];
}

export interface SponsoredSendRequest {
  address: string;
  to: string;
  amount: string;
  signature: SignResult['signature'];
  webauthn: SignResult['webauthn'];
}

export interface BatchOpsRequest {
  address: string;
  operations: { to: string; value: string; data: string }[];
  signature: SignResult['signature'];
  webauthn: SignResult['webauthn'];
}

export interface DeployExecuteRequest {
  address: string;
  bytecode: string;
  calldata: string;
  signature: SignResult['signature'];
  webauthn: SignResult['webauthn'];
}

async function post<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({ error: res.statusText }));
    throw new Error(err.error || `Request failed: ${res.status}`);
  }
  return res.json();
}

export async function registerAccount(
  credential: StoredCredential
): Promise<{ success: boolean; address: string }> {
  return post('/register', {
    credentialId: credential.id,
    publicKey: credential.publicKey,
  });
}

export async function getSigHash(
  demoType: string,
  params: Record<string, unknown>
): Promise<{ sigHash: string }> {
  return post('/sig-hash', { demoType, params });
}

export async function simpleSend(body: SimpleSendRequest): Promise<TxResult> {
  return post('/simple-send', body);
}

export async function sponsoredSend(body: SponsoredSendRequest): Promise<TxResult> {
  return post('/sponsored-send', body);
}

export async function batchOps(body: BatchOpsRequest): Promise<TxResult> {
  return post('/batch-ops', body);
}

export async function deployExecute(body: DeployExecuteRequest): Promise<TxResult> {
  return post('/deploy-execute', body);
}

export async function getTokenBalance(address: string): Promise<{ balance: string; formatted: string }> {
  const res = await fetch(`${API_BASE}/token-balance/${address}`);
  return res.json();
}
