import { useState } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'

export type NetworkMode = 'local' | 'testnet' | 'mainnet'

interface Props {
  onBack: () => void
  onCreate: (config: Record<string, string>) => void
  initialNetwork?: NetworkMode
}

const networkPresets: Record<NetworkMode, { l1Rpc: string; chainId: string; proverType: string }> = {
  local: { l1Rpc: 'http://localhost:8545', chainId: '17001', proverType: 'sp1' },
  testnet: { l1Rpc: 'https://rpc.sepolia.org', chainId: '17001', proverType: 'sp1' },
  mainnet: { l1Rpc: 'https://eth.llamarpc.com', chainId: '17001', proverType: 'sp1' },
}

const steps = ['myl2.wizard.step1', 'myl2.wizard.step2', 'myl2.wizard.step3', 'myl2.wizard.step4']

export default function CreateL2Wizard({ onBack, onCreate, initialNetwork }: Props) {
  const { lang } = useLang()
  const [networkMode, setNetworkMode] = useState<NetworkMode | null>(initialNetwork ?? null)
  const [step, setStep] = useState(0)
  const [config, setConfig] = useState(() => {
    const preset = initialNetwork ? networkPresets[initialNetwork] : networkPresets.local
    return {
      name: '', chainId: preset.chainId, description: '', icon: '🔗',
      l1Rpc: preset.l1Rpc, rpcPort: '8550',
      sequencerMode: 'standalone', proverType: preset.proverType,
      nativeToken: 'TON',
      isPublic: false, hashtags: '',
    }
  })

  const update = (key: string, value: string | boolean) => setConfig(prev => ({ ...prev, [key]: value }))

  const selectNetwork = (mode: NetworkMode) => {
    const preset = networkPresets[mode]
    setNetworkMode(mode)
    setConfig(prev => ({ ...prev, l1Rpc: preset.l1Rpc, chainId: preset.chainId, proverType: preset.proverType }))
  }

  const canNext = () => {
    if (step === 0) return config.name && config.chainId
    return true
  }

  // Network selection screen
  if (!networkMode) {
    return (
      <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
        <div className="px-4 py-3 border-b border-[var(--color-border)]">
          <button onClick={onBack} className="text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer">
            ← {t('openl2.back', lang)}
          </button>
        </div>
        <div className="flex-1 flex flex-col items-center justify-center px-6 pb-12">
          <h1 className="text-lg font-bold mb-1">{t('myl2.wizard.title', lang)}</h1>
          <p className="text-[13px] text-[var(--color-text-secondary)] mb-6">{t('myl2.wizard.selectNetwork', lang)}</p>
          <div className="w-full space-y-2">
            {([
              { mode: 'local' as NetworkMode, key: 'myl2.wizard.local', descKey: 'myl2.wizard.localDesc', color: 'bg-[var(--color-success)]' },
              { mode: 'testnet' as NetworkMode, key: 'myl2.wizard.testnet', descKey: 'myl2.wizard.testnetDesc', color: 'bg-[var(--color-warning)]' },
              { mode: 'mainnet' as NetworkMode, key: 'myl2.wizard.mainnet', descKey: 'myl2.wizard.mainnetDesc', color: 'bg-[var(--color-accent)]' },
            ]).map(({ mode, key, descKey, color }) => (
              <button
                key={mode}
                onClick={() => selectNetwork(mode)}
                className="w-full flex items-center gap-3 p-4 rounded-xl bg-[var(--color-bg-sidebar)] hover:bg-[var(--color-border)] border border-[var(--color-border)] transition-colors cursor-pointer text-left"
              >
                <div className={`w-10 h-10 rounded-lg ${color} flex items-center justify-center flex-shrink-0 text-white font-bold text-sm`}>
                  {mode === 'local' ? 'L' : mode === 'testnet' ? 'T' : 'M'}
                </div>
                <div className="flex-1">
                  <div className="text-[14px] font-medium">{t(key, lang)}</div>
                  <div className="text-[12px] text-[var(--color-text-secondary)]">{t(descKey, lang)}</div>
                </div>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--color-text-secondary)]">
                  <polyline points="9 18 15 12 9 6"/>
                </svg>
              </button>
            ))}
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      {/* Header */}
      <div className="px-4 py-3 border-b border-[var(--color-border)]">
        <div className="flex items-center justify-between mb-3">
          <button onClick={() => step > 0 ? setStep(step - 1) : setNetworkMode(null)} className="text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer">
            ← {step > 0 ? t('myl2.wizard.prev', lang) : t('openl2.back', lang)}
          </button>
          <span className={`text-[10px] px-2 py-0.5 rounded-full font-medium text-white ${
            networkMode === 'local' ? 'bg-[var(--color-success)]' : networkMode === 'testnet' ? 'bg-[var(--color-warning)]' : 'bg-[var(--color-accent)] text-[var(--color-accent-text)]'
          }`}>
            {t(`myl2.wizard.${networkMode}`, lang)}
          </span>
        </div>

        {/* Step indicator */}
        <div className="flex gap-2">
          {steps.map((s, i) => (
            <div key={s} className="flex-1 flex flex-col items-center gap-1">
              <div className={`w-full h-1 rounded-full ${i <= step ? 'bg-[var(--color-accent)]' : 'bg-[var(--color-border)]'}`} />
              <span className={`text-[10px] ${i <= step ? 'text-[var(--color-text-primary)]' : 'text-[var(--color-text-secondary)]'}`}>
                {t(s, lang)}
              </span>
            </div>
          ))}
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-4 space-y-3">
        {step === 0 && (
          <>
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
              <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('myl2.wizard.name', lang)} *</label>
              <input value={config.name} onChange={e => update('name', e.target.value)}
                placeholder="My DEX Chain"
                className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-sm outline-none placeholder-[var(--color-text-secondary)] border border-[var(--color-border)]" />
            </div>
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
              <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">Chain ID *</label>
              <input value={config.chainId} onChange={e => update('chainId', e.target.value)}
                placeholder="17001" type="number"
                className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-sm outline-none placeholder-[var(--color-text-secondary)] border border-[var(--color-border)]" />
            </div>
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
              <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('myl2.wizard.icon', lang)}</label>
              <div className="flex gap-2 mt-1 flex-wrap">
                {['🔗', '🔄', '🎨', '🎮', '🏦', '🌉', '🔒', '🤖', '🧪', '💎'].map(emoji => (
                  <button key={emoji} onClick={() => update('icon', emoji)}
                    className={`w-9 h-9 rounded-lg flex items-center justify-center text-lg cursor-pointer transition-colors ${
                      config.icon === emoji ? 'bg-[var(--color-accent)]' : 'bg-[var(--color-bg-main)] border border-[var(--color-border)] hover:bg-[var(--color-border)]'
                    }`}>
                    {emoji}
                  </button>
                ))}
              </div>
            </div>
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
              <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('myl2.detail.configDesc', lang)}</label>
              <textarea value={config.description} onChange={e => update('description', e.target.value)}
                placeholder="A brief description..."
                rows={2}
                className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-sm outline-none resize-none placeholder-[var(--color-text-secondary)] border border-[var(--color-border)]" />
            </div>
          </>
        )}

        {step === 1 && (
          <>
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
              <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">L1 RPC URL</label>
              <input value={config.l1Rpc} onChange={e => update('l1Rpc', e.target.value)}
                className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-sm outline-none border border-[var(--color-border)] font-mono text-[12px]" />
              {networkMode === 'local' && (
                <p className="text-[10px] text-[var(--color-text-secondary)] mt-1">anvil/hardhat이 자동으로 실행됩니다</p>
              )}
            </div>
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
              <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">L2 RPC Port</label>
              <input value={config.rpcPort} onChange={e => update('rpcPort', e.target.value)}
                type="number"
                className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-sm outline-none border border-[var(--color-border)]" />
            </div>
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
              <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('myl2.wizard.sequencerMode', lang)}</label>
              {networkMode === 'local' ? (
                <div className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-sm border border-[var(--color-border)] text-[var(--color-text-primary)] font-medium">
                  {t('myl2.wizard.standalone', lang)}
                </div>
              ) : (
                <select value={config.sequencerMode} onChange={e => update('sequencerMode', e.target.value)}
                  className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-sm outline-none border border-[var(--color-border)]">
                  <option value="standalone">{t('myl2.wizard.standalone', lang)}</option>
                  <option value="shared">{t('myl2.wizard.shared', lang)}</option>
                </select>
              )}
            </div>
          </>
        )}

        {step === 2 && (
          <>
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
              <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('myl2.detail.configToken', lang)}</label>
              <div className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-sm border border-[var(--color-border)] text-[var(--color-text-primary)] flex items-center gap-2">
                <span className="font-medium">TON</span>
                <span className="text-[var(--color-text-secondary)] text-[11px]">(TOKAMAK)</span>
              </div>
            </div>
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
              <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('myl2.wizard.proverType', lang)}</label>
              <div className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-sm border border-[var(--color-border)] text-[var(--color-text-primary)] font-medium">
                SP1
              </div>
              <p className="text-[10px] text-[var(--color-text-secondary)] mt-1">Succinct SP1 프로버가 사용됩니다</p>
            </div>
          </>
        )}

        {step === 3 && (
          <>
            <div className={`bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)] flex items-center justify-between ${networkMode === 'local' ? 'opacity-40' : ''}`}>
              <div>
                <div className="text-sm font-medium">{t('myl2.detail.configPublic', lang)}</div>
                <div className="text-[11px] text-[var(--color-text-secondary)]">
                  {networkMode === 'local'
                    ? (lang === 'ko' ? '로컬 모드에서는 공개할 수 없습니다' : 'Cannot publish in local mode')
                    : t('myl2.detail.configPublicDesc', lang)
                  }
                </div>
              </div>
              <button
                onClick={() => networkMode !== 'local' && update('isPublic', !config.isPublic)}
                disabled={networkMode === 'local'}
                className={`w-12 h-6 rounded-full flex items-center px-1 transition-colors ${networkMode === 'local' ? 'bg-[var(--color-border)] cursor-not-allowed' : `cursor-pointer ${config.isPublic ? 'bg-[var(--color-accent)]' : 'bg-[var(--color-border)]'}`}`}>
                <div className={`w-4 h-4 bg-white rounded-full transition-transform ${config.isPublic && networkMode !== 'local' ? 'translate-x-6' : ''}`} />
              </button>
            </div>
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
              <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('myl2.detail.configHashtags', lang)}</label>
              <input value={config.hashtags} onChange={e => update('hashtags', e.target.value)}
                placeholder="#DeFi #DEX #AMM"
                className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-sm outline-none placeholder-[var(--color-text-secondary)] border border-[var(--color-border)]" />
            </div>

            {/* Summary */}
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 space-y-2 border border-[var(--color-border)]">
              <h3 className="font-medium text-sm">{t('myl2.wizard.summary', lang)}</h3>
              <div className="grid grid-cols-2 gap-y-1.5 text-[12px]">
                <span className="text-[var(--color-text-secondary)]">{t('myl2.wizard.name', lang)}</span>
                <span>{config.icon} {config.name}</span>
                <span className="text-[var(--color-text-secondary)]">Chain ID</span>
                <span>{config.chainId}</span>
                <span className="text-[var(--color-text-secondary)]">{t('myl2.wizard.selectNetwork', lang)}</span>
                <span className="capitalize">{t(`myl2.wizard.${networkMode}`, lang)}</span>
                <span className="text-[var(--color-text-secondary)]">{t('myl2.detail.configToken', lang)}</span>
                <span>{config.nativeToken}</span>
                <span className="text-[var(--color-text-secondary)]">L1 RPC</span>
                <span className="truncate font-mono text-[11px]">{config.l1Rpc}</span>
                <span className="text-[var(--color-text-secondary)]">{t('myl2.wizard.proverType', lang)}</span>
                <span>{config.proverType === 'none' ? t('myl2.wizard.noProver', lang) : config.proverType.toUpperCase()}</span>
              </div>
            </div>
          </>
        )}
      </div>

      {/* Navigation */}
      <div className="px-4 py-3 border-t border-[var(--color-border)] flex justify-end">
        {step < steps.length - 1 ? (
          <button
            onClick={() => setStep(step + 1)}
            disabled={!canNext()}
            className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] disabled:opacity-40 px-6 py-2.5 rounded-xl text-sm font-medium transition-colors cursor-pointer text-[var(--color-accent-text)]"
          >
            {t('myl2.wizard.next', lang)}
          </button>
        ) : (
          <button
            onClick={() => onCreate({ ...config, networkMode: networkMode!, isPublic: String(config.isPublic) } as unknown as Record<string, string>)}
            className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] px-6 py-2.5 rounded-xl text-sm font-medium transition-colors cursor-pointer text-[var(--color-accent-text)]"
          >
            {t('myl2.wizard.create', lang)}
          </button>
        )}
      </div>
    </div>
  )
}
