import { useState, useEffect, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLang } from '../App'
import { t } from '../i18n'

interface SetupStep {
  id: string
  label: string
  status: 'pending' | 'inprogress' | 'done' | 'error' | 'skipped'
}

interface SetupProgress {
  steps: SetupStep[]
  current_step: number
  logs: string[]
  error: string | null
}

interface Props {
  chainId: string
  chainName: string
  chainIcon: string
  onDone: () => void
  onCancel: () => void
}

export default function SetupProgressView({ chainId, chainName, chainIcon, onDone, onCancel }: Props) {
  const { lang } = useLang()
  const [progress, setProgress] = useState<SetupProgress | null>(null)
  const [showLogs, setShowLogs] = useState(false)
  const logEndRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const poll = setInterval(async () => {
      try {
        const p = await invoke<SetupProgress>('get_setup_progress', { id: chainId })
        setProgress(p)
      } catch {
        // not ready yet
      }
    }, 1000)
    return () => clearInterval(poll)
  }, [chainId])

  // Auto-scroll logs
  useEffect(() => {
    if (showLogs && logEndRef.current) {
      logEndRef.current.scrollIntoView({ behavior: 'smooth' })
    }
  }, [progress?.logs.length, showLogs])

  const stepIcon = (status: string) => {
    switch (status) {
      case 'done': return (
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="var(--color-success)" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
          <polyline points="20 6 9 17 4 12"/>
        </svg>
      )
      case 'inprogress': return (
        <div className="w-4 h-4 border-2 border-[var(--color-accent)] border-t-transparent rounded-full animate-spin" />
      )
      case 'error': return (
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="var(--color-error)" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
          <line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
        </svg>
      )
      case 'skipped': return (
        <div className="w-4 h-4 rounded-full bg-[var(--color-text-secondary)] opacity-30" />
      )
      default: return (
        <div className="w-4 h-4 rounded-full border-2 border-[var(--color-border)]" />
      )
    }
  }

  const stepLabel = (step: SetupStep) => {
    const key = `setup.step.${step.id}`
    const translated = t(key, lang)
    return translated !== key ? translated : step.label
  }

  const allDone = progress?.steps.every(s => s.status === 'done' || s.status === 'skipped')

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      {/* Header */}
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
        <h1 className="text-base font-semibold">{t('setup.title', lang)}</h1>
      </div>

      <div className="flex-1 overflow-y-auto p-4">
        {/* Chain info */}
        <div className="flex items-center gap-3 mb-6">
          <div className="w-12 h-12 rounded-xl bg-[var(--color-bg-sidebar)] flex items-center justify-center text-2xl">
            {chainIcon}
          </div>
          <div>
            <div className="text-base font-semibold">{chainName}</div>
            <div className="text-[11px] text-[var(--color-text-secondary)]">ID: {chainId.slice(0, 8)}...</div>
          </div>
        </div>

        {/* Steps */}
        <div className="space-y-3 mb-6">
          {progress?.steps.map((step) => (
            <div key={step.id} className="flex items-center gap-3">
              <div className="flex-shrink-0">
                {stepIcon(step.status)}
              </div>
              <span className={`text-[13px] ${
                step.status === 'done' ? 'text-[var(--color-success)]' :
                step.status === 'inprogress' ? 'text-[var(--color-text-primary)] font-medium' :
                step.status === 'error' ? 'text-[var(--color-error)]' :
                'text-[var(--color-text-secondary)]'
              }`}>
                {stepLabel(step)}
              </span>
            </div>
          ))}
        </div>

        {/* Phase 1A notice */}
        <div className="bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] rounded-xl p-3 mb-4">
          <p className="text-[12px] text-[var(--color-text-secondary)]">
            {t('setup.waiting', lang)}
          </p>
        </div>

        {/* Logs toggle */}
        <button
          onClick={() => setShowLogs(!showLogs)}
          className="text-[12px] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer mb-2"
        >
          {t('setup.viewLogs', lang)} {showLogs ? '▲' : '▼'}
        </button>

        {showLogs && (
          <div className="bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] rounded-xl p-3 max-h-40 overflow-y-auto">
            {progress?.logs.map((log, i) => (
              <div key={i} className="text-[11px] font-mono text-[var(--color-text-secondary)] leading-relaxed">
                {log}
              </div>
            ))}
            {(!progress?.logs.length) && (
              <div className="text-[11px] text-[var(--color-text-secondary)]">No logs yet...</div>
            )}
            <div ref={logEndRef} />
          </div>
        )}

        {progress?.error && (
          <div className="mt-3 bg-red-50 border border-[var(--color-error)] rounded-xl p-3">
            <p className="text-[12px] text-[var(--color-error)]">{progress.error}</p>
          </div>
        )}
      </div>

      {/* Actions */}
      <div className="px-4 py-3 border-t border-[var(--color-border)] flex justify-end gap-2">
        {!allDone && (
          <button
            onClick={onCancel}
            className="px-4 py-2 rounded-xl text-[13px] bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] hover:bg-[var(--color-border)] cursor-pointer"
          >
            {t('setup.cancel', lang)}
          </button>
        )}
        {allDone && (
          <button
            onClick={onDone}
            className="px-4 py-2 rounded-xl text-[13px] bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] text-[var(--color-accent-text)] font-medium cursor-pointer"
          >
            {t('setup.goToChain', lang)}
          </button>
        )}
      </div>
    </div>
  )
}
