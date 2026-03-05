import { useState } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'

interface Props {
  onBack: () => void
  onCreate: (config: Record<string, string>) => void
}

const steps = ['myl2.wizard.step1', 'myl2.wizard.step2', 'myl2.wizard.step3', 'myl2.wizard.step4']

export default function CreateL2Wizard({ onBack, onCreate }: Props) {
  const { lang } = useLang()
  const [step, setStep] = useState(0)
  const [config, setConfig] = useState({
    name: '', chainId: '', description: '', icon: '🔗',
    l1Rpc: 'http://localhost:8545', rpcPort: '8550',
    sequencerMode: 'standalone', proverType: 'sp1',
    nativeToken: 'TON',
    isPublic: false, hashtags: '',
  })

  const update = (key: string, value: string | boolean) => setConfig(prev => ({ ...prev, [key]: value }))

  const canNext = () => {
    if (step === 0) return config.name && config.chainId
    return true
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-chat)]">
      {/* Header */}
      <div className="px-6 py-4 border-b border-[var(--color-border)]">
        <div className="flex items-center gap-3 mb-3">
          <button onClick={onBack} className="text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer">
            ← {t('openl2.back', lang)}
          </button>
        </div>
        <h1 className="text-lg font-semibold">{t('myl2.wizard.title', lang)}</h1>

        {/* Step indicator */}
        <div className="flex gap-2 mt-3">
          {steps.map((s, i) => (
            <div key={s} className="flex-1 flex flex-col items-center gap-1">
              <div className={`w-full h-1 rounded-full ${i <= step ? 'bg-[var(--color-accent)]' : 'bg-[var(--color-border)]'}`} />
              <span className={`text-[10px] ${i <= step ? 'text-[var(--color-accent)]' : 'text-[var(--color-text-secondary)]'}`}>
                {t(s, lang)}
              </span>
            </div>
          ))}
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-6 space-y-4">
        {step === 0 && (
          <>
            <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4">
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">{t('myl2.wizard.name', lang)} *</label>
              <input value={config.name} onChange={e => update('name', e.target.value)}
                placeholder="My DEX Chain"
                className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none placeholder-[var(--color-text-secondary)]" />
            </div>
            <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4">
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">Chain ID *</label>
              <input value={config.chainId} onChange={e => update('chainId', e.target.value)}
                placeholder="17001" type="number"
                className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none placeholder-[var(--color-text-secondary)]" />
            </div>
            <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4">
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">{t('myl2.wizard.icon', lang)}</label>
              <div className="flex gap-2 mt-1">
                {['🔗', '🔄', '🎨', '🎮', '🏦', '🌉', '🔒', '🤖', '🧪', '💎'].map(emoji => (
                  <button key={emoji} onClick={() => update('icon', emoji)}
                    className={`w-10 h-10 rounded-lg flex items-center justify-center text-xl cursor-pointer transition-colors ${
                      config.icon === emoji ? 'bg-[var(--color-accent)]' : 'bg-[var(--color-border)] hover:bg-[var(--color-accent)]'
                    }`}>
                    {emoji}
                  </button>
                ))}
              </div>
            </div>
            <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4">
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">{t('myl2.detail.configDesc', lang)}</label>
              <textarea value={config.description} onChange={e => update('description', e.target.value)}
                placeholder="A brief description of your L2..."
                rows={2}
                className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none resize-none placeholder-[var(--color-text-secondary)]" />
            </div>
          </>
        )}

        {step === 1 && (
          <>
            <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4">
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">L1 RPC URL</label>
              <input value={config.l1Rpc} onChange={e => update('l1Rpc', e.target.value)}
                className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none" />
            </div>
            <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4">
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">L2 RPC Port</label>
              <input value={config.rpcPort} onChange={e => update('rpcPort', e.target.value)}
                type="number"
                className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none" />
            </div>
            <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4">
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">{t('myl2.wizard.sequencerMode', lang)}</label>
              <select value={config.sequencerMode} onChange={e => update('sequencerMode', e.target.value)}
                className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none">
                <option value="standalone">{t('myl2.wizard.standalone', lang)}</option>
                <option value="shared">{t('myl2.wizard.shared', lang)}</option>
              </select>
            </div>
          </>
        )}

        {step === 2 && (
          <>
            <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4">
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">{t('myl2.detail.configToken', lang)}</label>
              <input value={config.nativeToken} onChange={e => update('nativeToken', e.target.value)}
                className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none" />
            </div>
            <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4">
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">{t('myl2.wizard.proverType', lang)}</label>
              <select value={config.proverType} onChange={e => update('proverType', e.target.value)}
                className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none">
                <option value="sp1">SP1</option>
                <option value="risc0">RISC Zero</option>
                <option value="none">{t('myl2.wizard.noProver', lang)}</option>
              </select>
            </div>
          </>
        )}

        {step === 3 && (
          <>
            <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4 flex items-center justify-between">
              <div>
                <div className="text-sm font-medium">{t('myl2.detail.configPublic', lang)}</div>
                <div className="text-xs text-[var(--color-text-secondary)]">{t('myl2.detail.configPublicDesc', lang)}</div>
              </div>
              <button
                onClick={() => update('isPublic', !config.isPublic)}
                className={`w-12 h-6 rounded-full flex items-center px-1 cursor-pointer transition-colors ${config.isPublic ? 'bg-[var(--color-accent)]' : 'bg-[var(--color-border)]'}`}>
                <div className={`w-4 h-4 bg-white rounded-full transition-transform ${config.isPublic ? 'translate-x-6' : ''}`} />
              </button>
            </div>
            <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4">
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">{t('myl2.detail.configHashtags', lang)}</label>
              <input value={config.hashtags} onChange={e => update('hashtags', e.target.value)}
                placeholder="#DeFi #DEX #AMM"
                className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none placeholder-[var(--color-text-secondary)]" />
            </div>

            {/* Summary */}
            <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4 space-y-2">
              <h3 className="font-medium text-sm">{t('myl2.wizard.summary', lang)}</h3>
              <div className="grid grid-cols-2 gap-2 text-xs">
                <span className="text-[var(--color-text-secondary)]">{t('myl2.wizard.name', lang)}</span>
                <span>{config.icon} {config.name}</span>
                <span className="text-[var(--color-text-secondary)]">Chain ID</span>
                <span>{config.chainId}</span>
                <span className="text-[var(--color-text-secondary)]">{t('myl2.detail.configToken', lang)}</span>
                <span>{config.nativeToken}</span>
                <span className="text-[var(--color-text-secondary)]">L1 RPC</span>
                <span className="truncate">{config.l1Rpc}</span>
                <span className="text-[var(--color-text-secondary)]">{t('myl2.wizard.proverType', lang)}</span>
                <span>{config.proverType.toUpperCase()}</span>
              </div>
            </div>
          </>
        )}
      </div>

      {/* Navigation */}
      <div className="px-6 py-4 border-t border-[var(--color-border)] flex justify-between">
        <button
          onClick={() => step > 0 ? setStep(step - 1) : onBack()}
          className="bg-[var(--color-border)] px-6 py-2.5 rounded-xl text-sm hover:opacity-80 transition-opacity cursor-pointer"
        >
          {step > 0 ? t('myl2.wizard.prev', lang) : t('openl2.back', lang)}
        </button>
        {step < steps.length - 1 ? (
          <button
            onClick={() => setStep(step + 1)}
            disabled={!canNext()}
            className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] disabled:opacity-40 px-6 py-2.5 rounded-xl text-sm font-medium transition-colors cursor-pointer"
          >
            {t('myl2.wizard.next', lang)}
          </button>
        ) : (
          <button
            onClick={() => onCreate(config as unknown as Record<string, string>)}
            className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] px-6 py-2.5 rounded-xl text-sm font-medium transition-colors cursor-pointer"
          >
            {t('myl2.wizard.create', lang)}
          </button>
        )}
      </div>
    </div>
  )
}
