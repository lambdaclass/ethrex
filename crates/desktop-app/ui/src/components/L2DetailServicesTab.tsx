import { useState } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'
import { platformAPI } from '../api/platform'
import { invoke } from '@tauri-apps/api/core'
import { SectionHeader, KV } from './ui-atoms'
import type { L2Config } from './MyL2View'
import type { ContainerInfo, Product } from './L2DetailView'

const SERVICE_NAME_PREFIXES = ['tokamak-app-', 'zk-dex-tools-'] as const

const CORE_SERVICES = [
  { label: 'L1 Node', service: 'tokamak-app-l1', portKey: 'l1Port' as const },
  { label: 'L2 Node', service: 'tokamak-app-l2', portKey: 'l2Port' as const },
  { label: 'Prover', service: 'tokamak-app-prover', portKey: null },
]

const TOOLS_SERVICES = [
  { label: 'L1 Explorer', service: 'frontend-l1' },
  { label: 'L2 Explorer', service: 'frontend-l2' },
  { label: 'Dashboard', service: 'bridge-ui' },
]

interface Props {
  l2: L2Config
  ko: boolean
  containers: ContainerInfo[]
  products: Product[]
  actionLoading: boolean
  handleAction: (action: 'start' | 'stop') => void
  platformLoggedIn: boolean
  onRefresh?: () => void
}

