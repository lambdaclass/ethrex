import type React from 'react'
import { useLang } from '../App'
import { t } from '../i18n'
import type { ViewType } from '../App'
import type { NetworkMode } from './CreateL2Wizard'

interface HomeViewProps {
  onNavigate: (view: ViewType) => void
  onCreateWithNetwork: (network: NetworkMode) => void
}

const features: { icon: React.JSX.Element; titleKey: string }[] = [
  {
    icon: (
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z"/>
      </svg>
    ),
    titleKey: 'home.why1',
  },
  {
    icon: (
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
      </svg>
    ),
    titleKey: 'home.why2',
  },
  {
    icon: (
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M12 2a10 10 0 1 0 10 10H12V2z"/><path d="M20 12a8 8 0 0 0-8-8v8h8z"/>
      </svg>
    ),
    titleKey: 'home.why3',
  },
  {
    icon: (
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>
      </svg>
    ),
    titleKey: 'home.why4',
  },
  {
    icon: (
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <polyline points="7 17 2 12 7 7"/><polyline points="17 7 22 12 17 17"/><line x1="2" y1="12" x2="22" y2="12"/>
      </svg>
    ),
    titleKey: 'home.why5',
  },
  {
    icon: (
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <polyline points="20 6 9 17 4 12"/>
      </svg>
    ),
    titleKey: 'home.why6',
  },
]

const steps: { num: string; titleKey: string; descKey: string; badgeColor: string; network: NetworkMode; icon: React.JSX.Element }[] = [
  {
    num: '1',
    titleKey: 'home.step1',
    descKey: 'home.step1.desc',
    badgeColor: 'bg-[var(--color-success)]',
    network: 'local',
    icon: (
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/>
      </svg>
    ),
  },
  {
    num: '2',
    titleKey: 'home.step2',
    descKey: 'home.step2.desc',
    badgeColor: 'bg-[var(--color-warning)]',
    network: 'testnet',
    icon: (
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z"/>
      </svg>
    ),
  },
  {
    num: '3',
    titleKey: 'home.step3',
    descKey: 'home.step3.desc',
    badgeColor: 'bg-[var(--color-accent)]',
    network: 'mainnet',
    icon: (
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>
      </svg>
    ),
  },
]

const quickLinks: { labelKey: string; view: ViewType; icon: React.JSX.Element }[] = [
  {
    labelKey: 'nav.openl2',
    view: 'openl2',
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>
      </svg>
    ),
  },
  {
    labelKey: 'nav.wallet',
    view: 'wallet',
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <rect x="1" y="4" width="22" height="16" rx="2"/><line x1="1" y1="10" x2="23" y2="10"/>
      </svg>
    ),
  },
  {
    labelKey: 'nav.chat',
    view: 'chat',
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
      </svg>
    ),
  },
  {
    labelKey: 'nav.settings',
    view: 'settings',
    icon: (
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
      </svg>
    ),
  },
]

export default function HomeView({ onNavigate, onCreateWithNetwork }: HomeViewProps) {
  const { lang } = useLang()

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      <div className="flex-1 overflow-y-auto">
        {/* Hero Card */}
        <div className="p-4">
          <div className="rounded-2xl bg-[var(--color-accent)] p-6 text-[var(--color-accent-text)]">
            <div className="flex items-center gap-3 mb-3">
              <div className="w-11 h-11 rounded-xl bg-white/20 flex items-center justify-center text-lg font-bold backdrop-blur-sm">
                T
              </div>
              <div>
                <h1 className="text-base font-bold">{t('home.welcome', lang)}</h1>
                <p className="text-[12px] opacity-80">{t('home.subtitle', lang)}</p>
              </div>
            </div>
            <div className="flex gap-2 mt-4">
              <button
                onClick={() => onCreateWithNetwork('local')}
                className="flex-1 bg-white/20 hover:bg-white/30 backdrop-blur-sm rounded-lg py-2 text-[12px] font-medium transition-colors cursor-pointer"
              >
                {t('home.quickStart', lang)}
              </button>
              <button
                onClick={() => onNavigate('openl2')}
                className="flex-1 bg-white/10 hover:bg-white/20 backdrop-blur-sm rounded-lg py-2 text-[12px] font-medium transition-colors cursor-pointer"
              >
                {t('home.explore', lang)}
              </button>
            </div>
          </div>
        </div>

        {/* Why Tokamak - 2 column grid with icons */}
        <div className="px-4 pb-4">
          <h2 className="text-[11px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider mb-2">
            {t('home.whyTitle', lang)}
          </h2>
          <div className="grid grid-cols-2 gap-2">
            {features.map((f) => (
              <div
                key={f.titleKey}
                className="flex items-center gap-2 px-3 py-2.5 rounded-xl bg-[var(--color-bg-sidebar)] border border-[var(--color-border)]"
              >
                <div className="w-7 h-7 rounded-lg bg-[var(--color-bg-main)] flex items-center justify-center flex-shrink-0 text-[var(--color-text-secondary)]">
                  {f.icon}
                </div>
                <span className="text-[11px] font-medium leading-tight">{t(f.titleKey, lang)}</span>
              </div>
            ))}
          </div>
        </div>

        {/* Journey - vertical timeline style */}
        <div className="px-4 pb-4">
          <h2 className="text-[11px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider mb-2">
            {t('home.journey', lang)}
          </h2>
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl border border-[var(--color-border)] overflow-hidden">
            {steps.map((step, i) => (
              <button
                key={step.titleKey}
                onClick={() => onCreateWithNetwork(step.network)}
                className={`w-full flex items-center gap-3 px-4 py-3.5 hover:bg-[var(--color-bg-main)] transition-colors cursor-pointer text-left ${
                  i < steps.length - 1 ? 'border-b border-[var(--color-border)]' : ''
                }`}
              >
                <div className="relative flex-shrink-0">
                  <div className={`w-10 h-10 rounded-xl ${step.badgeColor} flex items-center justify-center text-white`}>
                    {step.icon}
                  </div>
                  <span className="absolute -top-1 -right-1 w-4 h-4 rounded-full bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] flex items-center justify-center text-[9px] font-bold text-[var(--color-text-secondary)]">
                    {step.num}
                  </span>
                </div>
                <div className="flex-1 min-w-0">
                  <div className="text-[13px] font-semibold">{t(step.titleKey, lang)}</div>
                  <div className="text-[11px] text-[var(--color-text-secondary)] mt-0.5">{t(step.descKey, lang)}</div>
                </div>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--color-text-secondary)] flex-shrink-0">
                  <polyline points="9 18 15 12 9 6"/>
                </svg>
              </button>
            ))}
          </div>
        </div>

        {/* Quick Links */}
        <div className="px-4 pb-6">
          <h2 className="text-[11px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider mb-2">
            {t('home.quickLinks', lang)}
          </h2>
          <div className="grid grid-cols-4 gap-2">
            {quickLinks.map((link) => (
              <button
                key={link.view}
                onClick={() => onNavigate(link.view)}
                className="flex flex-col items-center gap-1.5 py-3 rounded-xl bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] hover:bg-[var(--color-border)] transition-colors cursor-pointer"
              >
                <div className="text-[var(--color-text-secondary)]">{link.icon}</div>
                <span className="text-[10px] font-medium text-[var(--color-text-secondary)]">{t(link.labelKey, lang)}</span>
              </button>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}
