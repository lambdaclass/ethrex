import { useState, useEffect } from 'react'
import { useLang } from '../App'
import { SectionHeader } from './ui-atoms'
import type { L2Config } from './MyL2View'
import type { ContainerInfo, Product } from './L2DetailView'

interface BridgeUIConfig {
  bridge_address?: string
  on_chain_proposer_address?: string
  timelock_address?: string
  sp1_verifier_address?: string
}

const SERVICE_NAME_PREFIXES = ['tokamak-app-', 'zk-dex-tools-'] as const

const CORE_SERVICES = [
  { label: 'L1 Node', service: 'tokamak-app-l1', portKey: 'l1Port' as const },
  { label: 'L2 Node', service: 'tokamak-app-l2', portKey: 'l2Port' as const },
  { label: 'Prover', service: 'tokamak-app-prover', portKey: null },
]

const TOOLS_SERVICES: { label: string; service: string; portKey: keyof L2Config | null }[] = [
  { label: 'L1 Explorer', service: 'frontend-l1', portKey: 'toolsL1ExplorerPort' },
  { label: 'L2 Explorer', service: 'frontend-l2', portKey: 'toolsL2ExplorerPort' },
  { label: 'Dashboard', service: 'bridge-ui', portKey: 'toolsBridgeUIPort' },
]

interface Props {
  l2: L2Config
  ko: boolean
  containers: ContainerInfo[]
  products: Product[]
  actionLoading: boolean
  handleAction: (action: 'start' | 'stop') => void
  onRefresh?: () => void
}

export default function L2DetailServicesTab({
  l2, ko, containers, products, actionLoading, handleAction,
  onRefresh,
}: Props) {
  const [toolsLoading, setToolsLoading] = useState(false)
  const [bridgeConfig, setBridgeConfig] = useState<BridgeUIConfig | null>(null)

  useEffect(() => {
    if (!l2.toolsBridgeUIPort) return
    fetch(`http://localhost:${l2.toolsBridgeUIPort}/config.json`)
      .then(r => r.ok ? r.json() : null)
      .then(data => { if (data) setBridgeConfig(data) })
      .catch(() => {})
  }, [l2.toolsBridgeUIPort])

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

  return (
    <>
      {/* Docker Services */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl border border-[var(--color-border)] overflow-hidden">
        <div className="px-3 pt-3 pb-1">
          <SectionHeader title={ko ? '서비스 상태' : 'Service Status'} />
        </div>
        {/* Core */}
        <div className="px-3 pb-1">
          <span className="text-[9px] uppercase tracking-wider text-[var(--color-text-secondary)] font-medium">Core</span>
        </div>
        {CORE_SERVICES.map(svc => {
          const state = svcState(svc.service)
          const running = state === 'running'
          const port = svc.portKey ? (l2[svc.portKey] ? `:${l2[svc.portKey]}` : null) : null
          const displayPort = port || svcPort(svc.service)
          return (
            <div key={svc.service} className="flex items-center gap-2 px-3 py-2 border-t border-[var(--color-border)]">
              <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ backgroundColor: dotColor(state) }} />
              <span className="text-[12px] font-medium flex-shrink-0">{svc.label}</span>
              <span className={`text-[11px] ${running ? 'text-[var(--color-success)]' : 'text-[var(--color-text-secondary)]'}`}>{state}</span>
              {displayPort && <code className="text-[10px] font-mono text-[#3b82f6] ml-auto">{displayPort}</code>}
            </div>
          )
        })}
        {/* Tools */}
        <div className="px-3 pt-2 pb-1 border-t border-[var(--color-border)] flex items-center justify-between">
          <span className="text-[9px] uppercase tracking-wider text-[var(--color-text-secondary)] font-medium">Tools</span>
          {(() => {
            const toolsAnyRunning = TOOLS_SERVICES.some(svc => svcState(svc.service) === 'running')
            const toolsAllStopped = TOOLS_SERVICES.every(svc => svcState(svc.service) !== 'running')
            if (!l2.dockerProject) return null
            return toolsAllStopped ? (
              <button disabled={toolsLoading} onClick={async () => {
                setToolsLoading(true)
                try {
                  const base = `http://127.0.0.1:${import.meta.env.VITE_LOCAL_SERVER_PORT || 5002}`
                  await fetch(`${base}/api/deployments/${l2.id}/service/frontend-l1/start`, { method: 'POST' })
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
                  await fetch(`${base}/api/deployments/${l2.id}/service/frontend-l1/stop`, { method: 'POST' })
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
        const explorerBase = l2.toolsL1ExplorerPort ? `http://localhost:${l2.toolsL1ExplorerPort}` : null
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
                  {explorerBase && (
                    <button
                      onClick={() => openInBrowser(`${explorerBase}/address/${c.addr}`)}
                      className="flex-shrink-0 text-[#3b82f6] hover:opacity-70 cursor-pointer bg-transparent border-none p-0"
                      title="Local Explorer"
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
