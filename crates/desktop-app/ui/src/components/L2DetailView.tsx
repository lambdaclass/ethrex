import { useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLang } from '../App'
import { t } from '../i18n'
import type { L2Config } from './MyL2View'

interface Props {
  l2: L2Config
  onBack: () => void
  onRefresh?: () => void
}

type DetailTab = 'control' | 'logs' | 'config' | 'dashboard'

const statusColor = (status: string) => {
  if (status === 'running') return 'var(--color-success)'
  if (status === 'starting') return 'var(--color-warning)'
  return 'var(--color-text-secondary)'
}

export default function L2DetailView({ l2, onBack, onRefresh }: Props) {
  const { lang } = useLang()
  const [activeTab, setActiveTab] = useState<DetailTab>('control')

  const handleStop = async () => {
    try {
      await invoke('stop_appchain', { id: l2.id })
      onRefresh?.()
    } catch (e) {
      console.error('Failed to stop appchain:', e)
    }
  }

  const tabs: { id: DetailTab; labelKey: string }[] = [
    { id: 'control', labelKey: 'myl2.detail.control' },
    { id: 'logs', labelKey: 'myl2.detail.logs' },
    { id: 'config', labelKey: 'myl2.detail.config' },
    { id: 'dashboard', labelKey: 'myl2.detail.dashboard' },
  ]

  const processes = [
    { name: t('myl2.sequencer', lang), status: l2.sequencerStatus },
    { name: t('myl2.prover', lang), status: l2.proverStatus },
    { name: 'L2 Client', status: l2.status === 'running' ? 'running' : 'stopped' },
  ]

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      {/* Header */}
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
        <div className="flex items-center gap-3 mb-2">
          <button onClick={onBack} className="text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer">
            ← {t('openl2.back', lang)}
          </button>
        </div>
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-xl bg-[var(--color-bg-sidebar)] flex items-center justify-center text-xl border border-[var(--color-border)]">
            {l2.icon}
          </div>
          <div>
            <div className="text-[13px] font-semibold">{l2.name}</div>
            <div className="text-[11px] text-[var(--color-text-secondary)]">
              Chain #{l2.chainId} · {l2.nativeToken}
              {l2.isPublic && <span className="ml-2 text-[var(--color-accent)]">{t('myl2.public', lang)}</span>}
            </div>
          </div>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex border-b border-[var(--color-border)] px-2">
        {tabs.map(tab => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`px-3 py-2.5 text-[13px] transition-colors cursor-pointer border-b-2 ${
              activeTab === tab.id
                ? 'border-[var(--color-text-primary)] text-[var(--color-text-primary)] font-medium'
                : 'border-transparent text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'
            }`}
          >
            {t(tab.labelKey, lang)}
          </button>
        ))}
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-4">
        {activeTab === 'control' && (
          <div className="space-y-3">
            {/* Quick Actions */}
            <div className="grid grid-cols-3 gap-2">
              <button className="bg-[var(--color-success)] text-black text-xs font-medium py-2.5 rounded-xl hover:opacity-80 transition-opacity cursor-pointer">
                {t('myl2.detail.startAll', lang)}
              </button>
              <button
                onClick={handleStop}
                className="bg-[var(--color-error)] text-white text-xs font-medium py-2.5 rounded-xl hover:opacity-80 transition-opacity cursor-pointer"
              >
                {t('myl2.detail.stopAll', lang)}
              </button>
              <button className="bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] text-xs font-medium py-2.5 rounded-xl hover:bg-[var(--color-border)] transition-colors cursor-pointer">
                {t('myl2.detail.restart', lang)}
              </button>
            </div>

            {/* Process List */}
            {processes.map(proc => (
              <div key={proc.name} className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 flex items-center justify-between border border-[var(--color-border)]">
                <div className="flex items-center gap-3">
                  <div className="w-3 h-3 rounded-full" style={{ backgroundColor: statusColor(proc.status) }} />
                  <div>
                    <div className="text-[13px] font-medium">{proc.name}</div>
                    <div className="text-[11px] text-[var(--color-text-secondary)] mt-0.5">
                      {t(`myl2.status.${proc.status}`, lang)}
                    </div>
                  </div>
                </div>
                <div className="flex gap-2">
                  <button
                    disabled={proc.status === 'running'}
                    className="bg-[var(--color-success)] text-black text-xs font-medium px-4 py-2 rounded-lg disabled:opacity-30 hover:opacity-80 transition-opacity cursor-pointer"
                  >
                    {t('nodes.start', lang)}
                  </button>
                  <button
                    disabled={proc.status === 'stopped'}
                    className="bg-[var(--color-error)] text-white text-xs font-medium px-4 py-2 rounded-lg disabled:opacity-30 hover:opacity-80 transition-opacity cursor-pointer"
                  >
                    {t('nodes.stop', lang)}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}

        {activeTab === 'logs' && (
          <div className="bg-black rounded-xl p-4 font-mono text-[11px] text-green-400 h-full min-h-[400px] overflow-auto border border-[var(--color-border)]">
            <div className="text-[var(--color-text-secondary)]">[{l2.name}] {t('myl2.detail.logsPlaceholder', lang)}</div>
            <div className="mt-2 text-gray-500">$ ethrex --chain-id {l2.chainId} --port {l2.rpcPort}</div>
            <div className="text-gray-500">INFO: Starting sequencer...</div>
            <div className="text-gray-500">INFO: Listening on 0.0.0.0:{l2.rpcPort}</div>
            <div className="text-gray-500">INFO: Block #1 produced</div>
            <div className="text-gray-500">INFO: Block #2 produced</div>
            <div className="animate-pulse mt-1">▊</div>
          </div>
        )}

        {activeTab === 'config' && (
          <div className="space-y-3">
            {[
              { label: t('myl2.detail.configName', lang), value: l2.name },
              { label: 'Chain ID', value: String(l2.chainId) },
              { label: t('myl2.detail.configToken', lang), value: l2.nativeToken },
              { label: 'L1 RPC', value: l2.l1Rpc },
              { label: t('myl2.detail.configPort', lang), value: String(l2.rpcPort) },
              { label: t('myl2.detail.configDesc', lang), value: l2.description },
            ].map(({ label, value }) => (
              <div key={label} className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
                <label className="text-xs text-[var(--color-text-secondary)] block mb-1">{label}</label>
                <input
                  type="text"
                  defaultValue={value}
                  className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)]"
                />
              </div>
            ))}
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">{t('myl2.detail.configHashtags', lang)}</label>
              <div className="flex flex-wrap gap-2 mt-1">
                {l2.hashtags.map(tag => (
                  <span key={tag} className="text-[11px] bg-[var(--color-tag-bg)] px-2 py-0.5 rounded text-[var(--color-tag-text)]">
                    #{tag} ×
                  </span>
                ))}
                <input
                  type="text"
                  placeholder="+ tag"
                  className="bg-transparent text-sm outline-none w-20"
                />
              </div>
            </div>
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)] flex items-center justify-between">
              <div>
                <div className="text-[13px] font-medium">{t('myl2.detail.configPublic', lang)}</div>
                <div className="text-[11px] text-[var(--color-text-secondary)]">{t('myl2.detail.configPublicDesc', lang)}</div>
              </div>
              <div className={`w-10 h-5 rounded-full flex items-center px-0.5 cursor-pointer transition-colors ${l2.isPublic ? 'bg-[var(--color-accent)]' : 'bg-[var(--color-border)]'}`}>
                <div className={`w-4 h-4 bg-white rounded-full transition-transform ${l2.isPublic ? 'translate-x-5' : ''}`} />
              </div>
            </div>
            <button className="w-full bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] rounded-xl py-2.5 text-[13px] font-medium transition-colors cursor-pointer text-[var(--color-accent-text)]">
              {t('settings.save', lang)}
            </button>
          </div>
        )}

        {activeTab === 'dashboard' && (
          <div className="flex items-center justify-center h-full min-h-[300px]">
            <div className="text-center space-y-3">
              <div className="w-12 h-12 mx-auto rounded-xl bg-[var(--color-bg-sidebar)] flex items-center justify-center border border-[var(--color-border)]">
                <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--color-text-secondary)]">
                  <rect x="3" y="3" width="7" height="9" rx="1"/><rect x="14" y="3" width="7" height="5" rx="1"/><rect x="14" y="12" width="7" height="9" rx="1"/><rect x="3" y="16" width="7" height="5" rx="1"/>
                </svg>
              </div>
              <div className="text-sm font-medium">{l2.name} {t('dashboard.title', lang)}</div>
              <div className="text-[11px] text-[var(--color-text-secondary)]">
                <code className="bg-[var(--color-bg-sidebar)] px-2 py-0.5 rounded text-[11px] border border-[var(--color-border)]">http://localhost:{l2.rpcPort + 1000}</code>
              </div>
              <button
                onClick={async () => {
                  try { await invoke('open_deployment_ui') }
                  catch (e) { console.error('Failed to open deployment UI:', e) }
                }}
                className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] text-[var(--color-accent-text)] text-xs font-medium px-5 py-2.5 rounded-xl transition-colors cursor-pointer"
              >
                {lang === 'ko' ? 'Docker 배포 관리 열기' : 'Open Docker Deployment Manager'}
              </button>
              <p className="text-[11px] text-[var(--color-text-secondary)] whitespace-pre-line">
                {t('dashboard.hint', lang)}
              </p>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
