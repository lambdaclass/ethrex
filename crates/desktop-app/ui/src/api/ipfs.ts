/**
 * IPFS upload via Pinata API.
 * Pinata JWT is stored in OS Keychain (same pattern as AI API keys).
 */

import { invoke } from '@tauri-apps/api/core'

const PINATA_API = import.meta.env.VITE_PINATA_API || 'https://api.pinata.cloud'
const PINATA_GATEWAY = import.meta.env.VITE_PINATA_GATEWAY || 'https://gateway.pinata.cloud/ipfs'

/** Retrieve Pinata JWT from OS Keychain */
async function getPinataJWT(): Promise<string> {
  const jwt = await invoke<string | null>('get_keychain_value', { key: 'pinata_jwt' })
  if (!jwt) throw new Error('Pinata API key not configured. Set it in Settings.')
  return jwt
}

/** Upload a file to IPFS via Pinata and return ipfs:// URI */
export async function uploadFileToIPFS(file: File): Promise<string> {
  const jwt = await getPinataJWT()
  const formData = new FormData()
  formData.append('file', file)

  const res = await fetch(`${PINATA_API}/pinning/pinFileToIPFS`, {
    method: 'POST',
    headers: { 'Authorization': `Bearer ${jwt}` },
    body: formData,
  })

  if (!res.ok) {
    const text = await res.text()
    let msg = res.statusText
    try { const parsed = JSON.parse(text); msg = parsed.error?.details || parsed.error || msg } catch { /* use statusText */ }
    throw new Error(`IPFS upload failed: ${msg}`)
  }

  const data = await res.json()
  return `ipfs://${data.IpfsHash}`
}

/** Upload JSON metadata to IPFS via Pinata and return ipfs:// URI */
export async function uploadJSONToIPFS(metadata: AppchainMetadata): Promise<string> {
  const jwt = await getPinataJWT()

  const res = await fetch(`${PINATA_API}/pinning/pinJSONToIPFS`, {
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${jwt}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({
      pinataContent: metadata,
      pinataMetadata: {
        name: `tokamak-appchain-${metadata.name}`,
      },
    }),
  })

  if (!res.ok) {
    const text = await res.text()
    let msg = res.statusText
    try { const parsed = JSON.parse(text); msg = parsed.error?.details || parsed.error || msg } catch { /* use statusText */ }
    throw new Error(`IPFS JSON upload failed: ${msg}`)
  }

  const data = await res.json()
  return `ipfs://${data.IpfsHash}`
}

/** Convert ipfs:// URI to HTTP gateway URL */
export function ipfsToHttp(uri: string): string {
  if (uri.startsWith('ipfs://')) {
    return `${PINATA_GATEWAY}/${uri.replace('ipfs://', '')}`
  }
  return uri
}

/** Check if Pinata JWT is configured */
export async function isPinataConfigured(): Promise<boolean> {
  try {
    const jwt = await invoke<string | null>('get_keychain_value', { key: 'pinata_jwt' })
    return !!jwt
  } catch (err) {
    console.warn('Failed to check Pinata configuration:', err)
    return false
  }
}

/** Save Pinata JWT to OS Keychain */
export async function savePinataJWT(jwt: string): Promise<void> {
  await invoke('save_keychain_value', { key: 'pinata_jwt', value: jwt })
}

/** Delete Pinata JWT from OS Keychain */
export async function deletePinataJWT(): Promise<void> {
  await invoke('delete_keychain_value', { key: 'pinata_jwt' })
}

// ============================================================
// Tokamak Appchain Metadata Schema (IPFS JSON)
// ============================================================

export interface AppchainMetadata {
  /** Schema version for forward compatibility */
  version: '1.0'
  /** Appchain display name */
  name: string
  /** Appchain description */
  description?: string
  /** Logo image (ipfs:// URI) */
  logo?: string
  /** Screenshot images (ipfs:// URIs) */
  screenshots: string[]
  /** Network configuration */
  network: {
    chainId: number
    rpcUrl?: string
    wsUrl?: string
    networkMode: 'local' | 'testnet' | 'mainnet'
    l1ChainId: number
  }
  /** L1 contract addresses */
  contracts: {
    onChainProposer: string
    commonBridge?: string
    sp1Verifier?: string
  }
  /** Service URLs */
  services?: {
    explorer?: string
    bridgeUI?: string
    dashboard?: string
  }
  /** Social links */
  socialLinks?: Record<string, string>
  /** Native token info */
  nativeToken: {
    name: string
    symbol: string
    decimals: number
  }
  /** ZK proof system */
  proofSystem: 'sp1' | 'risc0' | 'tdx'
  /** ISO 8601 timestamp of metadata creation */
  createdAt: string
  /** ISO 8601 timestamp of last metadata update */
  updatedAt: string
}

/** Build metadata object from appchain config */
export function buildMetadata(config: {
  name: string
  description?: string
  chainId: number
  rpcUrl?: string
  networkMode: string
  l1ChainId: number
  proposerAddress?: string
  bridgeAddress?: string
  screenshots?: string[]
  socialLinks?: Record<string, string>
  explorerUrl?: string
  bridgeUIUrl?: string
  nativeToken?: { name: string; symbol: string; decimals: number }
  proofSystem?: 'sp1' | 'risc0' | 'tdx'
}): AppchainMetadata {
  const now = new Date().toISOString()
  return {
    version: '1.0',
    name: config.name,
    description: config.description,
    screenshots: config.screenshots || [],
    network: {
      chainId: config.chainId,
      rpcUrl: config.rpcUrl,
      networkMode: (config.networkMode as 'local' | 'testnet' | 'mainnet') || 'local',
      l1ChainId: config.l1ChainId,
    },
    contracts: {
      onChainProposer: config.proposerAddress || '',
      commonBridge: config.bridgeAddress,
    },
    services: {
      explorer: config.explorerUrl,
      bridgeUI: config.bridgeUIUrl,
    },
    socialLinks: config.socialLinks,
    nativeToken: config.nativeToken || { name: 'Tokamak', symbol: 'TON', decimals: 18 },
    proofSystem: config.proofSystem || 'sp1',
    createdAt: now,
    updatedAt: now,
  }
}
