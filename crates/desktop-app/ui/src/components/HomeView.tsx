import { useState, useEffect, useCallback } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'
import type { ViewType } from '../App'
import type { NetworkMode } from './CreateL2Wizard'
import { invoke } from '@tauri-apps/api/core'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'

interface HomeViewProps {
  onNavigate: (view: ViewType) => void
  onCreateWithNetwork: (network: NetworkMode) => void
}

interface MiniDeployment {
  id: string
  name: string
  program_slug: string
  chain_id: number | null
  status: string
  phase: string
  config: string | null
  l1_port: number | null
  l2_port: number | null
}

async function openDeployManager(view?: string) {
  const baseUrl = await invoke<string>('open_deployment_ui')
  const url = view ? `${baseUrl}?view=${view}` : baseUrl
  const existing = await WebviewWindow.getByLabel('deploy-manager')
  if (existing) {
    if (view) {
      try { await existing.emit('navigate-view', view) } catch {}
      try { await (existing as any).eval(`showView('${view}')`) } catch {}
    }
    await existing.show()
    await existing.setFocus()
  } else {
    new WebviewWindow('deploy-manager', {
      url,
      title: 'Tokamak L2 Manager',
      width: 1100, height: 800,
      minWidth: 800, minHeight: 600,
      center: true,
    })
  }
}

