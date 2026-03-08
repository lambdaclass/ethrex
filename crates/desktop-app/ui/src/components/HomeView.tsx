import type React from 'react'
import { useState } from 'react'
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

async function openDeployManager() {
  const url = await invoke<string>('open_deployment_ui')
  const existing = await WebviewWindow.getByLabel('deploy-manager')
  if (existing) {
    await existing.show()
    await existing.setFocus()
  } else {
    new WebviewWindow('deploy-manager', {
      url,
      title: 'Tokamak L2 Manager',
      width: 1100,
      height: 800,
      minWidth: 800,
      minHeight: 600,
      center: true,
    })
  }
}

export default function HomeView({ onNavigate }: HomeViewProps) {
  const { lang } = useLang()
  const [deploymentOpened, setDeploymentOpened] = useState(false)

  const handleOpenDeploy = async () => {
    try {
      await openDeployManager()
      setDeploymentOpened(true)
    } catch (e) {
      console.error('Failed to open deployment UI:', e)
    }
  }

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
                onClick={handleOpenDeploy}
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

        {/* Create New Appchain */}
        <div className="px-4 pb-4">
          <h2 className="text-[11px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider mb-2">
            {lang === 'ko' ? '앱체인 관리' : 'Appchain Management'}
          </h2>
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl border border-[var(--color-border)] overflow-hidden">
            <button
              onClick={handleOpenDeploy}
              className="w-full flex items-center gap-3 px-4 py-3.5 hover:bg-[var(--color-bg-main)] transition-colors cursor-pointer text-left border-b border-[var(--color-border)]"
            >
              <div className="w-10 h-10 rounded-xl bg-[var(--color-success)] flex items-center justify-center text-white flex-shrink-0">
                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
                </svg>
              </div>
              <div className="flex-1 min-w-0">
                <div className="text-[13px] font-semibold">
                  {lang === 'ko' ? '새 앱체인 만들기' : 'Create New Appchain'}
                </div>
                <div className="text-[11px] text-[var(--color-text-secondary)] mt-0.5">
                  {lang === 'ko' ? 'L2 체인을 배포하고 관리합니다' : 'Deploy and manage your L2 chain'}
                </div>
              </div>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--color-text-secondary)] flex-shrink-0">
                <polyline points="9 18 15 12 9 6"/>
              </svg>
            </button>
            <button
              onClick={() => onNavigate('myl2')}
              className="w-full flex items-center gap-3 px-4 py-3.5 hover:bg-[var(--color-bg-main)] transition-colors cursor-pointer text-left"
            >
              <div className="w-10 h-10 rounded-xl bg-[#2563eb] flex items-center justify-center text-white flex-shrink-0">
                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/>
                </svg>
              </div>
              <div className="flex-1 min-w-0">
                <div className="text-[13px] font-semibold">
                  {lang === 'ko' ? '내 앱체인' : 'My Appchains'}
                </div>
                <div className="text-[11px] text-[var(--color-text-secondary)] mt-0.5">
                  {lang === 'ko' ? '기존 앱체인 목록 보기' : 'View your existing appchains'}
                </div>
              </div>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--color-text-secondary)] flex-shrink-0">
                <polyline points="9 18 15 12 9 6"/>
              </svg>
            </button>
          </div>
        </div>

        {/* Deployment UI opened banner */}
        {deploymentOpened && (
          <div className="px-4 pb-4">
            <div className="flex items-center gap-3 px-4 py-3 rounded-xl bg-[#122b1e] border border-[#1a4d2e]">
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="#22c55e" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="flex-shrink-0">
                <rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/>
              </svg>
              <div className="flex-1 min-w-0">
                <div className="text-[13px] font-semibold text-[#22c55e]">
                  {lang === 'ko' ? '배포 매니저가 열렸습니다' : 'Deployment Manager opened'}
                </div>
                <div className="text-[11px] text-[#22c55e]/70 mt-0.5">
                  {lang === 'ko' ? '별도 창에서 L2 체인을 배포하고 관리할 수 있습니다' : 'Deploy and manage L2 chains in the separate window'}
                </div>
              </div>
              <button
                onClick={() => setDeploymentOpened(false)}
                className="text-[#22c55e]/50 hover:text-[#22c55e] cursor-pointer flex-shrink-0"
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
                </svg>
              </button>
            </div>
          </div>
        )}

        {/* Quick Links */}
        <div className="px-4 pb-6">
          <h2 className="text-[11px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider mb-2">
            {t('home.quickLinks', lang)}
          </h2>
          <div className="grid grid-cols-5 gap-2">
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
