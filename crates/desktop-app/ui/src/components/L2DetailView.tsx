import { useState } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'
import type { L2Config } from './MyL2View'

interface Props {
  l2: L2Config
  onBack: () => void
}

type DetailTab = 'control' | 'logs' | 'config' | 'dashboard'

const statusColor = (status: string) => {
  if (status === 'running') return 'var(--color-success)'
  if (status === 'starting') return 'var(--color-warning)'
  return 'var(--color-text-secondary)'
}

export default function L2DetailView({ l2, onBack }: Props) {
  const { lang } = useLang()
  const [activeTab, setActiveTab] = useState<DetailTab>('control')

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
    <div className="flex flex-col h-full bg-[var(--color-bg-chat)]">
      {/* Header */}
      <div className="px-6 py-4 border-b border-[var(--color-border)]">
        <div className="flex items-center gap-3 mb-3">
          <button onClick={onBack} className="text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer">
            ← {t('openl2.back', lang)}
          </button>
        </div>
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-xl bg-[var(--color-bubble-ai)] flex items-center justify-center text-xl">
            {l2.icon}
          </div>
          <div>
            <div className="font-semibold">{l2.name}</div>
            <div className="text-xs text-[var(--color-text-secondary)]">
              Chain #{l2.chainId} · {l2.nativeToken}
              {l2.isPublic && <span className="ml-2 text-[var(--color-accent)]">{t('myl2.public', lang)}</span>}
            </div>
          </div>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex border-b border-[var(--color-border)] px-4">
        {tabs.map(tab => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`px-4 py-3 text-sm transition-colors cursor-pointer border-b-2 ${
              activeTab === tab.id
                ? 'border-[var(--color-accent)] text-[var(--color-accent)]'
                : 'border-transparent text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'
            }`}
          >
            {t(tab.labelKey, lang)}
          </button>
        ))}
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-6">
        {activeTab === 'control' && (
          <div className="space-y-4">
            {/* Quick Actions */}
            <div className="grid grid-cols-3 gap-3">
              <button className="bg-[var(--color-success)] text-black text-sm font-medium py-3 rounded-xl hover:opacity-80 transition-opacity cursor-pointer">
                {t('myl2.detail.startAll', lang)}
              </button>
              <button className="bg-[var(--color-error)] text-white text-sm font-medium py-3 rounded-xl hover:opacity-80 transition-opacity cursor-pointer">
                {t('myl2.detail.stopAll', lang)}
              </button>
              <button className="bg-[var(--color-border)] text-sm font-medium py-3 rounded-xl hover:opacity-80 transition-opacity cursor-pointer">
                {t('myl2.detail.restart', lang)}
              </button>
            </div>

            {/* Process List */}
            {processes.map(proc => (
              <div key={proc.name} className="bg-[var(--color-bubble-ai)] rounded-xl p-5 flex items-center justify-between">
                <div className="flex items-center gap-4">
                  <div className="w-3 h-3 rounded-full" style={{ backgroundColor: statusColor(proc.status) }} />
                  <div>
                    <div className="font-medium">{proc.name}</div>
                    <div className="text-xs text-[var(--color-text-secondary)] mt-0.5">
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
          <div className="bg-black rounded-xl p-4 font-mono text-xs text-green-400 h-full min-h-[400px] overflow-auto">
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
          <div className="space-y-4">
            {[
              { label: t('myl2.detail.configName', lang), value: l2.name },
              { label: 'Chain ID', value: String(l2.chainId) },
              { label: t('myl2.detail.configToken', lang), value: l2.nativeToken },
              { label: 'L1 RPC', value: l2.l1Rpc },
              { label: t('myl2.detail.configPort', lang), value: String(l2.rpcPort) },
              { label: t('myl2.detail.configDesc', lang), value: l2.description },
            ].map(({ label, value }) => (
              <div key={label} className="bg-[var(--color-bubble-ai)] rounded-xl p-4">
                <label className="text-xs text-[var(--color-text-secondary)] block mb-1">{label}</label>
                <input
                  type="text"
                  defaultValue={value}
                  className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none"
                />
              </div>
            ))}
            <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4">
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">{t('myl2.detail.configHashtags', lang)}</label>
              <div className="flex flex-wrap gap-2 mt-1">
                {l2.hashtags.map(tag => (
                  <span key={tag} className="text-xs bg-[var(--color-border)] px-3 py-1 rounded-full text-[var(--color-accent)]">
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
            <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4 flex items-center justify-between">
              <div>
                <div className="text-sm font-medium">{t('myl2.detail.configPublic', lang)}</div>
                <div className="text-xs text-[var(--color-text-secondary)]">{t('myl2.detail.configPublicDesc', lang)}</div>
              </div>
              <div className={`w-12 h-6 rounded-full flex items-center px-1 cursor-pointer transition-colors ${l2.isPublic ? 'bg-[var(--color-accent)]' : 'bg-[var(--color-border)]'}`}>
                <div className={`w-4 h-4 bg-white rounded-full transition-transform ${l2.isPublic ? 'translate-x-6' : ''}`} />
              </div>
            </div>
            <button className="w-full bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] rounded-xl py-3 text-sm font-medium transition-colors cursor-pointer">
              {t('settings.save', lang)}
            </button>
          </div>
        )}

        {activeTab === 'dashboard' && (
          <div className="flex items-center justify-center h-full min-h-[300px]">
            <div className="text-center space-y-3">
              <div className="text-4xl">📊</div>
              <div className="text-lg font-medium">{l2.name} {t('dashboard.title', lang)}</div>
              <div className="text-sm text-[var(--color-text-secondary)]">
                <code className="bg-[var(--color-border)] px-2 py-1 rounded">http://localhost:{l2.rpcPort + 1000}</code>
              </div>
              <p className="text-xs text-[var(--color-text-secondary)] whitespace-pre-line">
                {t('dashboard.hint', lang)}
              </p>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