export default function HomeView({ onNavigate }: HomeViewProps) {
  const { lang } = useLang()
  const ko = lang === 'ko'
  const [deployments, setDeployments] = useState<MiniDeployment[]>([])

  const loadDeployments = useCallback(async () => {
    try {
      const rows = await invoke<MiniDeployment[]>('list_docker_deployments')
      setDeployments(rows)
    } catch { /* ignore */ }
  }, [])

  useEffect(() => {
    loadDeployments()
    const interval = setInterval(loadDeployments, 10000)
    return () => clearInterval(interval)
  }, [loadDeployments])

  const runningCount = deployments.filter(d => d.status === 'active' || d.status === 'running').length
  const totalCount = deployments.length

  const getNetworkMode = (d: MiniDeployment) => {
    try {
      const cfg = d.config ? JSON.parse(d.config) : {}
      return cfg.mode || 'local'
    } catch { return 'local' }
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      <div className="flex-1 overflow-y-auto">
        {/* Hero */}
        <div className="p-4">
          <div className="rounded-2xl bg-[var(--color-accent)] p-5 text-[var(--color-accent-text)]">
            <div className="flex items-center gap-3">
              <div className="w-11 h-11 rounded-xl bg-white/20 flex items-center justify-center text-lg font-bold backdrop-blur-sm">
                T
              </div>
              <div>
                <h1 className="text-base font-bold">{t('home.welcome', lang)}</h1>
                <p className="text-[12px] opacity-80">{t('home.subtitle', lang)}</p>
              </div>
            </div>
          </div>
        </div>

        {/* My Appchains Status */}
        <div className="px-4 pb-4">
          <div className="flex items-center justify-between mb-2">
            <h2 className="text-[11px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider">
              {ko ? '내 앱체인 현황' : 'My Appchains'}
            </h2>
            {totalCount > 0 && (
              <span className="text-[10px] text-[var(--color-text-secondary)]">
                {runningCount}/{totalCount} {ko ? '실행 중' : 'running'}
              </span>
            )}
          </div>

          {totalCount === 0 ? (
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl border border-[var(--color-border)] p-6 text-center">
              <div className="text-2xl mb-2">🔗</div>
              <div className="text-[13px] font-medium">{ko ? '아직 앱체인이 없습니다' : 'No appchains yet'}</div>
              <div className="text-[11px] text-[var(--color-text-secondary)] mt-1">
                {ko ? 'L2 매니저에서 첫 앱체인을 만들어보세요' : 'Create your first appchain in L2 Manager'}
              </div>
              <button
                onClick={() => openDeployManager('launch')}
                className="mt-3 bg-[var(--color-success)] text-black text-[12px] font-medium px-4 py-2 rounded-lg hover:opacity-80 transition-opacity cursor-pointer"
              >
                {ko ? '새 앱체인 만들기' : 'Create Appchain'}
              </button>
            </div>
          ) : (
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl border border-[var(--color-border)] overflow-hidden">
              {deployments.map((d, i) => {
                const mode = getNetworkMode(d)
                const isRunning = d.status === 'active' || d.status === 'running'
                return (
                  <button
                    key={d.id}
                    onClick={() => onNavigate('myl2')}
                    className={`w-full flex items-center gap-3 px-3.5 py-3 hover:bg-[var(--color-bg-main)] transition-colors cursor-pointer text-left ${i > 0 ? 'border-t border-[var(--color-border)]' : ''}`}
                  >
                    <div className="w-8 h-8 rounded-lg bg-[var(--color-bg-main)] flex items-center justify-center text-sm flex-shrink-0 border border-[var(--color-border)]">
                      {d.program_slug === 'zk-dex' ? '🔐' : '⛓️'}
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-1.5">
                        <span className="text-[12px] font-medium truncate">{d.name}</span>
                        <span className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${isRunning ? 'bg-[var(--color-success)]' : 'bg-[var(--color-text-secondary)]'}`} />
                      </div>
                      <div className="text-[10px] text-[var(--color-text-secondary)] mt-0.5">
                        {d.chain_id ? `Chain ID: ${d.chain_id}` : d.phase}
                      </div>
                    </div>
                    <div className="flex flex-col items-end gap-1 flex-shrink-0">
                      <span className={`text-[9px] px-1.5 py-0.5 rounded font-medium ${
                        mode === 'testnet'
                          ? 'text-black bg-[var(--color-warning)]'
                          : 'text-white bg-[#6366f1]'
                      }`}>
                        {mode === 'testnet' ? 'Testnet' : 'Local'}
                      </span>
                      <span className={`text-[10px] ${isRunning ? 'text-[var(--color-success)]' : 'text-[var(--color-text-secondary)]'}`}>
                        {isRunning ? (ko ? '실행 중' : 'Running') : (ko ? '중지' : 'Stopped')}
                      </span>
                    </div>
                  </button>
                )
              })}
              {/* Add new */}
              <button
                onClick={() => openDeployManager('launch')}
                className="w-full flex items-center gap-3 px-3.5 py-2.5 hover:bg-[var(--color-bg-main)] transition-colors cursor-pointer text-left border-t border-[var(--color-border)]"
              >
                <div className="w-8 h-8 rounded-lg bg-[var(--color-success)]/10 flex items-center justify-center flex-shrink-0">
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--color-success)" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
                  </svg>
                </div>
                <span className="text-[11px] text-[var(--color-success)] font-medium">
                  {ko ? '새 앱체인 만들기' : 'Create New Appchain'}
                </span>
              </button>
            </div>
          )}
        </div>

        {/* Appchain Journey */}
        <div className="px-4 pb-4">
          <h2 className="text-[11px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider mb-2">
            {t('home.journey', lang)}
          </h2>
          <div className="space-y-2">
            {[
              { step: '1', label: t('home.step1', lang), desc: t('home.step1.desc', lang), done: deployments.some(d => getNetworkMode(d) === 'local') },
              { step: '2', label: t('home.step2', lang), desc: t('home.step2.desc', lang), done: deployments.some(d => getNetworkMode(d) === 'testnet') },
              { step: '3', label: t('home.step3', lang), desc: t('home.step3.desc', lang), done: false },
            ].map(item => (
              <div key={item.step} className={`flex items-center gap-3 px-3.5 py-3 rounded-xl border ${item.done ? 'bg-[var(--color-success)]/5 border-[var(--color-success)]/20' : 'bg-[var(--color-bg-sidebar)] border-[var(--color-border)]'}`}>
                <div className={`w-7 h-7 rounded-full flex items-center justify-center text-[11px] font-bold flex-shrink-0 ${item.done ? 'bg-[var(--color-success)] text-white' : 'bg-[var(--color-bg-main)] text-[var(--color-text-secondary)] border border-[var(--color-border)]'}`}>
                  {item.done ? '✓' : item.step}
                </div>
                <div className="flex-1 min-w-0">
                  <div className={`text-[12px] font-medium ${item.done ? 'text-[var(--color-success)]' : ''}`}>{item.label}</div>
                  <div className="text-[10px] text-[var(--color-text-secondary)]">{item.desc}</div>
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* Features */}
        <div className="px-4 pb-6">
          <h2 className="text-[11px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider mb-2">
            {ko ? '주요 기능' : 'Key Features'}
          </h2>
          <div className="grid grid-cols-3 gap-2">
            <button onClick={() => onNavigate('chat')} className="flex flex-col items-center gap-1.5 px-2 py-3 rounded-xl bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] hover:bg-[var(--color-bg-main)] transition-colors cursor-pointer text-center">
              <span className="text-base">🤖</span>
              <div>
                <div className="text-[11px] font-medium">{ko ? '앱체인 Pilot' : 'Appchain Pilot'}</div>
                <div className="text-[9px] text-[var(--color-text-secondary)]">{ko ? 'AI 어시스턴트' : 'AI Assistant'}</div>
              </div>
            </button>
            <button onClick={() => onNavigate('wallet')} className="flex flex-col items-center gap-1.5 px-2 py-3 rounded-xl bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] hover:bg-[var(--color-bg-main)] transition-colors cursor-pointer text-center">
              <span className="text-base">🔄</span>
              <div>
                <div className="text-[11px] font-medium">{ko ? 'AI 위임' : 'AI Delegation'}</div>
                <div className="text-[9px] text-[var(--color-text-secondary)]">{ko ? '운영 자동화' : 'Auto-ops'}</div>
              </div>
            </button>
            <button onClick={() => onNavigate('openl2')} className="flex flex-col items-center gap-1.5 px-2 py-3 rounded-xl bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] hover:bg-[var(--color-bg-main)] transition-colors cursor-pointer text-center">
              <span className="text-base">🌐</span>
              <div>
                <div className="text-[11px] font-medium">{ko ? '오픈 앱체인' : 'Open Appchains'}</div>
                <div className="text-[9px] text-[var(--color-text-secondary)]">{ko ? '공개 체인 탐색' : 'Explore public'}</div>
              </div>
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