export default function L2DetailServicesTab({
  l2, ko, containers, products, actionLoading, handleAction,
  platformLoggedIn, onRefresh,
}: Props) {
  const { lang } = useLang()
  const [isPublic, setIsPublic] = useState(l2.isPublic)
  const [publishing, setPublishing] = useState(false)
  const [publishError, setPublishError] = useState('')
  const [publishDesc, setPublishDesc] = useState('')
  const [publishScreenshots, setPublishScreenshots] = useState<string[]>([])

  const stripPrefixes = (s: string) =>
    SERVICE_NAME_PREFIXES.reduce((acc, p) => acc.replace(p, ''), s)

  const svcState = (svc: string): string => {
    const c = containers.find(c => c.service === svc || c.name?.includes(stripPrefixes(svc)))
    return c ? (c.state || 'stopped') : 'stopped'
  }

  const svcPort = (svc: string): string | null => {
    const c = containers.find(c => c.service === svc || c.name?.includes(stripPrefixes(svc)))
    if (!c?.ports) return null
    const m = c.ports.match(/0\.0\.0\.0:(\d+)/)
    return m ? `:${m[1]}` : null
  }

  const dotColor = (state: string) => {
    if (state === 'running') return 'var(--color-success)'
    if (state === 'restarting') return 'var(--color-warning)'
    return 'var(--color-text-secondary)'
  }

  return (
    <>
      {/* Docker Services */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl border border-[var(--color-border)] overflow-hidden">
        <div className="px-3 pt-3 pb-1">
          <SectionHeader title={ko ? '서비스 상태' : 'Service Status'} />
        </div>
        {/* Core */}
        <div className="px-3 pb-1">
          <span className="text-[9px] uppercase tracking-wider text-[var(--color-text-secondary)] font-medium">Core</span>
        </div>
        {CORE_SERVICES.map(svc => {
          const state = svcState(svc.service)
          const running = state === 'running'
          const port = svc.portKey ? (l2[svc.portKey] ? `:${l2[svc.portKey]}` : null) : null
          const displayPort = port || svcPort(svc.service)
          return (
            <div key={svc.service} className="flex items-center gap-2 px-3 py-2 border-t border-[var(--color-border)]">
              <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ backgroundColor: dotColor(state) }} />
              <span className="text-[12px] font-medium flex-shrink-0">{svc.label}</span>
              <span className={`text-[11px] ${running ? 'text-[var(--color-success)]' : 'text-[var(--color-text-secondary)]'}`}>{state}</span>
              {displayPort && <code className="text-[10px] font-mono text-[#3b82f6] ml-auto">{displayPort}</code>}
            </div>
          )
        })}
        {/* Tools */}
        <div className="px-3 pt-2 pb-1 border-t border-[var(--color-border)]">
          <span className="text-[9px] uppercase tracking-wider text-[var(--color-text-secondary)] font-medium">Tools</span>
        </div>
        {TOOLS_SERVICES.map(svc => {
          const state = svcState(svc.service)
          const running = state === 'running'
          const port = svcPort(svc.service)
          return (
            <div key={svc.service} className="flex items-center gap-2 px-3 py-2 border-t border-[var(--color-border)]">
              <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ backgroundColor: dotColor(state) }} />
              <span className="text-[12px] font-medium flex-shrink-0">{svc.label}</span>
              <span className={`text-[11px] ${running ? 'text-[var(--color-success)]' : 'text-[var(--color-text-secondary)]'}`}>{state}</span>
              {port && running && (
                <a href={`http://localhost${port}`} target="_blank" rel="noopener noreferrer"
                  className="text-[10px] font-mono text-[#3b82f6] ml-auto hover:underline" onClick={e => e.stopPropagation()}>
                  {port} ↗
                </a>
              )}
            </div>
          )
        })}
      </div>

      {/* Actions — show contextual button based on container state */}
      {(() => {
        const allStopped = [...CORE_SERVICES, ...TOOLS_SERVICES].every(svc => svcState(svc.service) !== 'running')
        const anyRunning = [...CORE_SERVICES, ...TOOLS_SERVICES].some(svc => svcState(svc.service) === 'running')
        return (
          <div className="flex gap-2">
            {allStopped ? (
              <button disabled={actionLoading} onClick={() => handleAction('start')}
                className="flex-1 bg-[var(--color-success)] text-black text-xs font-medium py-2 rounded-xl hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-50">
                {actionLoading ? '...' : (ko ? '전체 시작' : 'Start All')}
              </button>
            ) : anyRunning ? (
              <button disabled={actionLoading} onClick={() => handleAction('stop')}
                className="flex-1 bg-[var(--color-error)] text-white text-xs font-medium py-2 rounded-xl hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-50">
                {actionLoading ? '...' : (ko ? '전체 중지' : 'Stop All')}
              </button>
            ) : null}
          </div>
        )
      })()}

      {/* Products */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <SectionHeader title={ko ? '탑재 제품' : 'Products'} />
        <div className="mt-1 space-y-1.5">
          {products.map(p => (
            <div key={p.name} className="flex items-center gap-2 bg-[var(--color-bg-main)] rounded-lg px-2.5 py-2 border border-[var(--color-border)]">
              <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ backgroundColor: p.status === 'active' ? 'var(--color-success)' : 'var(--color-text-secondary)' }} />
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-1.5">
                  <span className="text-[12px] font-medium">{p.name}</span>
                  <span className="text-[9px] text-[var(--color-tag-text)] bg-[var(--color-tag-bg)] px-1.5 py-0.5 rounded">{p.type}</span>
                </div>
                <div className="text-[10px] text-[var(--color-text-secondary)] truncate">{p.description}</div>
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Contracts */}
      {(l2.bridgeAddress || l2.proposerAddress) && (
        <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
          <SectionHeader title={ko ? '컨트랙트' : 'Contracts'} />
          <div className="mt-1 space-y-1.5">
            {l2.bridgeAddress && <KV label="Bridge" value={l2.bridgeAddress} mono />}
            {l2.proposerAddress && <KV label="Proposer" value={l2.proposerAddress} mono />}
          </div>
        </div>
      )}

      {/* Settings */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <SectionHeader title={ko ? '설정' : 'Settings'} />
        <div className="mt-1 space-y-1.5">
          <KV label="Chain ID" value={l2.chainId ? String(l2.chainId) : '-'} mono />
          <KV label="L1 RPC" value={l2.l1Rpc || '-'} mono />
          <KV label="Docker" value={l2.dockerProject || '-'} mono />
          <KV label={ko ? '생성일' : 'Created'} value={new Date(l2.createdAt).toLocaleDateString()} />
        </div>
        {/* Public Toggle */}
        <div className="mt-2 pt-2 border-t border-[var(--color-border)] flex items-center justify-between">
          <div>
            <div className="text-[11px] font-medium">{t('myl2.detail.configPublic', lang)}</div>
            <div className="text-[9px] text-[var(--color-text-secondary)]">{t('myl2.detail.configPublicDesc', lang)}</div>
          </div>
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
        {publishError && <p className="text-[9px] text-[var(--color-error)] mt-1">{publishError}</p>}
        {publishing && <p className="text-[9px] text-[var(--color-text-secondary)] mt-1">{ko ? '등록 중...' : 'Registering...'}</p>}

        {/* Publish Details (shown when public is on) */}
        {isPublic && (
          <div className="mt-2 pt-2 border-t border-[var(--color-border)] space-y-2">
            <div>
              <div className="text-[11px] font-medium mb-1">{ko ? '소개글' : 'Description'}</div>
              <textarea
                value={publishDesc}
                onChange={e => setPublishDesc(e.target.value)}
                placeholder={ko ? '앱체인을 소개하는 글을 작성하세요. 다른 사용자에게 보여집니다.' : 'Describe your appchain. This is shown to other users.'}
                rows={3}
                className="w-full bg-[var(--color-bg-main)] rounded-lg px-2.5 py-2 text-[11px] outline-none border border-[var(--color-border)] resize-none"
              />
            </div>
            <div>
              <div className="text-[11px] font-medium mb-1">{ko ? '스크린샷' : 'Screenshots'}</div>
              <div className="flex gap-2 flex-wrap">
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
                <button
                  onClick={() => setPublishScreenshots([...publishScreenshots, `screenshot-${Date.now()}`])}
                  className="w-20 h-14 rounded-lg border-2 border-dashed border-[var(--color-border)] flex items-center justify-center text-[var(--color-text-secondary)] hover:border-[#3b82f6] hover:text-[#3b82f6] cursor-pointer transition-colors"
                >
                  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
                  </svg>
                </button>
              </div>
              <div className="text-[9px] text-[var(--color-text-secondary)] mt-1">
                {ko ? '앱체인의 화면 캡쳐를 추가하세요 (최대 5장)' : 'Add screenshots of your appchain (max 5)'}
              </div>
            </div>
          </div>
        )}
      </div>
    </>
  )
}
