import { createCredential, parsePublicKey, sign } from 'webauthn-p256';
import { slice } from 'viem';

export interface StoredCredential {
  id: string;
  publicKey: { x: string; y: string };
  address: string;
}

export interface SignResult {
  signature: { r: string; s: string };
  webauthn: {
    authenticatorData: string;
    clientDataJSON: string;
    challengeIndex: number;
    typeIndex: number;
    userVerificationRequired: boolean;
  };
}

const STORAGE_KEY = 'eip8141_credential';

export async function registerPasskey(username: string): Promise<StoredCredential> {
  const id = new Uint8Array(32);
  crypto.getRandomValues(id);

  const credential = await createCredential({
    user: {
      name: `EIP-8141 ${username}`,
      id,
    },
  });

  const pubKey = parsePublicKey(credential.publicKey);

  const stored: StoredCredential = {
    id: credential.id,
    publicKey: {
      x: `0x${pubKey.x.toString(16).padStart(64, '0')}`,
      y: `0x${pubKey.y.toString(16).padStart(64, '0')}`,
    },
    address: '',
  };

  localStorage.setItem(STORAGE_KEY, JSON.stringify(stored));
  return stored;
}

export async function signChallenge(credentialId: string, sigHash: string): Promise<SignResult> {
  const result = await sign({
    hash: sigHash as `0x${string}`,
    credentialId,
  });

  const r = slice(result.signature, 0, 32);
  const s = slice(result.signature, 32, 64);

  const clientDataStr = typeof result.webauthn.clientDataJSON === 'string'
    ? result.webauthn.clientDataJSON
    : new TextDecoder().decode(
        Uint8Array.from(atob(result.webauthn.clientDataJSON), c => c.charCodeAt(0))
      );

  const challengeIndex = clientDataStr.indexOf('"challenge":"');
  const typeIndex = clientDataStr.indexOf('"type":"');

  return {
    signature: { r, s },
    webauthn: {
      authenticatorData: result.webauthn.authenticatorData,
      clientDataJSON: clientDataStr,
      challengeIndex,
      typeIndex,
      userVerificationRequired: true,
    },
  };
}

export function getStoredCredential(): StoredCredential | null {
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return null;
  return JSON.parse(raw) as StoredCredential;
}

export function setStoredCredential(credential: StoredCredential): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(credential));
}

export function clearCredential(): void {
  localStorage.removeItem(STORAGE_KEY);
}
