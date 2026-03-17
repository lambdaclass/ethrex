import { useState, useEffect, useCallback, useRef } from 'react'
import { SectionHeader } from './ui-atoms'
import type { L2Config } from './MyL2View'
import type { ContainerInfo, Product } from './L2DetailView'
import { localServerAPI } from '../api/local-server'

interface BridgeUIConfig {
  bridge_address?: string
  on_chain_proposer_address?: string
  timelock_address?: string
  sp1_verifier_address?: string
}

const SERVICE_NAME_PREFIXES = ['tokamak-app-', 'zk-dex-tools-'] as const

const CORE_SERVICES = [
  { label: 'L1 Node', service: 'tokamak-app-l1', portKey: 'l1Port' as const, localOnly: true },
  { label: 'L2 Node', service: 'tokamak-app-l2', portKey: 'l2Port' as const, localOnly: false },
  { label: 'Prover', service: 'tokamak-app-prover', portKey: null, localOnly: false },
]

const TOOLS_SERVICES: { label: string; service: string; portKey: keyof L2Config | null; localOnly?: boolean }[] = [
  { label: 'L1 Explorer', service: 'frontend-l1', portKey: 'toolsL1ExplorerPort', localOnly: true },
  { label: 'L2 Explorer', service: 'frontend-l2', portKey: 'toolsL2ExplorerPort' },
  { label: 'Dashboard', service: 'bridge-ui', portKey: 'toolsBridgeUIPort' },
]

const TESTNET_EXPLORER_URLS: Record<string, string> = {
  sepolia: 'https://sepolia.etherscan.io',
  holesky: 'https://holesky.etherscan.io',
}

const TESTNET_NETWORK_NAMES: Record<string, string> = {
  sepolia: 'Sepolia',
  holesky: 'Holesky',
}

/** Mask API key in RPC URL: https://...alchemy.com/v2/abcdef123 → https://...alchemy.com/v2/abc***23 */
function maskRpcUrl(url: string): string {
  return url.replace(/(\/v[12]\/)([a-zA-Z0-9_-]+)/, (_, prefix, key) => {
    if (key.length <= 6) return `${prefix}${'*'.repeat(key.length)}`
    return `${prefix}${key.slice(0, 3)}***${key.slice(-2)}`
  })
}

interface Props {
  l2: L2Config
  ko: boolean
  containers: ContainerInfo[]
  products: Product[]
  actionLoading: boolean
  handleAction: (action: 'start' | 'stop') => void
  onRefresh?: () => void
  onRetry?: () => void
  l1ChainId?: number
  l2ChainId?: number
  onOpenManager?: () => void
}

