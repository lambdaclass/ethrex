import { useState } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'

const defaultTabs = [
  { id: 'l1', labelKey: 'dashboard.tab.l1', url: 'http://localhost:3010' },
  { id: 'l2', labelKey: 'dashboard.tab.l2', url: 'http://localhost:3011' },
  { id: 'explorer', labelKey: 'dashboard.tab.explorer', url: 'http://localhost:3012' },
  { id: 'prover', labelKey: 'dashboard.tab.prover', url: 'http://localhost:3013' },
  { id: 'metrics', labelKey: 'dashboard.tab.metrics', url: 'http://localhost:3000' },
]

export default function DashboardView() {
  const { lang } = useLang()
  const [activeTab, setActiveTab] = useState(defaultTabs[0].id)
  const currentTab = defaultTabs.find(tab => tab.id === activeTab)

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      <div className="flex border-b border-[var(--color-border)] px-2 overflow-x-auto bg-[var(--color-bg-sidebar)]">
        {defaultTabs.map(tab => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`px-3 py-2.5 text-[13px] whitespace-nowrap transition-colors cursor-pointer border-b-2 ${
              activeTab === tab.id
                ? 'border-[var(--color-text-primary)] text-[var(--color-text-primary)] font-medium'
                : 'border-transparent text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'
            }`}
          >
            {t(tab.labelKey, lang)}
          </button>
        ))}
      </div>

      <div className="flex-1 flex items-center justify-center">
        <div className="text-center space-y-3 px-6">
          <div className="w-12 h-12 mx-auto rounded-xl bg-[var(--color-bg-sidebar)] flex items-center justify-center">
            <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--color-text-secondary)]">
              <rect x="3" y="3" width="7" height="9" rx="1"/><rect x="14" y="3" width="7" height="5" rx="1"/><rect x="14" y="12" width="7" height="9" rx="1"/><rect x="3" y="16" width="7" height="5" rx="1"/>
            </svg>
          </div>
          <div className="text-sm font-medium">{t(currentTab?.labelKey ?? '', lang)} {t('dashboard.title', lang)}</div>
          <div className="text-xs text-[var(--color-text-secondary)]">
            {t('dashboard.webview', lang)}: <code className="bg-[var(--color-bg-sidebar)] px-2 py-0.5 rounded text-[11px] border border-[var(--color-border)]">{currentTab?.url}</code>
          </div>
          <p className="text-xs text-[var(--color-text-secondary)] mt-4">
            {t('dashboard.hint', lang)}
          </p>
        </div>
      </div>
    </div>
  )
}
