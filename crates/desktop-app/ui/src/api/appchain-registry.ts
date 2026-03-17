/**
 * Appchain Registry API client.
 * Submits signed metadata to Platform server, which creates GitHub PRs.
 */

const BASE_URL = import.meta.env.VITE_PLATFORM_URL || 'https://tokamak-appchain.vercel.app'

export interface SubmitResult {
  success: boolean
  prUrl?: string
  prNumber?: number
  filePath?: string
  error?: string
  code?: string
}

export interface PRStatus {
  prNumber: number
  state: string
  merged: boolean
  mergeable: boolean | null
  title: string
  htmlUrl: string
}

export interface CheckResult {
  exists: boolean
  createdAt?: string
  // Immutable fields from existing metadata (for updates)
  l2ChainId?: number
  nativeToken?: { type: 'eth' | 'erc20'; symbol: string; name: string; decimals: number; l1Address?: string }
}

export async function checkMetadataExists(
  l1ChainId: number,
  stackType: string,
  identityAddress: string,
): Promise<CheckResult> {
  const resp = await fetch(
    `${BASE_URL}/api/appchain-registry/check/${l1ChainId}/${stackType}/${identityAddress.toLowerCase()}`,
  )
  if (!resp.ok) return { exists: false }
  return resp.json()
}

export async function submitMetadata(
  metadata: Record<string, unknown>,
  operation: 'register' | 'update' = 'register',
): Promise<SubmitResult> {
  const resp = await fetch(`${BASE_URL}/api/appchain-registry/submit`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ metadata, operation }),
  })
  return resp.json()
}

export async function getSubmissionStatus(prNumber: number): Promise<PRStatus> {
  const resp = await fetch(`${BASE_URL}/api/appchain-registry/status/${prNumber}`)
  if (!resp.ok) throw new Error(`Failed to get PR status: ${resp.statusText}`)
  return resp.json()
}
