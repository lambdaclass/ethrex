import { useState } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'

const defaultTabs = [
  { id: 'l1', name: 'L1 Dashboard', url: 'http://localhost:3010' },
  { id: 'l2', name: 'L2 Dashboard', url: 'http://localhost:3011' },
  { id: 'explorer', name: 'Explorer', url: 'http://localhost:3012' },
  { id: 'prover', name: 'Prover', url: 'http://localhost:3013' },
  { id: 'metrics', name: 'Metrics', url: 'http://localhost:3000' },
]

export default function DashboardView() {
  const { lang } = useLang()
  const [activeTab, setActiveTab] = useState(defaultTabs[0].id)
  const currentTab = defaultTabs.find(t => t.id === activeTab)

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-chat)]">
      <div className="flex border-b border-[var(--color-border)] px-4">
        {defaultTabs.map(tab => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`px-4 py-3 text-sm transition-colors cursor-pointer border-b-2 ${
              activeTab === tab.id
                ? 'border-[var(--color-accent)] text-[var(--color-accent)]'
                : 'border-transparent text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'
            }`}
          >
            {tab.name}
          </button>
        ))}
      </div>

      <div className="flex-1 flex items-center justify-center">
        <div className="text-center space-y-3">
          <div className="text-4xl">📊</div>
          <div className="text-lg font-medium">{currentTab?.name}</div>
          <div className="text-sm text-[var(--color-text-secondary)]">
            {t('dashboard.webview', lang)}: <code className="bg-[var(--color-border)] px-2 py-1 rounded">{currentTab?.url}</code>
          </div>
          <p className="text-xs text-[var(--color-text-secondary)] mt-4 whitespace-pre-line">
            {t('dashboard.hint', lang)}
          </p>
        </div>
      </div>
    </div>
  )
}
