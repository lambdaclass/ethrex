import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLang } from '../App'
import { t } from '../i18n'
import { platformAPI } from '../api/platform'
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
  const [publishScreenshots, setPublishScreenshots] = useState<string[]>([])

  // Sync isPublic when parent re-fetches
  useEffect(() => { setIsPublic(l2.isPublic) }, [l2.isPublic])

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
                    const r = await platformAPI.registerDeployment({ programId: 'ethrex-appchain', name: l2.name, chainId: l2.chainId, rpcUrl: `http://localhost:${l2.rpcPort}` })
                    await platformAPI.activateDeployment(r.deployment.id)
                    setIsPublic(true)
                    await invoke('update_appchain_public', { id: l2.id, isPublic: true })
                    onRefresh?.()
                  } catch (e: unknown) { setPublishError(e instanceof Error ? e.message : String(e)) }
                  finally { setPublishing(false) }
                } else {
                  setIsPublic(false)
                  try { await invoke('update_appchain_public', { id: l2.id, isPublic: false }); onRefresh?.() }
                  catch { /* ignore */ }
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
          <SectionHeader title={ko ? '소개글' : 'Description'} />
          <textarea
            value={publishDesc}
            onChange={e => setPublishDesc(e.target.value)}
            placeholder={ko ? '앱체인을 소개하는 글을 작성하세요. 다른 사용자에게 보여집니다.' : 'Describe your appchain. This is shown to other users.'}
            rows={4}
            className="w-full mt-1 bg-[var(--color-bg-main)] rounded-lg px-2.5 py-2 text-[11px] outline-none border border-[var(--color-border)] resize-none"
          />
        </div>

        <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
          <SectionHeader title={ko ? '스크린샷' : 'Screenshots'} />
          <div className="flex gap-2 flex-wrap mt-1">
            {publishScreenshots.map((_, i) => (
              <div key={i} className="relative w-20 h-14 rounded-lg bg-[var(--color-bg-main)] border border-[var(--color-border)] flex items-center justify-center">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--color-text-secondary)]">
                  <rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/>
                </svg>
                <button
                  onClick={() => setPublishScreenshots(publishScreenshots.filter((_, j) => j !== i))}
                  className="absolute -top-1 -right-1 w-4 h-4 rounded-full bg-[var(--color-error)] text-white text-[8px] flex items-center justify-center cursor-pointer"
                >×</button>
              </div>
            ))}
            {publishScreenshots.length < 5 && (
              <button
                onClick={() => setPublishScreenshots([...publishScreenshots, `screenshot-${Date.now()}`])}
                className="w-20 h-14 rounded-lg border-2 border-dashed border-[var(--color-border)] flex items-center justify-center text-[var(--color-text-secondary)] hover:border-[#3b82f6] hover:text-[#3b82f6] cursor-pointer transition-colors"
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
                </svg>
              </button>
            )}
          </div>
          <div className="text-[9px] text-[var(--color-text-secondary)] mt-1">
            {ko ? '앱체인의 화면 캡쳐를 추가하세요 (최대 5장)' : 'Add screenshots of your appchain (max 5)'}
          </div>
        </div>
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
