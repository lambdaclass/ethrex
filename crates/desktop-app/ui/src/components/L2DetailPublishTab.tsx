import { useState, useEffect, useRef, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLang } from '../App'
import { uploadFileToIPFS, ipfsToHttp, isPinataConfigured } from '../api/ipfs'
import { submitMetadata, getSubmissionStatus, checkMetadataExists, type SubmitResult } from '../api/appchain-registry'
import { open } from '@tauri-apps/plugin-shell'
import { SectionHeader } from './ui-atoms'
import type { L2Config } from './MyL2View'

function fileToDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(reader.result as string)
    reader.onerror = reject
    reader.readAsDataURL(file)
  })
}

function screenshotUrl(uri: string): string {
  if (uri.startsWith('ipfs://')) return ipfsToHttp(uri)
  return uri
}

/** Resolve the host IP from L2Config's rawConfig (for remote deployments). */
function resolveHostIp(l2: L2Config): string | null {
  try {
    if (l2.rawConfig) {
      const cfg = JSON.parse(l2.rawConfig)
      if (cfg.ec2IP) return cfg.ec2IP
    }
  } catch { /* ignore */ }
  return null
}

/** Build the L2 RPC URL from L2Config. */
function resolveRpcUrl(l2: L2Config): string {
  const hostIp = resolveHostIp(l2)
  return l2.publicRpcUrl || (hostIp ? `http://${hostIp}:${l2.rpcPort || 1729}` : `http://localhost:${l2.rpcPort}`)
}

/** Fetch actual chain ID from the L2 RPC node via eth_chainId. */
async function fetchRpcChainId(rpcUrl: string): Promise<number | null> {
  try {
    const resp = await fetch(rpcUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', method: 'eth_chainId', params: [], id: 1 }),
      signal: AbortSignal.timeout(5000),
    })
    const data = await resp.json()
    if (data.result) return parseInt(data.result, 16)
  } catch { /* RPC unreachable */ }
  return null
}

