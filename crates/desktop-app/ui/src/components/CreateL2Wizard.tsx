import { useState, useEffect, useCallback } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'
import { localServerAPI } from '../api/local-server'

export type NetworkMode = 'local' | 'testnet' | 'mainnet'

// Program slug for Docker image detection — update when multi-program UI is added
const PROGRAM_SLUG = 'zk-dex'

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

interface RpcStatus {
  state: 'idle' | 'testing' | 'ok' | 'error'
  chainId?: number
  chainName?: string
  blockNumber?: number
  error?: string
}

interface ResolvedRoles {
  [key: string]: { address: string; balance: string; label: string; error?: string }
}

interface KeysResolution {
  state: 'idle' | 'resolving' | 'ok' | 'error'
  roles?: ResolvedRoles
  gasPriceGwei?: string
  estimatedDeployCostEth?: string
  deployerSufficient?: boolean
  error?: string
}

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
      deployerKeychainKey: '', committerKeychainKey: '', proofCoordinatorKeychainKey: '', bridgeOwnerKeychainKey: '',
    }
  })
  const [rpcStatus, setRpcStatus] = useState<RpcStatus>({ state: 'idle' })
  const [keychainAccounts, setKeychainAccounts] = useState<string[]>([])
  const [keysResolution, setKeysResolution] = useState<KeysResolution>({ state: 'idle' })
  const [showConfirm, setShowConfirm] = useState(false)
  const [forceRebuild, setForceRebuild] = useState(false)
  const [forceRedeploy, setForceRedeploy] = useState(false)
  const [existingImage, setExistingImage] = useState<{ checked: boolean; exists: boolean }>({ checked: false, exists: false })
  const [gasEstimate, setGasEstimate] = useState<{
    breakdown: Record<string, { gas: number; label: string; costEth: string; interval: string | null }>
    totalCostEth: string; gasPriceGwei: string
  } | null>(null)

  const isTestnetOrMainnet = networkMode === 'testnet' || networkMode === 'mainnet'

  // Steps differ: local has 4 steps, testnet/mainnet has 5 (with wallet step)
  const steps = isTestnetOrMainnet
    ? ['myl2.wizard.step1', 'myl2.wizard.step2', 'myl2.wizard.step5', 'myl2.wizard.step3', 'myl2.wizard.step4']
    : ['myl2.wizard.step1', 'myl2.wizard.step2', 'myl2.wizard.step3', 'myl2.wizard.step4']

  // Map logical step index to content type
  const getStepContent = (s: number): string => {
    if (isTestnetOrMainnet) {
      return ['basic', 'network', 'wallet', 'token', 'publish'][s] || 'basic'
    }
    return ['basic', 'network', 'token', 'publish'][s] || 'basic'
  }

  const update = (key: string, value: string | boolean) => setConfig(prev => ({ ...prev, [key]: value }))

  const selectNetwork = (mode: NetworkMode) => {
    const preset = networkPresets[mode]
    setNetworkMode(mode)
    setConfig(prev => ({ ...prev, l1Rpc: preset.l1Rpc, chainId: preset.chainId, proverType: preset.proverType }))
    setRpcStatus({ state: 'idle' })
    setKeysResolution({ state: 'idle' })
  }

  // Load keychain accounts when entering wallet step
  const isWalletStep = isTestnetOrMainnet && getStepContent(step) === 'wallet'
  useEffect(() => {
    if (isWalletStep) {
      localServerAPI.listKeychainAccounts()
        .then(r => setKeychainAccounts(r.accounts || []))
        .catch(() => setKeychainAccounts([]))
    }
  }, [isWalletStep])

  // Check for existing Docker image when entering publish step
  const isPublishStep = getStepContent(step) === 'publish'
  useEffect(() => {
    if (isPublishStep && !existingImage.checked) {
      localServerAPI.checkImage(PROGRAM_SLUG)
        .then(r => setExistingImage({ checked: true, exists: r.exists }))
        .catch(() => setExistingImage({ checked: true, exists: false }))
    }
  }, [isPublishStep, existingImage.checked])

  const testRpc = useCallback(async () => {
    setRpcStatus({ state: 'testing' })
    setGasEstimate(null)
    try {
      const result = await localServerAPI.checkRpc(config.l1Rpc)
      if (result.ok) {
        setRpcStatus({ state: 'ok', chainId: result.chainId, chainName: result.chainName, blockNumber: result.blockNumber })
        // Auto-fetch gas estimate on successful connection
        localServerAPI.estimateGas(config.l1Rpc)
          .then(est => setGasEstimate({ breakdown: est.breakdown, totalCostEth: est.totalCostEth, gasPriceGwei: est.gasPriceGwei }))
          .catch(() => {})
      } else {
        setRpcStatus({ state: 'error', error: 'Unexpected response' })
      }
    } catch (e: unknown) {
      setRpcStatus({ state: 'error', error: e instanceof Error ? e.message : 'Connection failed' })
    }
  }, [config.l1Rpc])

  const resolveKeys = useCallback(async () => {
    if (!config.deployerKeychainKey) return
    setKeysResolution({ state: 'resolving' })
    try {
      const result = await localServerAPI.resolveKeys({
        rpcUrl: config.l1Rpc,
        deployerKey: config.deployerKeychainKey,
        committerKey: config.committerKeychainKey || undefined,
        proofCoordinatorKey: config.proofCoordinatorKeychainKey || undefined,
        bridgeOwnerKey: config.bridgeOwnerKeychainKey || undefined,
      })
      setKeysResolution({
        state: 'ok',
        roles: result.roles,
        gasPriceGwei: result.gasPriceGwei,
        estimatedDeployCostEth: result.estimatedDeployCostEth,
        deployerSufficient: result.deployerSufficient,
      })
    } catch (e: unknown) {
      setKeysResolution({ state: 'error', error: e instanceof Error ? e.message : 'Failed to resolve keys' })
    }
  }, [config.l1Rpc, config.deployerKeychainKey, config.committerKeychainKey, config.proofCoordinatorKeychainKey, config.bridgeOwnerKeychainKey])

  const canNext = () => {
    const content = getStepContent(step)
    if (content === 'basic') return config.name && config.chainId
    if (content === 'wallet') return !!config.deployerKeychainKey
    return true
  }

  const handleCreate = () => {
    if (isTestnetOrMainnet) {
      setShowConfirm(true)
    } else {
      doCreate()
    }
  }

  const doCreate = () => {
    setShowConfirm(false)
    const { isPublic, deployerKeychainKey, committerKeychainKey, proofCoordinatorKeychainKey, bridgeOwnerKeychainKey, ...rest } = config
    const out: Record<string, string> = {
      ...rest,
      networkMode: networkMode!,
      isPublic: String(isPublic),
    }
    // Only include keychain keys that are set
    if (deployerKeychainKey) out.deployerKeychainKey = deployerKeychainKey
    if (committerKeychainKey) out.committerKeychainKey = committerKeychainKey
    if (proofCoordinatorKeychainKey) out.proofCoordinatorKeychainKey = proofCoordinatorKeychainKey
    if (bridgeOwnerKeychainKey) out.bridgeOwnerKeychainKey = bridgeOwnerKeychainKey
    // Build optimization flags
    if (forceRebuild) out.forceRebuild = 'true'
    if (forceRedeploy) out.forceRedeploy = 'true'
    onCreate(out)
  }

  const maskAddress = (addr: string) => addr ? `${addr.slice(0, 6)}...${addr.slice(-4)}` : ''

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

  const content = getStepContent(step)

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      {/* Confirmation Dialog */}
      {showConfirm && (
        <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="bg-[var(--color-bg-main)] rounded-2xl p-5 mx-4 max-w-sm w-full border border-[var(--color-border)] shadow-xl">
            <h3 className="text-base font-bold mb-2">{t('myl2.wizard.confirmTitle', lang)}</h3>
            <div className="space-y-2 text-[13px] mb-4">
              <div className="flex justify-between">
                <span className="text-[var(--color-text-secondary)]">{t('myl2.wizard.name', lang)}</span>
                <span>{config.icon} {config.name}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[var(--color-text-secondary)]">L1</span>
                <span>{rpcStatus.chainName || networkMode}</span>
              </div>
              {keysResolution.state === 'ok' && keysResolution.roles?.deployer && (
                <>
                  <div className="flex justify-between">
                    <span className="text-[var(--color-text-secondary)]">Deployer</span>
                    <span className="font-mono text-[11px]">{maskAddress(keysResolution.roles.deployer.address)}</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-[var(--color-text-secondary)]">{t('myl2.wizard.balance', lang)}</span>
                    <span>{keysResolution.roles.deployer.balance} ETH</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-[var(--color-text-secondary)]">{t('myl2.wizard.estimatedCost', lang)}</span>
                    <span>~{keysResolution.estimatedDeployCostEth} ETH</span>
                  </div>
                </>
              )}
              {keysResolution.state !== 'ok' && (
                <div className="mt-2 p-2 rounded-lg bg-[var(--color-error)]/10 text-[var(--color-error)] text-[12px]">
                  {lang === 'ko' ? '주소 및 잔액 확인이 완료되지 않았습니다' : 'Address & balance check not completed'}
                </div>
              )}
              <div className="mt-2 p-2 rounded-lg bg-[var(--color-warning)]/10 text-[var(--color-warning)] text-[12px]">
                {t('myl2.wizard.confirmDesc', lang)}
              </div>
            </div>
            <div className="flex gap-2">
              <button
                onClick={() => setShowConfirm(false)}
                className="flex-1 px-4 py-2 rounded-xl text-sm border border-[var(--color-border)] hover:bg-[var(--color-border)] transition-colors cursor-pointer"
              >
                {t('myl2.wizard.confirmCancel', lang)}
              </button>
              <button
                onClick={doCreate}
                className="flex-1 px-4 py-2 rounded-xl text-sm font-medium bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] text-[var(--color-accent-text)] transition-colors cursor-pointer"
              >
                {t('myl2.wizard.confirmDeploy', lang)}
              </button>
            </div>
          </div>
        </div>
      )}

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
        {/* Step: Basic Info */}
        {content === 'basic' && (
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

        {/* Step: Network */}
        {content === 'network' && (
          <>
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
              <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">L1 RPC URL</label>
              <div className="flex gap-2">
                <input value={config.l1Rpc} onChange={e => { update('l1Rpc', e.target.value); setRpcStatus({ state: 'idle' }) }}
                  className="flex-1 bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-sm outline-none border border-[var(--color-border)] font-mono text-[12px]" />
                {isTestnetOrMainnet && (
                  <button
                    onClick={testRpc}
                    disabled={rpcStatus.state === 'testing'}
                    className="px-3 py-2 rounded-lg text-[12px] font-medium border border-[var(--color-border)] hover:bg-[var(--color-border)] disabled:opacity-50 transition-colors cursor-pointer whitespace-nowrap"
                  >
                    {rpcStatus.state === 'testing' ? t('myl2.wizard.testing', lang) : t('myl2.wizard.testConnection', lang)}
                  </button>
                )}
              </div>
              {networkMode === 'local' && (
                <p className="text-[10px] text-[var(--color-text-secondary)] mt-1">anvil/hardhat이 자동으로 실행됩니다</p>
              )}
              {rpcStatus.state === 'ok' && (
                <div className="mt-2 flex items-center gap-2 text-[11px]">
                  <span className="w-2 h-2 rounded-full bg-[var(--color-success)]" />
                  <span className="text-[var(--color-success)] font-medium">{t('myl2.wizard.connected', lang)}</span>
                  <span className="text-[var(--color-text-secondary)]">· {rpcStatus.chainName} · Block #{rpcStatus.blockNumber?.toLocaleString()}</span>
                </div>
              )}
              {rpcStatus.state === 'error' && (
                <div className="mt-2 flex items-center gap-2 text-[11px]">
                  <span className="w-2 h-2 rounded-full bg-[var(--color-error)]" />
                  <span className="text-[var(--color-error)]">{t('myl2.wizard.connectionFailed', lang)}: {rpcStatus.error}</span>
                </div>
              )}
            </div>
            {/* Gas Cost Breakdown (shown after successful RPC test) */}
            {isTestnetOrMainnet && gasEstimate && rpcStatus.state === 'ok' && (
              <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
                <h3 className="text-sm font-medium mb-2">{t('myl2.wizard.costBreakdown', lang)}</h3>
                <div className="space-y-1.5 text-[12px]">
                  {Object.entries(gasEstimate.breakdown).map(([role, info]) => (
                    <div key={role} className="flex items-center justify-between">
                      <div className="flex-1 min-w-0">
                        <span className="text-[var(--color-text-primary)]">{info.label}</span>
                        {info.interval && (
                          <span className="text-[10px] text-[var(--color-text-secondary)] ml-1">({info.interval})</span>
                        )}
                      </div>
                      <span className="font-mono text-[11px] ml-2 flex-shrink-0">~{info.costEth} ETH</span>
                    </div>
                  ))}
                  <div className="border-t border-[var(--color-border)] pt-1.5 mt-1.5 flex justify-between font-medium">
                    <span>{t('myl2.wizard.totalCost', lang)}</span>
                    <span className="font-mono text-[11px]">~{gasEstimate.totalCostEth} ETH</span>
                  </div>
                  <div className="text-[10px] text-[var(--color-text-secondary)]">
                    {t('myl2.wizard.gasPrice', lang)}: {gasEstimate.gasPriceGwei} Gwei
                  </div>
                </div>
              </div>
            )}

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

        {/* Step: Wallet (testnet/mainnet only) */}
        {content === 'wallet' && (
          <>
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
              <h3 className="text-sm font-medium mb-1">{t('myl2.wizard.walletTitle', lang)}</h3>
              <p className="text-[11px] text-[var(--color-text-secondary)] mb-3">{t('myl2.wizard.walletDesc', lang)}</p>

              {keychainAccounts.length === 0 ? (
                <div className="text-center py-4">
                  <p className="text-[13px] text-[var(--color-text-secondary)]">{t('myl2.wizard.noKeychainKeys', lang)}</p>
                  <p className="text-[11px] text-[var(--color-text-secondary)] mt-1">{t('myl2.wizard.keychainHint', lang)}</p>
                </div>
              ) : (
                <div className="space-y-3">
                  {/* Deployer Key (required) */}
                  <div>
                    <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('myl2.wizard.deployerKey', lang)} *</label>
                    <select
                      value={config.deployerKeychainKey}
                      onChange={e => { update('deployerKeychainKey', e.target.value); setKeysResolution({ state: 'idle' }) }}
                      className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-sm outline-none border border-[var(--color-border)]"
                    >
                      <option value="">{t('myl2.wizard.selectKey', lang)}</option>
                      {keychainAccounts.map(acc => <option key={acc} value={acc}>{acc}</option>)}
                    </select>
                  </div>

                  {/* Optional role keys */}
                  {[
                    { key: 'committerKeychainKey', label: 'myl2.wizard.committerKey' },
                    { key: 'proofCoordinatorKeychainKey', label: 'myl2.wizard.proofCoordinatorKey' },
                    { key: 'bridgeOwnerKeychainKey', label: 'myl2.wizard.bridgeOwnerKey' },
                  ].map(({ key, label }) => (
                    <div key={key}>
                      <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t(label, lang)}</label>
                      <select
                        value={config[key as keyof typeof config] as string}
                        onChange={e => { update(key, e.target.value); setKeysResolution({ state: 'idle' }) }}
                        className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-sm outline-none border border-[var(--color-border)]"
                      >
                        <option value="">{t('myl2.wizard.sameAsDeployer', lang)}</option>
                        {keychainAccounts.map(acc => <option key={acc} value={acc}>{acc}</option>)}
                      </select>
                    </div>
                  ))}
                </div>
              )}
            </div>

            {/* Resolve keys button */}
            {config.deployerKeychainKey && (
              <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
                <button
                  onClick={resolveKeys}
                  disabled={keysResolution.state === 'resolving'}
                  className="w-full py-2.5 rounded-xl text-sm font-medium border border-[var(--color-accent)] text-[var(--color-accent)] hover:bg-[var(--color-accent)]/10 disabled:opacity-50 transition-colors cursor-pointer"
                >
                  {keysResolution.state === 'resolving' ? t('myl2.wizard.resolving', lang) : t('myl2.wizard.resolveKeys', lang)}
                </button>

                {keysResolution.state === 'error' && (
                  <p className="text-[11px] text-[var(--color-error)] mt-2">{keysResolution.error}</p>
                )}

                {keysResolution.state === 'ok' && keysResolution.roles && (
                  <div className="mt-3 space-y-2">
                    {Object.entries(keysResolution.roles).map(([roleKey, role]) => (
                      <div key={roleKey} className="flex items-center justify-between text-[12px]">
                        <div className="flex items-center gap-2">
                          <span className="text-[var(--color-text-secondary)] w-28">{role.label}</span>
                          <span className="font-mono text-[11px]">{maskAddress(role.address)}</span>
                        </div>
                        <span className={`font-medium ${parseFloat(role.balance) > 0 ? 'text-[var(--color-success)]' : 'text-[var(--color-error)]'}`}>
                          {role.balance} ETH
                        </span>
                      </div>
                    ))}

                    <div className="border-t border-[var(--color-border)] pt-2 mt-2 space-y-1 text-[12px]">
                      <div className="flex justify-between">
                        <span className="text-[var(--color-text-secondary)]">{t('myl2.wizard.gasPrice', lang)}</span>
                        <span>{keysResolution.gasPriceGwei} Gwei</span>
                      </div>
                      <div className="flex justify-between">
                        <span className="text-[var(--color-text-secondary)]">{t('myl2.wizard.estimatedCost', lang)}</span>
                        <span>~{keysResolution.estimatedDeployCostEth} ETH</span>
                      </div>
                      <div className="flex justify-between">
                        <span className="text-[var(--color-text-secondary)]">Deployer</span>
                        <span className={`font-medium ${keysResolution.deployerSufficient ? 'text-[var(--color-success)]' : 'text-[var(--color-error)]'}`}>
                          {keysResolution.deployerSufficient ? t('myl2.wizard.sufficient', lang) : t('myl2.wizard.insufficient', lang)}
                        </span>
                      </div>
                    </div>
                  </div>
                )}
              </div>
            )}
          </>
        )}

        {/* Step: Token/Prover */}
        {content === 'token' && (
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

        {/* Step: Publish */}
        {content === 'publish' && (
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

            {/* Build Options */}
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)]">
              <h3 className="text-sm font-medium mb-2">{t('myl2.wizard.buildOptions', lang)}</h3>

              {existingImage.checked && existingImage.exists && !forceRebuild && (
                <div className="mb-3 p-2 rounded-lg bg-[var(--color-success)]/10 text-[var(--color-success)] text-[12px]">
                  {t('myl2.wizard.existingImageFound', lang)}
                </div>
              )}

              <div className="space-y-2">
                <label className="flex items-start gap-3 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={forceRebuild}
                    onChange={e => setForceRebuild(e.target.checked)}
                    className="mt-0.5 cursor-pointer"
                  />
                  <div>
                    <div className="text-[13px]">{t('myl2.wizard.forceRebuild', lang)}</div>
                    <div className="text-[11px] text-[var(--color-text-secondary)]">{t('myl2.wizard.forceRebuildDesc', lang)}</div>
                  </div>
                </label>

                {isTestnetOrMainnet && (
                  <label className="flex items-start gap-3 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={forceRedeploy}
                      onChange={e => setForceRedeploy(e.target.checked)}
                      className="mt-0.5 cursor-pointer"
                    />
                    <div>
                      <div className="text-[13px]">{t('myl2.wizard.forceRedeploy', lang)}</div>
                      <div className="text-[11px] text-[var(--color-text-secondary)]">{t('myl2.wizard.forceRedeployDesc', lang)}</div>
                    </div>
                  </label>
                )}
              </div>

              {existingImage.checked && (
                <div className="mt-2 text-[11px] text-[var(--color-text-secondary)]">
                  {forceRebuild
                    ? t('myl2.wizard.imageWillBuild', lang)
                    : existingImage.exists
                      ? t('myl2.wizard.imageWillReuse', lang)
                      : t('myl2.wizard.imageWillBuild', lang)
                  }
                </div>
              )}
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
                {isTestnetOrMainnet && keysResolution.state === 'ok' && keysResolution.roles?.deployer && (
                  <>
                    <span className="text-[var(--color-text-secondary)]">Deployer</span>
                    <span className="font-mono text-[11px]">{maskAddress(keysResolution.roles.deployer.address)}</span>
                    <span className="text-[var(--color-text-secondary)]">{t('myl2.wizard.estimatedCost', lang)}</span>
                    <span>~{keysResolution.estimatedDeployCostEth} ETH</span>
                  </>
                )}
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
            onClick={handleCreate}
            className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] px-6 py-2.5 rounded-xl text-sm font-medium transition-colors cursor-pointer text-[var(--color-accent-text)]"
          >
            {t('myl2.wizard.create', lang)}
          </button>
        )}
      </div>
    </div>
  )
}