export default function L2DetailServicesTab({
  l2, ko, containers, products, actionLoading, handleAction,
  onRefresh, onRetry, l1ChainId, l2ChainId, onOpenManager,
}: Props) {
  const [toolsLoading, setToolsLoading] = useState(false)
  const [bridgeConfig, setBridgeConfig] = useState<BridgeUIConfig | null>(null)
  const [copiedAddr, setCopiedAddr] = useState<string | null>(null)
  const [roleBalances, setRoleBalances] = useState<Record<string, { address: string; balance: string }> | null>(null)
  const [balanceLoading, setBalanceLoading] = useState(false)

  const checkRoleBalances = useCallback(async () => {
    if (!l2.rawConfig || !l2.testnetL1RpcUrl) return
    let config: Record<string, unknown> = {}
    try { config = JSON.parse(l2.rawConfig) } catch { return }
    const testnet = (config.testnet || {}) as Record<string, string>
    if (!testnet.keychainKeyName) return
    setBalanceLoading(true)
    try {
      const result = await localServerAPI.resolveKeys({
        rpcUrl: l2.testnetL1RpcUrl,
        deployerKey: testnet.keychainKeyName,
        committerKey: testnet.committerKeychainKey || undefined,
        proofCoordinatorKey: testnet.proofCoordinatorKeychainKey || undefined,
        bridgeOwnerKey: testnet.bridgeOwnerKeychainKey || undefined,
      })
      setRoleBalances(result.roles)
    } catch { /* ignore */ }
    finally { setBalanceLoading(false) }
  }, [l2.rawConfig, l2.testnetL1RpcUrl])

  const copyTimerRef = useRef<ReturnType<typeof setTimeout>>(null)
  useEffect(() => () => { if (copyTimerRef.current) clearTimeout(copyTimerRef.current) }, [])
  const copyToClipboard = useCallback((addr: string) => {
    navigator.clipboard.writeText(addr).then(() => {
      setCopiedAddr(addr)
      if (copyTimerRef.current) clearTimeout(copyTimerRef.current)
      copyTimerRef.current = setTimeout(() => setCopiedAddr(null), 2000)
    }).catch(() => {})
  }, [])

  // Re-fetch config.json when bridge-ui container becomes running
  const bridgeUIRunning = containers.some(c =>
    (c.service === 'bridge-ui' || c.name?.includes('bridge-ui')) && c.state === 'running'
  )
  useEffect(() => {
    if (!l2.toolsBridgeUIPort) return
    fetch(`http://localhost:${l2.toolsBridgeUIPort}/config.json`)
      .then(r => r.ok ? r.json() : null)
      .then(data => { if (data) setBridgeConfig(data) })
      .catch((e) => { console.error('Failed to fetch bridge UI config:', e) })
  }, [l2.toolsBridgeUIPort, bridgeUIRunning])

  const stripPrefixes = (s: string) =>
    SERVICE_NAME_PREFIXES.reduce((acc, p) => acc.replace(p, ''), s)

  const svcState = (svc: string): string => {
    const c = containers.find(c => c.service === svc || c.name?.includes(stripPrefixes(svc)))
    return c ? (c.state || 'stopped') : 'stopped'
  }

  const svcPort = (svc: string): string | null => {
    const c = containers.find(c => c.service === svc || c.name?.includes(stripPrefixes(svc)))
    if (!c?.ports) return null
    const m = c.ports.match(/0\.0\.0\.0:(\d+)/)
    return m ? `:${m[1]}` : null
  }

  const dotColor = (state: string) => {
    if (state === 'running') return 'var(--color-success)'
    if (state === 'restarting') return 'var(--color-warning)'
    return 'var(--color-text-secondary)'
  }

  const openInBrowser = async (url: string) => {
    try {
      const base = `http://127.0.0.1:${import.meta.env.VITE_LOCAL_SERVER_PORT || 5002}`
      await fetch(`${base}/api/open-url`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ url }),
      })
    } catch (e) { console.error('Failed to open URL:', e) }
  }

  const isTestnet = l2.networkMode === 'testnet'
  const testnetNetworkName = l2.testnetNetwork ? (TESTNET_NETWORK_NAMES[l2.testnetNetwork] || l2.testnetNetwork) : 'External'
  const testnetExplorerUrl = l2.testnetNetwork ? TESTNET_EXPLORER_URLS[l2.testnetNetwork] : null

  return (
    <>
      {/* Docker Services */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl border border-[var(--color-border)] overflow-hidden">
        <div className="px-3 pt-3 pb-1 flex items-center justify-between">
          <SectionHeader title={ko ? '서비스 상태' : 'Service Status'} />
          {onOpenManager && (
            <button
              onClick={onOpenManager}
              className="flex items-center gap-1 text-[10px] text-[#3b82f6] hover:opacity-70 cursor-pointer bg-transparent border-none"
              title={ko ? '매니저에서 열기' : 'Open in Manager'}
            >
              {ko ? '매니저' : 'Manager'}
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>
              </svg>
            </button>
          )}
        </div>
        {/* Core */}
        <div className="px-3 pb-1">
          <span className="text-[9px] uppercase tracking-wider text-[var(--color-text-secondary)] font-medium">Core</span>
        </div>
        {CORE_SERVICES.map(svc => {
          // Testnet: replace L1 Node with external RPC info
          if (isTestnet && svc.localOnly) {
            return (
              <div key={svc.service} className="flex items-center gap-2 px-3 py-2 border-t border-[var(--color-border)]">
                <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ backgroundColor: '#3b82f6' }} />
                <span className="text-[12px] font-medium flex-shrink-0">L1 ({testnetNetworkName})</span>
                <span className="text-[11px] text-[#2563eb]">external</span>
                {l1ChainId ? <code className="text-[10px] font-mono text-[var(--color-text-secondary)]">ID: {l1ChainId}</code> : null}
                {l2.testnetL1RpcUrl && (
                  <code className="text-[10px] font-mono text-[var(--color-text-secondary)] ml-auto truncate max-w-[180px]" title={maskRpcUrl(l2.testnetL1RpcUrl)}>
                    {maskRpcUrl(l2.testnetL1RpcUrl).replace(/^https?:\/\//, '').slice(0, 30)}
                  </code>
                )}
              </div>
            )
          }
          const state = svcState(svc.service)
          const running = state === 'running'
          const port = svc.portKey ? (l2[svc.portKey] ? `:${l2[svc.portKey]}` : null) : null
          const displayPort = port || svcPort(svc.service)
          const chainId = svc.service === 'tokamak-app-l1' ? l1ChainId : svc.service === 'tokamak-app-l2' ? l2ChainId : undefined
          return (
            <div key={svc.service} className="flex items-center gap-2 px-3 py-2 border-t border-[var(--color-border)]">
              <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ backgroundColor: dotColor(state) }} />
              <span className="text-[12px] font-medium flex-shrink-0">{svc.label}</span>
              <span className={`text-[11px] ${running ? 'text-[var(--color-success)]' : 'text-[var(--color-text-secondary)]'}`}>{state}</span>
              {running && chainId ? <code className="text-[10px] font-mono text-[var(--color-text-secondary)]">ID: {chainId}</code> : null}
              {displayPort && <code className="text-[10px] font-mono text-[#3b82f6] ml-auto">{displayPort}</code>}
            </div>
          )
        })}
        {/* Tools */}
        <div className="px-3 pt-2 pb-1 border-t border-[var(--color-border)] flex items-center justify-between">
          <span className="text-[9px] uppercase tracking-wider text-[var(--color-text-secondary)] font-medium">Tools</span>
          {(() => {
            // For testnet mode, skip localOnly services (frontend-l1) when checking tools status
            const relevantTools = isTestnet ? TOOLS_SERVICES.filter(svc => !svc.localOnly) : TOOLS_SERVICES
            const toolsAnyRunning = relevantTools.some(svc => svcState(svc.service) === 'running')
            const toolsAllStopped = relevantTools.every(svc => svcState(svc.service) !== 'running')
            if (!l2.dockerProject) return null
            // Use bridge-ui as the trigger service — it exists in both local and external L1 modes
            return toolsAllStopped ? (
              <button disabled={toolsLoading} onClick={async () => {
                setToolsLoading(true)
                try {
                  const base = `http://127.0.0.1:${import.meta.env.VITE_LOCAL_SERVER_PORT || 5002}`
                  await fetch(`${base}/api/deployments/${l2.id}/service/bridge-ui/start`, { method: 'POST' })
                  onRefresh?.()
                } catch (e) { console.error('Tools start failed:', e) }
                finally { setToolsLoading(false) }
              }}
                className="text-[10px] px-2.5 py-1 rounded-lg bg-[var(--color-success)] text-black font-medium cursor-pointer hover:opacity-80 disabled:opacity-50">
                {toolsLoading ? (ko ? '시작 중...' : 'Starting...') : (ko ? 'Tools 시작' : 'Start Tools')}
              </button>
            ) : toolsAnyRunning ? (
              <button disabled={toolsLoading} onClick={async () => {
                setToolsLoading(true)
                try {
                  const base = `http://127.0.0.1:${import.meta.env.VITE_LOCAL_SERVER_PORT || 5002}`
                  await fetch(`${base}/api/deployments/${l2.id}/service/bridge-ui/stop`, { method: 'POST' })
                  onRefresh?.()
                } catch (e) { console.error('Tools stop failed:', e) }
                finally { setToolsLoading(false) }
              }}
                className="text-[10px] px-2.5 py-1 rounded-lg bg-[var(--color-error)] text-white font-medium cursor-pointer hover:opacity-80 disabled:opacity-50">
                {toolsLoading ? (ko ? '중지 중...' : 'Stopping...') : (ko ? 'Tools 중지' : 'Stop Tools')}
              </button>
            ) : null
          })()}
        </div>
        {TOOLS_SERVICES.map(svc => {
          // Testnet: replace L1 Explorer with public Etherscan link
          if (isTestnet && svc.localOnly) {
            if (!testnetExplorerUrl) return null
            return (
              <div key={svc.service} className="flex items-center gap-2 px-3 py-2 border-t border-[var(--color-border)]">
                <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ backgroundColor: '#3b82f6' }} />
                <span className="text-[12px] font-medium flex-shrink-0">L1 Explorer</span>
                <button
                  onClick={() => openInBrowser(testnetExplorerUrl)}
                  className="ml-auto flex items-center gap-1 text-[10px] font-mono text-[#3b82f6] hover:opacity-70 cursor-pointer bg-transparent border-none"
                >
                  {testnetExplorerUrl.replace('https://', '')}
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>
                  </svg>
                </button>
              </div>
            )
          }
          const state = svcState(svc.service)
          const running = state === 'running'
          const dbPort = svc.portKey ? (l2[svc.portKey] as number | null) : null
          const containerPort = svcPort(svc.service)
          const displayPort = dbPort ? `:${dbPort}` : containerPort
          return (
            <div key={svc.service} className="flex items-center gap-2 px-3 py-2 border-t border-[var(--color-border)]">
              <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ backgroundColor: dotColor(state) }} />
              <span className="text-[12px] font-medium flex-shrink-0">{svc.label}</span>
              <span className={`text-[11px] ${running ? 'text-[var(--color-success)]' : 'text-[var(--color-text-secondary)]'}`}>{state}</span>
              {displayPort && running && (
                <button
                  onClick={() => openInBrowser(`http://localhost${displayPort}`)}
                  className="ml-auto flex items-center gap-1 text-[10px] font-mono text-[#3b82f6] hover:opacity-70 cursor-pointer bg-transparent border-none"
                >
                  {displayPort}
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>
                  </svg>
                </button>
              )}
            </div>
          )
        })}
      </div>

      {/* Error Recovery */}
      {(l2.phase === 'error' || l2.status === 'error') && (
        <div className="bg-[var(--color-error)]/10 rounded-xl p-3 border border-[var(--color-error)]/30">
          <div className="flex items-start gap-2">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="var(--color-error)" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="flex-shrink-0 mt-0.5">
              <circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>
            </svg>
            <div className="flex-1 min-w-0">
              <div className="text-[12px] font-medium text-[var(--color-error)]">
                {ko ? '배포 중 오류가 발생했습니다' : 'Deployment failed'}
              </div>
              {l2.errorMessage && (
                <div className="text-[11px] text-[var(--color-text-secondary)] mt-0.5 break-words">
                  {l2.errorMessage}
                </div>
              )}
              {(l2.bridgeAddress || l2.proposerAddress) && (
                <div className="text-[10px] text-[var(--color-text-secondary)] mt-1">
                  {ko
                    ? '일부 컨트랙트가 이미 배포되었습니다. 재시도 시 기존 컨트랙트를 재사용합니다.'
                    : 'Some contracts were already deployed. Retry will reuse existing contracts.'}
                </div>
              )}
            </div>
          </div>
          {onRetry && (
            <button
              onClick={onRetry}
              disabled={actionLoading}
              className="mt-2 w-full py-2 rounded-xl text-[12px] font-medium bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] text-[var(--color-accent-text)] transition-colors cursor-pointer disabled:opacity-50"
            >
              {actionLoading
                ? (ko ? '재시도 중...' : 'Retrying...')
                : (ko ? '배포 재시도' : 'Retry Deployment')}
            </button>
          )}
        </div>
      )}

      {/* Actions — show contextual button based on container state */}
      {(() => {
        const allStopped = [...CORE_SERVICES, ...TOOLS_SERVICES].every(svc => svcState(svc.service) !== 'running')
        const anyRunning = [...CORE_SERVICES, ...TOOLS_SERVICES].some(svc => svcState(svc.service) === 'running')
        return (
          <div className="flex gap-2">
            {allStopped ? (
              <button disabled={actionLoading} onClick={() => handleAction('start')}
                className="flex-1 bg-[var(--color-success)] text-black text-xs font-medium py-2 rounded-xl hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-50">
                {actionLoading ? (ko ? '시작 중...' : 'Starting...') : (ko ? '전체 시작' : 'Start All')}
              </button>
            ) : anyRunning ? (
              <button disabled={actionLoading} onClick={() => handleAction('stop')}
                className="flex-1 bg-[var(--color-error)] text-white text-xs font-medium py-2 rounded-xl hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-50">
                {actionLoading ? (ko ? '중지 중...' : 'Stopping...') : (ko ? '전체 중지' : 'Stop All')}
              </button>
            ) : null}
          </div>
        )
      })()}

      {/* Products */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <SectionHeader title={ko ? '탑재 제품' : 'Products'} />
        <div className="mt-1 space-y-1.5">
          {products.map(p => (
            <div key={p.name} className="flex items-center gap-2 bg-[var(--color-bg-main)] rounded-lg px-2.5 py-2 border border-[var(--color-border)]">
              <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ backgroundColor: p.status === 'active' ? 'var(--color-success)' : 'var(--color-text-secondary)' }} />
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-1.5">
                  <span className="text-[12px] font-medium">{p.name}</span>
                  <span className="text-[9px] text-[var(--color-tag-text)] bg-[var(--color-tag-bg)] px-1.5 py-0.5 rounded">{p.type}</span>
                </div>
                <div className="text-[10px] text-[var(--color-text-secondary)] truncate">{p.description}</div>
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Deployment Keys (testnet only) */}
      {isTestnet && (() => {
        let parsedConfig: Record<string, unknown> = {}
        try {
          parsedConfig = l2.rawConfig ? JSON.parse(l2.rawConfig) : {}
        } catch { /* ignore */ }
        const testnet = (parsedConfig.testnet || {}) as Record<string, string>
        const deployerKey = testnet.keychainKeyName
        if (!deployerKey) return null
        const roles = [
          { label: 'Deployer', key: deployerKey },
          { label: 'Committer', key: testnet.committerKeychainKey || deployerKey, isDefault: !testnet.committerKeychainKey },
          { label: 'Proof Coordinator', key: testnet.proofCoordinatorKeychainKey || deployerKey, isDefault: !testnet.proofCoordinatorKeychainKey },
          { label: 'Bridge Owner', key: testnet.bridgeOwnerKeychainKey || deployerKey, isDefault: !testnet.bridgeOwnerKeychainKey },
        ]
        const LOW_BALANCE_THRESHOLD = 0.01 // ETH — minimum balance to avoid operational issues
        return (
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <div className="flex items-center justify-between">
              <SectionHeader title={ko ? '배포 키' : 'Deployment Keys'} />
              <button
                onClick={checkRoleBalances}
                disabled={balanceLoading}
                className="text-[10px] px-2 py-1 rounded-lg border border-[var(--color-border)] hover:bg-[var(--color-border)] cursor-pointer disabled:opacity-50"
              >
                {balanceLoading ? (ko ? '확인 중...' : 'Checking...') : (ko ? '잔액 확인' : 'Check Balances')}
              </button>
            </div>
            <div className="mt-1 space-y-1">
              {roles.map(r => {
                const roleKey = r.label.toLowerCase().replace(/ /g, '')
                const balanceInfo = roleBalances?.[roleKey === 'proofcoordinator' ? 'proofCoordinator' : roleKey === 'bridgeowner' ? 'bridgeOwner' : roleKey]
                const balance = balanceInfo ? parseFloat(balanceInfo.balance) : null
                const isLow = balance !== null && balance < LOW_BALANCE_THRESHOLD
                return (
                  <div key={r.label} className="flex items-center justify-between text-[12px] px-1 py-0.5">
                    <span className="text-[var(--color-text-secondary)]">{r.label}</span>
                    <div className="flex items-center gap-2">
                      {balanceInfo && (
                        <span className={`text-[11px] font-medium ${isLow ? 'text-[var(--color-error)]' : 'text-[var(--color-success)]'}`}>
                          {balanceInfo.balance} ETH
                        </span>
                      )}
                      <span className="font-mono text-[11px]">
                        {r.key}
                        {r.isDefault && <span className="text-[var(--color-text-secondary)] ml-1 text-[10px]">(= Deployer)</span>}
                      </span>
                    </div>
                  </div>
                )
              })}
            </div>
            {roleBalances && Object.values(roleBalances).some(r => parseFloat(r.balance) < LOW_BALANCE_THRESHOLD) && (
              <div className="mt-2 p-2 rounded-lg bg-[var(--color-error)]/10 text-[var(--color-error)] text-[11px]">
                {ko ? '잔액이 부족한 역할이 있습니다. ETH를 보충하세요.' : 'Some roles have low balance. Please top up ETH.'}
              </div>
            )}
          </div>
        )
      })()}

      {/* Contracts */}
      {(() => {
        const contracts: { label: string; addr: string }[] = []
        const src = bridgeConfig || {}
        const bridge = src.bridge_address || l2.bridgeAddress
        const proposer = src.on_chain_proposer_address || l2.proposerAddress
        const timelock = src.timelock_address || l2.timelockAddress
        const sp1Verifier = src.sp1_verifier_address || l2.sp1VerifierAddress
        if (bridge) contracts.push({ label: 'CommonBridge', addr: bridge })
        if (proposer) contracts.push({ label: 'OnChainProposer', addr: proposer })
        if (timelock) contracts.push({ label: 'Timelock', addr: timelock })
        if (sp1Verifier) contracts.push({ label: 'SP1 Verifier', addr: sp1Verifier })
        const explorerBase = isTestnet
          ? testnetExplorerUrl
          : l2.toolsL1ExplorerPort ? `http://localhost:${l2.toolsL1ExplorerPort}` : null
        if (contracts.length === 0) return null
        return (
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <SectionHeader title={ko ? 'L1 배포 컨트랙트' : 'L1 Deployed Contracts'} />
            <div className="mt-1 space-y-1.5">
              {contracts.map(c => (
                <div key={c.label} className="flex items-center gap-2 bg-[var(--color-bg-main)] rounded-lg px-2.5 py-2 border border-[var(--color-border)]">
                  <div className="flex-1 min-w-0">
                    <div className="text-[11px] font-medium text-[var(--color-text-secondary)]">{c.label}</div>
                    <div className="text-[10px] font-mono text-[var(--color-text-primary)] truncate">{c.addr}</div>
                  </div>
                  <button
                    onClick={() => copyToClipboard(c.addr)}
                    className="flex-shrink-0 text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer bg-transparent border-none p-0"
                    title={ko ? '주소 복사' : 'Copy address'}
                  >
                    {copiedAddr === c.addr ? (
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--color-success)" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <polyline points="20 6 9 17 4 12"/>
                      </svg>
                    ) : (
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>
                      </svg>
                    )}
                  </button>
                  {explorerBase && (
                    <button
                      onClick={() => openInBrowser(`${explorerBase}/address/${c.addr}`)}
                      className="flex-shrink-0 text-[#3b82f6] hover:opacity-70 cursor-pointer bg-transparent border-none p-0"
                      title={isTestnet ? 'Etherscan' : 'Explorer'}
                    >
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>
                      </svg>
                    </button>
                  )}
                </div>
              ))}
            </div>
          </div>
        )
      })()}
    </>
  )
}