/** Build TokamakAppchainMetadata from L2Config + form state */
function buildAppchainMetadata(
  l2: L2Config,
  description: string,
  socialLinks: Record<string, string>,
  signerAddress: string,
  l1ChainId: number,
  overrides?: {
    l2ChainId?: number
    nativeToken?: { type: 'eth' | 'erc20'; symbol: string; name: string; decimals: number; l1Address?: string }
  },
) {
  const now = new Date().toISOString()
  const filteredSocial = Object.fromEntries(Object.entries(socialLinks).filter(([, v]) => v.trim()))

  const hostIp = resolveHostIp(l2)
  const rpcUrl = resolveRpcUrl(l2)
  const l1RpcUrl = l2.testnetL1RpcUrl || (hostIp ? `http://${hostIp}:${l2.l1Port || 8545}` : l2.l1Port ? `http://localhost:${l2.l1Port}` : undefined)
  const l2ExplorerUrl = l2.toolsL2ExplorerPort ? (hostIp ? `http://${hostIp}:${l2.toolsL2ExplorerPort}` : `http://localhost:${l2.toolsL2ExplorerPort}`) : undefined
  const l1ExplorerUrl = l2.toolsL1ExplorerPort ? (hostIp ? `http://${hostIp}:${l2.toolsL1ExplorerPort}` : `http://localhost:${l2.toolsL1ExplorerPort}`) : undefined
  // Dashboard: use DB port if available, fallback to default 3000 for remote deployments
  const bridgeUIPort = l2.toolsBridgeUIPort || (hostIp ? 3000 : null)
  const dashboardUrl = bridgeUIPort ? (hostIp ? `http://${hostIp}:${bridgeUIPort}` : `http://localhost:${bridgeUIPort}`) : undefined

  // Use overrides for immutable fields (from existing registry data or RPC)
  const l2ChainId = overrides?.l2ChainId ?? l2.l2ChainId ?? l2.chainId

  // Build explorers array
  const explorers = [
    ...(l2ExplorerUrl ? [{ name: 'L2 Explorer', url: l2ExplorerUrl, type: 'blockscout' as const, status: 'active' as const }] : []),
    ...(l1ExplorerUrl ? [{ name: 'L1 Explorer', url: l1ExplorerUrl, type: 'blockscout' as const, status: 'active' as const }] : []),
  ]

  // Build bridges array (only if dashboardUrl is a valid URI)
  const bridges = (l2.bridgeAddress && dashboardUrl)
    ? [{ name: 'Native Bridge', type: 'native' as const, url: dashboardUrl, status: 'active' as const }]
    : undefined

  // Build supportResources (schema fields: xUrl, telegramUrl, communityUrl, documentationUrl, dashboardUrl, etc.)
  const supportResources: Record<string, string> = {}
  if (dashboardUrl) supportResources.dashboardUrl = dashboardUrl
  if (filteredSocial.twitter) supportResources.xUrl = filteredSocial.twitter
  if (filteredSocial.discord) supportResources.communityUrl = filteredSocial.discord
  if (filteredSocial.telegram) supportResources.telegramUrl = filteredSocial.telegram
  if (filteredSocial.github) supportResources.documentationUrl = filteredSocial.github

  // nativeToken: use override (from existing registry) or derive from L2Config
  let nativeToken: Record<string, unknown>
  if (overrides?.nativeToken) {
    nativeToken = { ...overrides.nativeToken }
  } else {
    const isErc20 = l2.nativeToken && l2.nativeToken !== 'ETH'
    nativeToken = {
      type: isErc20 ? 'erc20' : 'eth',
      symbol: l2.nativeToken || 'ETH',
      name: l2.nativeToken || 'Ether',
      decimals: 18,
    }
  }

  return {
    l1ChainId,
    l2ChainId,
    name: l2.name,
    description: description || `${l2.name} appchain`,
    // Top-level website field (per schema)
    ...(filteredSocial.website ? { website: filteredSocial.website } : {}),
    stackType: 'tokamak-appchain',
    stackVersion: '0.1.0',
    rollupType: 'zk' as const,
    rpcUrl,
    l1RpcUrl,
    nativeToken,
    status: (l2.status === 'running' ? 'active' : 'inactive') as 'active' | 'inactive',
    createdAt: now,
    lastUpdated: now,
    l1Contracts: {
      Timelock: l2.timelockAddress!,
      OnChainProposer: l2.proposerAddress!,
      ...(l2.bridgeAddress ? { CommonBridge: l2.bridgeAddress } : {}),
      ...(l2.sp1VerifierAddress ? { SP1Verifier: l2.sp1VerifierAddress } : {}),
    },
    operator: {
      address: signerAddress,
    },
    ...(explorers.length > 0 ? { explorers } : {}),
    ...(bridges ? { bridges } : {}),
    ...(Object.keys(supportResources).length > 0 ? { supportResources } : {}),
    metadata: {
      version: '1.0.0',
      signature: '', // filled after signing
      signedBy: '',  // filled after signing
    },
  }
}

interface Props {
  l2: L2Config
  ko: boolean
  onRefresh?: () => void
}

