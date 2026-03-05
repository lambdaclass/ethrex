import { useLang } from '../App'
import { t } from '../i18n'
import type { ViewType } from '../App'
import type { NetworkMode } from './CreateL2Wizard'

interface HomeViewProps {
  onNavigate: (view: ViewType) => void
  onCreateWithNetwork: (network: NetworkMode) => void
}

const whyKeys = [
  'home.why1', 'home.why2', 'home.why3',
  'home.why4', 'home.why5', 'home.why6',
]

const steps: { num: string; titleKey: string; descKey: string; color: string; network: NetworkMode; icon: JSX.Element }[] = [
  {
    num: '1',
    titleKey: 'home.step1',
    descKey: 'home.step1.desc',
    color: 'bg-[var(--color-success)]',
    network: 'local',
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/>
      </svg>
    ),
  },
  {
    num: '2',
    titleKey: 'home.step2',
    descKey: 'home.step2.desc',
    color: 'bg-[var(--color-warning)]',
    network: 'testnet',
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z"/>
      </svg>
    ),
  },
  {
    num: '3',
    titleKey: 'home.step3',
    descKey: 'home.step3.desc',
    color: 'bg-[var(--color-accent)]',
    network: 'mainnet',
    icon: (
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>
      </svg>
    ),
  },
]

export default function HomeView({ onNavigate, onCreateWithNetwork }: HomeViewProps) {
  const { lang } = useLang()

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      <div className="flex-1 overflow-y-auto">
        {/* Hero */}
        <div className="px-6 pt-10 pb-5 text-center">
          <div className="w-14 h-14 mx-auto mb-3 rounded-2xl bg-[var(--color-accent)] flex items-center justify-center text-xl font-bold text-[var(--color-accent-text)] shadow-md">
            T
          </div>
          <h1 className="text-lg font-bold">{t('home.welcome', lang)}</h1>
          <p className="text-[13px] text-[var(--color-text-secondary)] mt-1">{t('home.subtitle', lang)}</p>
        </div>

        {/* Why */}
        <div className="px-4 pb-5">
          <h2 className="text-[11px] font-semibold text-[var(--color-text-secondary)] uppercase tracking-wide mb-2 px-1">
            {t('home.whyTitle', lang)}
          </h2>
          <div className="flex flex-wrap gap-1.5">
            {whyKeys.map((key) => (
              <span
                key={key}
                className="text-[11px] px-2.5 py-1.5 rounded-lg bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] text-[var(--color-text-primary)]"
              >
                {t(key, lang)}
              </span>
            ))}
          </div>
        </div>

        {/* Journey */}
        <div className="px-4 pb-8">
          <h2 className="text-[11px] font-semibold text-[var(--color-text-secondary)] uppercase tracking-wide mb-2 px-1">
            {t('home.journey', lang)}
          </h2>
          <div className="space-y-2">
            {steps.map((step, i) => (
              <button
                key={step.titleKey}
                onClick={() => onCreateWithNetwork(step.network)}
                className="w-full flex items-center gap-3 p-3 rounded-xl bg-[var(--color-bg-sidebar)] hover:bg-[var(--color-border)] border border-[var(--color-border)] transition-colors cursor-pointer text-left"
              >
                <div className={`w-9 h-9 rounded-lg ${step.color} flex items-center justify-center flex-shrink-0 text-white`}>
                  {step.icon}
                </div>
                <div className="flex-1 min-w-0">
                  <div className="text-[13px] font-medium flex items-center gap-1.5">
                    <span className="text-[10px] text-[var(--color-text-secondary)]">STEP {step.num}</span>
                    {t(step.titleKey, lang)}
                  </div>
                  <div className="text-[11px] text-[var(--color-text-secondary)] mt-0.5">{t(step.descKey, lang)}</div>
                </div>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--color-text-secondary)] flex-shrink-0">
                  <polyline points="9 18 15 12 9 6"/>
                </svg>
              </button>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}
