import { useState, useEffect, useRef, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLang } from '../App'
import { t } from '../i18n'
import { platformAPI } from '../api/platform'
import { localServerAPI } from '../api/local-server'
import { uploadFileToIPFS, ipfsToHttp, uploadJSONToIPFS, buildMetadata, isPinataConfigured } from '../api/ipfs'
import { SectionHeader } from './ui-atoms'
import type { L2Config } from './MyL2View'

interface Props {
  l2: L2Config
  ko: boolean
  platformLoggedIn: boolean
  onRefresh?: () => void
}

export default function L2DetailPublishTab({ l2, ko, platformLoggedIn, onRefresh }: Props) {
  const { lang } = useLang()
  const [isPublic, setIsPublic] = useState(l2.isPublic)
  const [publishing, setPublishing] = useState(false)
  const [publishError, setPublishError] = useState('')
  const [publishDesc, setPublishDesc] = useState('')
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const [socialLinks, setSocialLinks] = useState<Record<string, string>>({})
  const [screenshots, setScreenshots] = useState<string[]>([])
  const [uploading, setUploading] = useState(false)
  const [uploadError, setUploadError] = useState('')
  const [pinataReady, setPinataReady] = useState(false)
  const [metadataUploading, setMetadataUploading] = useState(false)
  const [metadataCID, setMetadataCID] = useState('')
  const fileInputRef = useRef<HTMLInputElement>(null)
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const socialTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Sync isPublic when parent re-fetches
  useEffect(() => { setIsPublic(l2.isPublic) }, [l2.isPublic])

  // Check Pinata configuration
  useEffect(() => { isPinataConfigured().then(setPinataReady) }, [])

  // Load existing data from Platform on mount (if already published)
  useEffect(() => {
    if (!l2.platformDeploymentId || !l2.isPublic) return
    platformAPI.getPublicAppchain(l2.platformDeploymentId).then(appchain => {
      if (appchain?.description) setPublishDesc(appchain.description)
      if (appchain?.social_links && Object.keys(appchain.social_links).length > 0) {
        setSocialLinks(appchain.social_links)
      }
      if (appchain?.screenshots && appchain.screenshots.length > 0) {
        setScreenshots(appchain.screenshots)
      }
    }).catch((err) => console.warn('[publish] Failed to load appchain data:', err))
  }, [l2.platformDeploymentId, l2.isPublic])

  // Auto-save description with debounce (guard against concurrent saves)
  const savingRef = useRef(false)
  const saveDescription = useCallback(async (desc: string) => {
    const platformId = l2.platformDeploymentId
    if (!platformId || !isPublic || savingRef.current) return
    savingRef.current = true
    setSaving(true)
    try {
      await platformAPI.updateDeployment(platformId, { description: desc })
      setSaved(true)
      setTimeout(() => setSaved(false), 2000)
    } catch (err) {
      console.warn('[publish] Failed to save description:', err)
    } finally {
      setSaving(false)
      savingRef.current = false
    }
  }, [l2.platformDeploymentId, isPublic])

  const handleDescChange = (value: string) => {
    setPublishDesc(value)
    setSaved(false)
    if (saveTimerRef.current) clearTimeout(saveTimerRef.current)
    saveTimerRef.current = setTimeout(() => saveDescription(value), 1500)
  }

  // Save social links with debounce
  const saveSocialLinks = useCallback(async (links: Record<string, string>) => {
    const platformId = l2.platformDeploymentId
    if (!platformId || !isPublic) return
    // Filter out empty values
    const filtered = Object.fromEntries(Object.entries(links).filter(([, v]) => v.trim()))
    setSaving(true)
    try {
      await platformAPI.updateDeployment(platformId, { social_links: JSON.stringify(filtered) })
      setSaved(true)
      setTimeout(() => setSaved(false), 2000)
    } catch (err) { console.warn('[publish] Failed to save social links:', err) }
    finally { setSaving(false) }
  }, [l2.platformDeploymentId, isPublic])

  const handleSocialChange = (key: string, value: string) => {
    const updated = { ...socialLinks, [key]: value }
    setSocialLinks(updated)
    setSaved(false)
    if (socialTimerRef.current) clearTimeout(socialTimerRef.current)
    socialTimerRef.current = setTimeout(() => saveSocialLinks(updated), 1500)
  }

  // Screenshot upload handler
  const handleScreenshotUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files
    if (!files || files.length === 0) return
    setUploading(true)
    setUploadError('')
    try {
      const newUris: string[] = []
      for (const file of Array.from(files)) {
        const uri = await uploadFileToIPFS(file)
        newUris.push(uri)
      }
      const updated = [...screenshots, ...newUris]
      setScreenshots(updated)
      // Save to Platform
      const platformId = l2.platformDeploymentId
      if (platformId) {
        await platformAPI.updateDeployment(platformId, { screenshots: JSON.stringify(updated) })
      }
    } catch (err) {
      setUploadError(err instanceof Error ? err.message : String(err))
    } finally {
      setUploading(false)
      if (fileInputRef.current) fileInputRef.current.value = ''
    }
  }

  const removeScreenshot = async (index: number) => {
    const updated = screenshots.filter((_, i) => i !== index)
    setScreenshots(updated)
    const platformId = l2.platformDeploymentId
    if (platformId) {
      try {
        await platformAPI.updateDeployment(platformId, { screenshots: JSON.stringify(updated) })
      } catch (err) {
        console.warn('Failed to sync screenshot removal:', err)
      }
    }
  }

  // Upload full metadata JSON to IPFS (for on-chain metadataURI)
  const handleUploadMetadata = async () => {
    setMetadataUploading(true)
    setUploadError('')
    try {
      const rpcUrl = l2.publicRpcUrl || `http://localhost:${l2.rpcPort}`
      const metadata = buildMetadata({
        name: l2.name,
        description: publishDesc || undefined,
        chainId: l2.chainId,
        rpcUrl,
        networkMode: l2.networkMode || 'local',
        l1ChainId: l2.l1ChainId || 1,
        proposerAddress: l2.proposerAddress || undefined,
        bridgeAddress: l2.bridgeAddress || undefined,
        screenshots,
        socialLinks: Object.fromEntries(Object.entries(socialLinks).filter(([, v]) => v.trim())),
        explorerUrl: l2.toolsL2ExplorerPort ? `http://localhost:${l2.toolsL2ExplorerPort}` : undefined,
        bridgeUIUrl: l2.toolsBridgeUIPort ? `http://localhost:${l2.toolsBridgeUIPort}` : undefined,
      })
      const cid = await uploadJSONToIPFS(metadata)
      setMetadataCID(cid)
    } catch (err) {
      setUploadError(err instanceof Error ? err.message : String(err))
    } finally {
      setMetadataUploading(false)
    }
  }

  // Cleanup debounce timers on unmount
  useEffect(() => {
    return () => {
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current)
      if (socialTimerRef.current) clearTimeout(socialTimerRef.current)
    }
  }, [])

  return (
    <>
      {/* Public Toggle */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <SectionHeader title={ko ? '오픈 앱체인 공개' : 'Open Appchain Publishing'} />
        <div className="mt-2 flex items-center justify-between">
          <div>
            <div className="text-[11px] font-medium">{t('myl2.detail.configPublic', lang)}</div>
            <div className="text-[9px] text-[var(--color-text-secondary)]">{t('myl2.detail.configPublicDesc', lang)}</div>
          </div>
          <div className="flex items-center gap-2">
            {isPublic && <span className="text-[9px] text-[var(--color-success)] font-medium">{ko ? '공개 중' : 'Public'}</span>}
            <button
              disabled={publishing || (l2.networkMode === 'local')}
              onClick={async () => {
                if (!isPublic) {
                  if (!platformLoggedIn) { setPublishError(ko ? 'Platform 로그인 필요' : 'Login required'); return }
                  setPublishing(true); setPublishError('')
                  try {
                    const r = await platformAPI.registerDeployment({
                      programId: 'ethrex-appchain',
                      name: l2.name,
                      chainId: l2.chainId,
                      rpcUrl: `http://localhost:${l2.rpcPort}`,
                    })
                    const platformId = r.deployment.id

                    // Update Platform deployment with chain details + service URLs
                    const explorerUrl = l2.toolsL2ExplorerPort ? `http://localhost:${l2.toolsL2ExplorerPort}` : undefined
                    const dashboardUrl = l2.toolsBridgeUIPort ? `http://localhost:${l2.toolsBridgeUIPort}` : undefined
                    await platformAPI.updateDeployment(platformId, {
                      bridge_address: l2.bridgeAddress || undefined,
                      proposer_address: l2.proposerAddress || undefined,
                      network_mode: l2.networkMode || 'local',
                      l1_chain_id: l2.l1ChainId || undefined,
                      explorer_url: explorerUrl,
                      dashboard_url: dashboardUrl,
                    })

                    await platformAPI.activateDeployment(platformId)
                    setIsPublic(true)

                    // Save platformDeploymentId to local DB
                    try {
                      await localServerAPI.updateDeployment(l2.id, {
                        is_public: 1,
                        platform_deployment_id: platformId,
                      })
                    } catch {
                      // Fallback: try Rust appchain manager (for non-Docker appchains)
                      await invoke('update_appchain_public', { id: l2.id, isPublic: true, platformDeploymentId: platformId })
                    }
                    onRefresh?.()
                  } catch (e: unknown) { setPublishError(e instanceof Error ? e.message : String(e)) }
                  finally { setPublishing(false) }
                } else {
                  setIsPublic(false)
                  // Deactivate on Platform
                  if (l2.platformDeploymentId) {
                    try { await platformAPI.updateDeployment(l2.platformDeploymentId, { status: 'inactive' }) } catch (err) { console.warn('[publish] Failed to deactivate:', err) }
                  }
                  // Clear local DB
                  try {
                    await localServerAPI.updateDeployment(l2.id, {
                      is_public: 0,
                      platform_deployment_id: null,
                    })
                  } catch {
                    try { await invoke('update_appchain_public', { id: l2.id, isPublic: false }) } catch (err) { console.warn('[publish] Fallback unpublish failed:', err) }
                  }
                  onRefresh?.()
                }
              }}
              className={`w-10 h-5 rounded-full flex items-center px-0.5 cursor-pointer transition-colors disabled:opacity-50 flex-shrink-0 ${isPublic ? 'bg-[var(--color-accent)]' : 'bg-[var(--color-border)]'}`}
            >
              <div className={`w-4 h-4 bg-white rounded-full transition-transform ${isPublic ? 'translate-x-5' : ''}`} />
            </button>
          </div>
        </div>
        {publishError && <p className="text-[9px] text-[var(--color-error)] mt-1">{publishError}</p>}
        {publishing && <p className="text-[9px] text-[var(--color-text-secondary)] mt-1">{ko ? '등록 중...' : 'Registering...'}</p>}
      </div>

      {/* Publish Details (shown when public) */}
      {isPublic && (<>
        <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
          <div className="flex items-center justify-between">
            <SectionHeader title={ko ? '소개글' : 'Description'} />
            <div className="text-[9px] text-[var(--color-text-secondary)]">
              {saving ? (ko ? '저장 중...' : 'Saving...') : saved ? (ko ? '저장됨' : 'Saved') : ''}
            </div>
          </div>
          <textarea
            value={publishDesc}
            onChange={e => handleDescChange(e.target.value)}
            placeholder={ko ? '앱체인을 소개하는 글을 작성하세요. 다른 사용자에게 보여집니다.' : 'Describe your appchain. This is shown to other users.'}
            rows={4}
            className="w-full mt-1 bg-[var(--color-bg-main)] rounded-lg px-2.5 py-2 text-[11px] outline-none border border-[var(--color-border)] resize-none"
          />
        </div>

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
                  src={ipfsToHttp(uri)}
                  alt={`Screenshot ${i + 1}`}
                  className="w-20 h-14 rounded-lg border object-cover"
                />
                <button
                  onClick={() => removeScreenshot(i)}
                  className="absolute -top-1 -right-1 w-4 h-4 bg-red-500 text-white rounded-full text-[8px] flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity"
                >
                  x
                </button>
              </div>
            ))}
            <button
              disabled={uploading || !pinataReady}
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
            <div className="text-[9px] text-[var(--color-warning)] mt-1">
              {ko ? 'Settings에서 Pinata API 키를 설정하세요' : 'Set Pinata API key in Settings'}
            </div>
          )}
          {uploadError && <div className="text-[9px] text-[var(--color-error)] mt-1">{uploadError}</div>}
        </div>

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

        {/* On-chain Metadata (Phase 2) */}
        {l2.proposerAddress && (
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <SectionHeader title={ko ? '온체인 메타데이터' : 'On-chain Metadata'} />
            <div className="text-[9px] text-[var(--color-text-secondary)] mt-1">
              {ko
                ? '메타데이터를 IPFS에 업로드하고 OnChainProposer에 등록합니다.'
                : 'Upload metadata to IPFS and register it on OnChainProposer.'}
            </div>
            <div className="mt-2 space-y-2">
              <button
                disabled={metadataUploading || !pinataReady}
                onClick={handleUploadMetadata}
                className="w-full py-1.5 bg-[var(--color-accent)] text-white rounded-lg text-[10px] font-medium disabled:opacity-50"
              >
                {metadataUploading
                  ? (ko ? 'IPFS 업로드 중...' : 'Uploading to IPFS...')
                  : (ko ? '메타데이터 IPFS 업로드' : 'Upload Metadata to IPFS')}
              </button>
              {metadataCID && (
                <div className="bg-[var(--color-bg-main)] rounded-lg p-2 border border-[var(--color-border)]">
                  <div className="text-[9px] text-[var(--color-text-secondary)]">Metadata CID</div>
                  <div className="text-[10px] font-mono break-all mt-0.5">{metadataCID}</div>
                  <button
                    onClick={async () => {
                      try {
                        const result = await invoke<string>('set_metadata_uri', {
                          l1RpcUrl: l2.testnetL1RpcUrl || `http://localhost:${l2.l1Port || 8545}`,
                          proposerAddress: l2.proposerAddress || '',
                          metadataUri: metadataCID,
                          keychainKey: `deployer_pk_${l2.id}`,
                        })
                        const txData = JSON.parse(result)
                        // Copy calldata for wallet signing
                        await navigator.clipboard.writeText(JSON.stringify(txData, null, 2))
                        setSaved(true)
                        setTimeout(() => setSaved(false), 3000)
                      } catch (err) {
                        setUploadError(err instanceof Error ? err.message : String(err))
                      }
                    }}
                    className="mt-1.5 w-full py-1.5 bg-purple-600 text-white rounded-lg text-[10px] font-medium hover:bg-purple-700"
                  >
                    {ko ? 'L1 트랜잭션 데이터 준비' : 'Prepare L1 Transaction Data'}
                  </button>
                </div>
              )}
            </div>
          </div>
        )}
      </>)}

      {/* Not public hint */}
      {!isPublic && (
        <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)] text-center">
          <div className="text-2xl mb-2">🌐</div>
          <div className="text-[12px] font-medium">{ko ? '앱체인을 공개하면' : 'When you publish'}</div>
          <div className="text-[11px] text-[var(--color-text-secondary)] mt-1 space-y-0.5">
            <div>{ko ? '· 다른 사용자가 내 앱체인을 발견할 수 있습니다' : '· Others can discover your appchain'}</div>
            <div>{ko ? '· 소개글과 스크린샷을 등록할 수 있습니다' : '· You can add a description and screenshots'}</div>
            <div>{ko ? '· 커뮤니티 피드백을 받을 수 있습니다' : '· You can receive community feedback'}</div>
          </div>
        </div>
      )}
    </>
  )
}