export default function L2DetailPublishTab({ l2, ko }: Props) {
  useLang()

  const [publishDesc, setPublishDesc] = useState('')
  const [saved, setSaved] = useState(false)
  const [socialLinks, setSocialLinks] = useState<Record<string, string>>({})
  const [screenshots, setScreenshots] = useState<string[]>([])
  const [uploading, setUploading] = useState(false)
  const [uploadError, setUploadError] = useState('')
  const [pinataReady, setPinataReady] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)

  // Registry submission state
  const [submitting, setSubmitting] = useState(false)
  const [submitResult, setSubmitResult] = useState<SubmitResult | null>(null)
  const [submitError, setSubmitError] = useState('')

  // Keychain key selection
  const [keychainKeys, setKeychainKeys] = useState<Array<{ name: string; address: string }>>([])
  const [selectedKey, setSelectedKey] = useState('')
  const [keychainLoading, setKeychainLoading] = useState(true)

  const draftKey = `tokamak_publish_draft_${l2.id}`
  const LOCAL_SERVER = `http://127.0.0.1:${import.meta.env.VITE_LOCAL_SERVER_PORT || 5002}`

  useEffect(() => { isPinataConfigured().then(setPinataReady) }, [])

  // Load keychain keys from local server
  useEffect(() => {
    if (!l2.timelockAddress) return
    setKeychainLoading(true)
    fetch(`${LOCAL_SERVER}/api/keychain/keys`)
      .then(r => r.json())
      .then(async (data: { keys: string[] }) => {
        const keys: Array<{ name: string; address: string }> = []
        for (const name of data.keys || []) {
          try {
            const res = await fetch(`${LOCAL_SERVER}/api/keychain/keys/${encodeURIComponent(name)}`)
            const info = await res.json()
            if (info.address) keys.push({ name, address: info.address })
          } catch { /* skip */ }
        }
        setKeychainKeys(keys)

        // Auto-select if config has a keychainKeyName
        try {
          if (l2.rawConfig) {
            const cfg = JSON.parse(l2.rawConfig)
            const saved = cfg.testnet?.keychainKeyName
            if (saved && keys.some(k => k.name === saved)) {
              setSelectedKey(saved)
              return
            }
          }
        } catch { /* ignore */ }

        // Auto-select deployer_pk_{id} if exists
        const defaultKey = `deployer_pk_${l2.id}`
        if (keys.some(k => k.name === defaultKey)) {
          setSelectedKey(defaultKey)
        }
      })
      .catch(() => setKeychainKeys([]))
      .finally(() => setKeychainLoading(false))
  }, [l2.timelockAddress, l2.id, l2.rawConfig, LOCAL_SERVER])

  // Load draft
  useEffect(() => {
    try {
      const raw = localStorage.getItem(draftKey)
      if (raw) {
        const draft = JSON.parse(raw)
        if (draft.description) setPublishDesc(draft.description)
        if (draft.socialLinks) setSocialLinks(draft.socialLinks)
        if (draft.screenshots) setScreenshots(draft.screenshots)
        if (draft.prNumber) setSubmitResult({ success: true, prNumber: draft.prNumber, prUrl: draft.prUrl })
      }
    } catch { /* ignore */ }
  }, [draftKey])

  const saveDraft = useCallback((prResult?: SubmitResult | null) => {
    const filteredSocial = Object.fromEntries(Object.entries(socialLinks).filter(([, v]) => v.trim()))
    const draft: Record<string, unknown> = {
      description: publishDesc,
      socialLinks: Object.keys(filteredSocial).length > 0 ? filteredSocial : undefined,
      screenshots: screenshots.length > 0 ? screenshots : undefined,
    }
    // Use passed result (fresh) or fall back to state (may be stale in same render)
    const pr = prResult ?? submitResult
    if (pr?.prNumber) {
      draft.prNumber = pr.prNumber
      draft.prUrl = pr.prUrl
    }
    localStorage.setItem(draftKey, JSON.stringify(draft))
    setSaved(true)
    setTimeout(() => setSaved(false), 2000)
  }, [draftKey, publishDesc, socialLinks, screenshots, submitResult])

  const handleDescChange = (value: string) => {
    setPublishDesc(value)
    setSaved(false)
  }

  const handleSocialChange = (key: string, value: string) => {
    setSocialLinks(prev => ({ ...prev, [key]: value }))
    setSaved(false)
  }

  const handleScreenshotUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files
    if (!files || files.length === 0) return
    setUploading(true)
    setUploadError('')
    try {
      const newUrls: string[] = []
      for (const file of Array.from(files)) {
        if (pinataReady) {
          newUrls.push(await uploadFileToIPFS(file))
        } else {
          newUrls.push(await fileToDataUrl(file))
        }
      }
      setScreenshots(prev => [...prev, ...newUrls])
    } catch (err) {
      setUploadError(err instanceof Error ? err.message : String(err))
    } finally {
      setUploading(false)
      if (fileInputRef.current) fileInputRef.current.value = ''
    }
  }

  const removeScreenshot = (index: number) => {
    setScreenshots(prev => prev.filter((_, i) => i !== index))
  }

  // Sign and submit metadata to registry
  const handleSubmitMetadata = async () => {
    if (!l2.timelockAddress || !l2.proposerAddress) return
    setSubmitting(true)
    setSubmitError('')
    setSubmitResult(null)
    try {
      // 0. Check if metadata already exists → decide register vs update
      const check = await checkMetadataExists(effectiveL1ChainId, 'tokamak-appchain', l2.timelockAddress)
      const operation = check.exists ? 'update' : 'register'

      // 0.5. Fetch actual chain ID from L2 RPC (reuse cached value, or fetch fresh)
      const actualChainId = rpcChainId ?? await fetchRpcChainId(resolveRpcUrl(l2))

      // Determine l2ChainId: for updates use existing (immutable), otherwise prefer RPC value
      let resolvedL2ChainId: number
      if (check.exists && check.l2ChainId) {
        // Update: must preserve immutable l2ChainId from registry
        resolvedL2ChainId = check.l2ChainId
      } else if (actualChainId) {
        // Register: use actual RPC value
        resolvedL2ChainId = actualChainId
      } else {
        // Fallback: use L2Config value
        resolvedL2ChainId = l2.l2ChainId ?? l2.chainId
      }

      // Determine nativeToken: for updates use existing (immutable), otherwise derive from L2Config
      const resolvedNativeToken = (check.exists && check.nativeToken) ? check.nativeToken : undefined

      // 1. Sign metadata
      const timestamp = Math.floor(Date.now() / 1000)
      const signResult = await invoke<{ signature: string; signerAddress: string }>('sign_appchain_metadata', {
        l1ChainId: effectiveL1ChainId,
        l2ChainId: resolvedL2ChainId,
        stackType: 'tokamak-appchain',
        operation,
        identityContract: l2.timelockAddress.toLowerCase(),
        timestamp,
        keychainKey: selectedKey,
      })

      // 2. Build metadata with signature (using resolved immutable fields)
      const metadata = buildAppchainMetadata(l2, publishDesc, socialLinks, signResult.signerAddress, effectiveL1ChainId, {
        l2ChainId: resolvedL2ChainId,
        nativeToken: resolvedNativeToken,
      })
      const nowIso = new Date(timestamp * 1000).toISOString()
      // For updates: preserve original createdAt; for register: use current timestamp
      metadata.createdAt = check.exists && check.createdAt ? check.createdAt : nowIso
      metadata.lastUpdated = nowIso
      metadata.metadata.signature = signResult.signature
      metadata.metadata.signedBy = signResult.signerAddress

      // 3. Submit to platform server
      const result = await submitMetadata(metadata, operation)
      setSubmitResult(result)

      if (result.success) {
        saveDraft(result)
      } else {
        setSubmitError(result.error || 'Submission failed')
      }
    } catch (err) {
      setSubmitError(err instanceof Error ? err.message : String(err))
    } finally {
      setSubmitting(false)
    }
  }

  // Check PR status
  const handleCheckStatus = async () => {
    if (!submitResult?.prNumber) return
    try {
      const status = await getSubmissionStatus(submitResult.prNumber)
      setSubmitResult(prev => prev ? { ...prev, ...status } : prev)
    } catch { /* ignore */ }
  }

  // Fetch real chain ID from L2 RPC on mount
  const [rpcChainId, setRpcChainId] = useState<number | null>(null)
  useEffect(() => {
    const rpcUrl = resolveRpcUrl(l2)
    fetchRpcChainId(rpcUrl).then(setRpcChainId)
  }, [l2.publicRpcUrl, l2.rpcPort, l2.rawConfig])

  // timelockAddress가 핵심 — proposerAddress와 l1ChainId는 폴백 가능
  const canSubmit = !!(l2.timelockAddress && l2.proposerAddress)
  const LOCAL_L1_CHAIN_ID = 9 // Default chain ID for the bundled local L1 node
  const effectiveL1ChainId = l2.l1ChainId || LOCAL_L1_CHAIN_ID
  const displayL2ChainId = rpcChainId ?? l2.l2ChainId ?? l2.chainId

  return (
    <>
      {/* Description */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <div className="flex items-center justify-between">
          <SectionHeader title={ko ? '소개글' : 'Description'} />
          <div className="text-[9px] text-[var(--color-text-secondary)]">
            {saved ? (ko ? '저장됨' : 'Saved') : ''}
          </div>
        </div>
        <textarea
          value={publishDesc}
          onChange={e => handleDescChange(e.target.value)}
          placeholder={ko ? '앱체인을 소개하는 글을 작성하세요. 메타데이터에 포함됩니다.' : 'Describe your appchain. Included in metadata.'}
          rows={4}
          className="w-full mt-1 bg-[var(--color-bg-main)] rounded-lg px-2.5 py-2 text-[11px] outline-none border border-[var(--color-border)] resize-none"
        />
      </div>

      {/* Screenshots */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <SectionHeader title={ko ? '스크린샷' : 'Screenshots'} />
        <input
          ref={fileInputRef}
          type="file"
          accept="image/*"
          multiple
          onChange={handleScreenshotUpload}
          className="hidden"
        />
        <div className="flex gap-2 flex-wrap mt-1">
          {screenshots.map((uri, i) => (
            <div key={i} className="relative group">
              <img
                src={screenshotUrl(uri)}
                alt={`Screenshot ${i + 1}`}
                className="w-20 h-14 rounded-lg border object-cover"
              />
              <button
                onClick={() => removeScreenshot(i)}
                className="absolute -top-1 -right-1 w-4 h-4 bg-red-500 text-white rounded-full text-[8px] flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity cursor-pointer"
              >
                x
              </button>
            </div>
          ))}
          <button
            disabled={uploading}
            onClick={() => fileInputRef.current?.click()}
            className="w-20 h-14 rounded-lg border-2 border-dashed border-[var(--color-border)] flex items-center justify-center text-[var(--color-text-secondary)] hover:border-[#3b82f6] hover:text-[#3b82f6] cursor-pointer transition-colors disabled:opacity-50"
          >
            {uploading ? (
              <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-current" />
            ) : (
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
              </svg>
            )}
          </button>
        </div>
        {!pinataReady && (
          <div className="text-[9px] text-[var(--color-text-secondary)] mt-1">
            {ko ? 'IPFS 업로드는 Settings에서 Pinata 키 설정 필요. 미설정 시 로컬 저장.' : 'IPFS upload requires Pinata key in Settings. Without it, stored locally.'}
          </div>
        )}
        {uploadError && <div className="text-[9px] text-[var(--color-error)] mt-1">{uploadError}</div>}
      </div>

      {/* Social Links */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <SectionHeader title={ko ? '소셜 링크' : 'Social Links'} />
        <div className="mt-1 space-y-1.5">
          {(['website', 'github', 'twitter', 'discord', 'telegram'] as const).map(key => (
            <div key={key} className="flex items-center gap-2">
              <span className="text-[10px] text-[var(--color-text-secondary)] w-14 flex-shrink-0 capitalize">{key}</span>
              <input
                type="text"
                value={socialLinks[key] || ''}
                onChange={e => handleSocialChange(key, e.target.value)}
                placeholder={`https://...`}
                className="flex-1 bg-[var(--color-bg-main)] rounded-lg px-2 py-1.5 text-[10px] outline-none border border-[var(--color-border)]"
              />
            </div>
          ))}
        </div>
      </div>

      {/* Save Draft */}
      <div className="flex items-center justify-between">
        <button
          onClick={saveDraft}
          className="bg-[var(--color-accent)] text-[var(--color-accent-text)] text-[11px] font-medium px-4 py-1.5 rounded-lg cursor-pointer disabled:opacity-50 transition-colors hover:opacity-90"
        >
          {ko ? '저장' : 'Save'}
        </button>
        <span className="text-[9px] text-[var(--color-text-secondary)]">
          {saved ? (ko ? '저장됨 ✓' : 'Saved ✓') : (ko ? '로컬에 저장됩니다' : 'Saved locally')}
        </span>
      </div>

      {/* Metadata Registry Submission */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <SectionHeader title={ko ? '메타데이터 저장소 제출' : 'Metadata Registry Submission'} />
        <div className="text-[9px] text-[var(--color-text-secondary)] mt-1">
          {ko
            ? '메타데이터를 서명하고 GitHub 저장소에 PR을 제출합니다. Deployer 키(SECURITY_COUNCIL)로 서명됩니다.'
            : 'Sign metadata and submit a PR to the GitHub registry. Signed with your deployer key (SECURITY_COUNCIL).'}
        </div>

        {!canSubmit && (
          <p className="text-[9px] text-[var(--color-warning)] mt-2">
            {ko
              ? 'Timelock, OnChainProposer 주소가 필요합니다. 배포 후 컨트랙트 주소가 자동으로 설정됩니다.'
              : 'Timelock and OnChainProposer addresses are required. Contract addresses are set automatically after deployment.'}
          </p>
        )}

        {canSubmit && (
          <div className="mt-2 space-y-2">
            {/* Metadata preview */}
            <div className="bg-[var(--color-bg-main)] rounded-lg p-2 border border-[var(--color-border)] text-[9px] font-mono space-y-0.5">
              <div><span className="text-[var(--color-text-secondary)]">L1 Chain ID:</span> {effectiveL1ChainId}</div>
              <div><span className="text-[var(--color-text-secondary)]">L2 Chain ID:</span> {displayL2ChainId}{rpcChainId && rpcChainId !== (l2.l2ChainId ?? l2.chainId) ? <span className="text-[var(--color-warning)] ml-1">(RPC)</span> : ''}</div>
              <div><span className="text-[var(--color-text-secondary)]">Timelock:</span> {l2.timelockAddress}</div>
              <div><span className="text-[var(--color-text-secondary)]">OnChainProposer:</span> {l2.proposerAddress}</div>
              {l2.bridgeAddress && <div><span className="text-[var(--color-text-secondary)]">CommonBridge:</span> {l2.bridgeAddress}</div>}
              <div><span className="text-[var(--color-text-secondary)]">Stack:</span> tokamak-appchain (zk)</div>
            </div>

            {/* Signing Key Selection */}
            <div className="space-y-1">
              <div className="flex items-center justify-between">
                <div className="text-[10px] font-medium">{ko ? '서명 키 (SECURITY_COUNCIL)' : 'Signing Key (SECURITY_COUNCIL)'}</div>
                <div className="flex items-center gap-1.5">
                  <button
                    onClick={async () => {
                      try {
                        const res = await fetch(`${LOCAL_SERVER}/api/open-url`, {
                          method: 'POST',
                          headers: { 'Content-Type': 'application/json' },
                          body: JSON.stringify({ url: 'keychain-register' }),
                        })
                        const data = await res.json()
                        if (data.ok && data.keyName) {
                          // Reload keychain keys
                          const keysRes = await fetch(`${LOCAL_SERVER}/api/keychain/keys`)
                          const keysData = await keysRes.json()
                          const keys: Array<{ name: string; address: string }> = []
                          for (const name of keysData.keys || []) {
                            try {
                              const infoRes = await fetch(`${LOCAL_SERVER}/api/keychain/keys/${encodeURIComponent(name)}`)
                              const info = await infoRes.json()
                              if (info.address) keys.push({ name, address: info.address })
                            } catch { /* skip */ }
                          }
                          setKeychainKeys(keys)
                          setSelectedKey(data.keyName)
                        }
                      } catch (err) {
                        setSubmitError(err instanceof Error ? err.message : String(err))
                      }
                    }}
                    className="text-[9px] text-[#2563eb] cursor-pointer hover:underline"
                  >
                    {ko ? '+ 키 등록' : '+ Register Key'}
                  </button>
                  <button
                    onClick={async () => {
                      setKeychainLoading(true)
                      try {
                        const res = await fetch(`${LOCAL_SERVER}/api/keychain/keys`)
                        const data = await res.json()
                        const keys: Array<{ name: string; address: string }> = []
                        for (const name of data.keys || []) {
                          try {
                            const infoRes = await fetch(`${LOCAL_SERVER}/api/keychain/keys/${encodeURIComponent(name)}`)
                            const info = await infoRes.json()
                            if (info.address) keys.push({ name, address: info.address })
                          } catch { /* skip */ }
                        }
                        setKeychainKeys(keys)
                      } catch { /* ignore */ }
                      finally { setKeychainLoading(false) }
                    }}
                    className="text-[9px] text-[var(--color-text-secondary)] cursor-pointer hover:text-[var(--color-text)]"
                    title={ko ? '새로고침' : 'Refresh'}
                  >
                    ↻
                  </button>
                </div>
              </div>
              {keychainLoading ? (
                <div className="text-[9px] text-[var(--color-text-secondary)]">{ko ? '키체인 로딩 중...' : 'Loading keychain...'}</div>
              ) : keychainKeys.length === 0 ? (
                <div className="text-[9px] text-[var(--color-text-secondary)]">
                  {ko ? '키체인에 등록된 키가 없습니다.' : 'No keys in keychain.'}
                </div>
              ) : (
                <select
                  value={selectedKey}
                  onChange={e => setSelectedKey(e.target.value)}
                  className="w-full bg-[var(--color-bg-main)] rounded-lg px-2 py-1.5 text-[10px] outline-none border border-[var(--color-border)] cursor-pointer"
                >
                  <option value="">{ko ? '키 선택...' : 'Select key...'}</option>
                  {keychainKeys.map(k => (
                    <option key={k.name} value={k.name}>
                      {k.name} ({k.address.slice(0, 6)}...{k.address.slice(-4)})
                    </option>
                  ))}
                </select>
              )}
            </div>

            {/* Submit button */}
            <button
              disabled={submitting || !selectedKey}
              onClick={handleSubmitMetadata}
              className="w-full py-2 bg-[#2563eb] text-white rounded-lg text-[11px] font-medium disabled:opacity-50 cursor-pointer hover:bg-[#1d4ed8] transition-colors"
            >
              {submitting
                ? (ko ? '서명 및 제출 중...' : 'Signing & Submitting...')
                : (ko ? '서명 & PR 제출' : 'Sign & Submit PR')}
            </button>

            {/* Error */}
            {submitError && (
              <div className="text-[9px] text-[var(--color-error)]">{submitError}</div>
            )}

            {/* Success — PR link */}
            {submitResult?.success && submitResult.prUrl && (
              <div className="bg-[var(--color-bg-main)] rounded-lg p-2 border border-[var(--color-success)]/30">
                <div className="text-[10px] text-[var(--color-success)] font-medium">
                  {(submitResult as any).updated
                    ? (ko ? 'PR 업데이트 완료!' : 'PR Updated!')
                    : (ko ? 'PR 생성 완료!' : 'PR Created!')}
                </div>
                <a
                  href="#"
                  onClick={e => { e.preventDefault(); open(submitResult.prUrl!) }}
                  className="text-[10px] text-[#2563eb] underline break-all"
                >
                  {submitResult.prUrl}
                </a>
                {submitResult.filePath && (
                  <div className="text-[9px] text-[var(--color-text-secondary)] mt-1 font-mono">
                    {submitResult.filePath}
                  </div>
                )}
                <button
                  onClick={handleCheckStatus}
                  className="mt-1.5 text-[9px] text-[var(--color-text-secondary)] underline cursor-pointer"
                >
                  {ko ? '상태 확인' : 'Check Status'}
                </button>
              </div>
            )}

          </div>
        )}
      </div>
    </>
  )
}
