import { useState, useEffect, useCallback, useMemo } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLang } from '../App'
import { t } from '../i18n'
import { platformAPI } from '../api/platform'
import type { L2Config } from './MyL2View'
import type { Comment } from '../types/comments'

interface Props {
  l2: L2Config
  onBack: () => void
  onRefresh?: () => void
}

type DetailTab = 'overview' | 'economy' | 'services' | 'community' | 'logs'

interface ContainerInfo {
  name: string
  service: string
  state: string
  status: string
  ports: string
}

// --- Mock data (replace with real API later) ---
interface ChainMetrics {
  l1BlockNumber: number
  l2BlockNumber: number
  l1ChainId: number
  l2ChainId: number
  l2Tps: number
  l2BlockTime: number
  totalTxCount: number
  activeAccounts: number
  lastCommittedBatch: number
  lastVerifiedBatch: number
  latestBatch: number
}

interface EconomyMetrics {
  tvl: string
  tvlUsd: string
  nativeToken: string
  l1TokenAddress: string
  l1GasPrice: string
  l2GasPrice: string
  gasRevenue: string
  bridgeDeposits: number
  bridgeWithdrawals: number
}

interface Product {
  name: string
  type: string
  status: 'active' | 'inactive'
  description: string
}

function getMockChainMetrics(l2: L2Config): ChainMetrics {
  const on = l2.status === 'running'
  return {
    l1BlockNumber: on ? 1247 : 0, l2BlockNumber: on ? 3891 : 0,
    l1ChainId: 3151908, l2ChainId: l2.chainId || 65536999,
    l2Tps: on ? 12.4 : 0, l2BlockTime: 2,
    totalTxCount: on ? 48210 : 0, activeAccounts: on ? 156 : 0,
    lastCommittedBatch: on ? 142 : 0, lastVerifiedBatch: on ? 139 : 0, latestBatch: on ? 145 : 0,
  }
}

function getMockEconomyMetrics(l2: L2Config): EconomyMetrics {
  const on = l2.status === 'running'
  return {
    tvl: on ? '125.4 ETH' : '0 ETH', tvlUsd: on ? '$312,500' : '$0',
    nativeToken: 'TON', l1TokenAddress: '0x2be5e8c109e2197D077D13A82dAead6a9b3433C5',
    l1GasPrice: on ? '1.2' : '-', l2GasPrice: on ? '0.001' : '-',
    gasRevenue: on ? '2.18 TON' : '0 TON',
    bridgeDeposits: on ? 342 : 0, bridgeWithdrawals: on ? 89 : 0,
  }
}

function getMockProducts(l2: L2Config): Product[] {
  const base: Product[] = [
    { name: 'Bridge', type: 'infra', status: 'active', description: 'L1↔L2 asset bridge' },
    { name: 'Block Explorer', type: 'tool', status: 'active', description: 'Blockscout-based explorer' },
  ]
  if (l2.programSlug === 'zk-dex') {
    base.unshift({ name: 'ZK-DEX', type: 'dapp', status: 'active', description: 'ZK proof-based decentralized exchange' })
  } else if (l2.programSlug === 'tokamon') {
    base.unshift({ name: 'Tokamon', type: 'dapp', status: 'active', description: 'On-chain gaming state machine' })
  } else {
    base.unshift({ name: 'EVM Runtime', type: 'core', status: 'active', description: 'Full EVM-compatible execution' })
  }
  return base
}
const L2_DETAIL_MOCK_COMMENTS: Comment[] = [
  {
    id: '1', author: 'kim_dev', avatar: 'K', text: 'ZK-DEX 성능이 정말 좋네요! TPS가 어느정도까지 나오나요?',
    time: '2시간 전', likes: 5, liked: false,
    replies: [
      { id: '1-1', author: 'operator_01', avatar: 'O', text: '현재 테스트넷에서 약 12 TPS 정도 나오고 있습니다. 최적화 진행 중이에요.',
        time: '1시간 전', likes: 3, liked: false, replies: [] },
      { id: '1-2', author: 'lee_blockchain', avatar: 'L', text: '저도 비슷한 결과 확인했습니다. 프루버 성능이 핵심인 것 같아요.',
        time: '45분 전', likes: 1, liked: false, replies: [] },
    ],
  },
  {
    id: '2', author: 'eth_researcher', avatar: 'E', text: '브릿지 수수료가 다른 L2 대비 어느정도인가요? 비교 자료가 있으면 좋겠습니다.',
    time: '5시간 전', likes: 8, liked: true,
    replies: [
      { id: '2-1', author: 'operator_01', avatar: 'O', text: 'L1 가스비 기준으로 약 0.001 gwei 수준입니다. 상세 비교는 문서에 추가할 예정입니다.',
        time: '4시간 전', likes: 2, liked: false, replies: [] },
    ],
  },
  {
    id: '3', author: 'web3_builder', avatar: 'W', text: '이 앱체인에 dApp 배포하려면 어떤 절차가 필요한가요?',
    time: '1일 전', likes: 12, liked: false, replies: [],
  },
]
// --- End mock data ---

const CORE_SERVICES = [
  { label: 'L1 Node', service: 'tokamak-app-l1', portKey: 'l1Port' as const },
  { label: 'L2 Node', service: 'tokamak-app-l2', portKey: 'l2Port' as const },
  { label: 'Prover', service: 'tokamak-app-prover', portKey: null },
]

const TOOLS_SERVICES = [
  { label: 'L1 Explorer', service: 'frontend-l1' },
  { label: 'L2 Explorer', service: 'frontend-l2' },
  { label: 'Dashboard', service: 'bridge-ui' },
]

// Reusable UI atoms
const SectionHeader = ({ title }: { title: string }) => (
  <div className="pb-1">
    <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">{title}</span>
  </div>
)

const StatCard = ({ label, value, sub }: { label: string; value: string | number; sub?: string }) => (
  <div className="bg-[var(--color-bg-main)] rounded-lg p-2.5 border border-[var(--color-border)]">
    <div className="text-[10px] text-[var(--color-text-secondary)]">{label}</div>
    <div className="text-[14px] font-semibold mt-0.5 font-mono">{value}</div>
    {sub && <div className="text-[9px] text-[var(--color-text-secondary)] mt-0.5">{sub}</div>}
  </div>
)

const KV = ({ label, value, mono }: { label: string; value: string; mono?: boolean }) => (
  <div className="flex items-center justify-between text-[11px]">
    <span className="text-[var(--color-text-secondary)]">{label}</span>
    <span className={`truncate ml-2 max-w-[200px] ${mono ? 'font-mono text-[10px]' : ''}`}>{value}</span>
  </div>
)

export default function L2DetailView({ l2, onBack, onRefresh }: Props) {
  const { lang } = useLang()
  const ko = lang === 'ko'
  const [activeTab, setActiveTab] = useState<DetailTab>('overview')
  const [containers, setContainers] = useState<ContainerInfo[]>([])
  const [actionLoading, setActionLoading] = useState(false)
  const [isPublic, setIsPublic] = useState(l2.isPublic)
  const [publishing, setPublishing] = useState(false)
  const [publishError, setPublishError] = useState('')
  const [platformLoggedIn, setPlatformLoggedIn] = useState(false)
  const [tags, setTags] = useState<string[]>(l2.hashtags || [])
  const [tagInput, setTagInput] = useState('')
  const [comments, setComments] = useState<Comment[]>(() => [...L2_DETAIL_MOCK_COMMENTS])
  const [commentInput, setCommentInput] = useState('')
  const [replyingTo, setReplyingTo] = useState<string | null>(null)
  const [replyInput, setReplyInput] = useState('')
  const [publishDesc, setPublishDesc] = useState('')
  const [publishScreenshots, setPublishScreenshots] = useState<string[]>([])

  const chain = useMemo(() => getMockChainMetrics(l2), [l2])
  const econ = useMemo(() => getMockEconomyMetrics(l2), [l2])
  const products = useMemo(() => getMockProducts(l2), [l2])

  const fetchContainers = useCallback(async () => {
    try {
      const result = await invoke<ContainerInfo[]>('get_docker_containers', { id: l2.id })
      setContainers(result)
    } catch { /* local-server not reachable */ }
  }, [l2.id])

  useEffect(() => {
    platformAPI.loadToken().then(ok => setPlatformLoggedIn(ok))
    fetchContainers()
    const interval = setInterval(fetchContainers, 5000)
    return () => clearInterval(interval)
  }, [fetchContainers])

  const svcState = (svc: string): string => {
    const c = containers.find(c => c.service === svc || c.name?.includes(svc.replace('tokamak-app-', '').replace('zk-dex-tools-', '')))
    return c ? (c.state || 'stopped') : 'stopped'
  }

  const svcPort = (svc: string): string | null => {
    const c = containers.find(c => c.service === svc || c.name?.includes(svc.replace('tokamak-app-', '').replace('zk-dex-tools-', '')))
    if (!c?.ports) return null
    const m = c.ports.match(/0\.0\.0\.0:(\d+)/)
    return m ? `:${m[1]}` : null
  }

  const handleAction = async (action: 'start' | 'stop') => {
    setActionLoading(true)
    try {
      await invoke(action === 'stop' ? 'stop_docker_deployment' : 'start_docker_deployment', { id: l2.id })
      await fetchContainers()
      onRefresh?.()
    } catch (e) { console.error(`Failed to ${action}:`, e) }
    finally { setActionLoading(false) }
  }

  const dotColor = (state: string) => {
    if (state === 'running') return 'var(--color-success)'
    if (state === 'restarting') return 'var(--color-warning)'
    return 'var(--color-text-secondary)'
  }

  const health = useMemo(() => {
    if (containers.length === 0) return { color: 'var(--color-text-secondary)', label: ko ? '오프라인' : 'Offline' }
    const all = containers.every(c => c.state === 'running')
    const any = containers.some(c => c.state === 'running')
    if (all) return { color: 'var(--color-success)', label: ko ? '정상' : 'Healthy' }
    if (any) return { color: 'var(--color-warning)', label: ko ? '부분 가동' : 'Partial' }
    return { color: 'var(--color-error)', label: ko ? '중지됨' : 'Down' }
  }, [containers, ko])

  const tabs: { id: DetailTab; label: string }[] = [
    { id: 'overview', label: ko ? '개요' : 'Overview' },
    { id: 'economy', label: ko ? '경제' : 'Economy' },
    { id: 'services', label: ko ? '서비스' : 'Services' },
    { id: 'community', label: ko ? '커뮤니티' : 'Community' },
    { id: 'logs', label: ko ? '로그' : 'Logs' },
  ]

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      {/* Header */}
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
        <div className="flex items-center gap-3 mb-2">
          <button onClick={onBack} className="text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer">
            ← {t('openl2.back', lang)}
          </button>
        </div>
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-xl bg-[var(--color-bg-sidebar)] flex items-center justify-center text-xl border border-[var(--color-border)]">
            {l2.icon}
          </div>
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className="text-[13px] font-semibold truncate">{l2.name}</span>
              <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ backgroundColor: health.color }} />
              <span className="text-[11px] font-medium" style={{ color: health.color }}>{health.label}</span>
            </div>
            <div className="text-[11px] text-[var(--color-text-secondary)]">
              {l2.programSlug} · {l2.phase}
              {l2.isPublic && <span className="ml-2 text-[#3b82f6]">{t('myl2.public', lang)}</span>}
            </div>
          </div>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex border-b border-[var(--color-border)] px-1">
        {tabs.map(tab => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`px-2.5 py-2 text-[12px] transition-colors cursor-pointer border-b-2 ${
              activeTab === tab.id
                ? 'border-[var(--color-text-primary)] text-[var(--color-text-primary)] font-medium'
                : 'border-transparent text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'
            }`}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-3 space-y-3">

        {/* ═══ TAB 1: Overview (체인 현황 + 증명) ═══ */}
        {activeTab === 'overview' && (<>
          {/* Chain Status */}
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <SectionHeader title={ko ? '체인 현황' : 'Chain Status'} />
            <div className="grid grid-cols-2 gap-2 mt-1">
              <StatCard label={ko ? 'L1 블록' : 'L1 Block'} value={chain.l1BlockNumber.toLocaleString()} sub={`Chain ID: ${chain.l1ChainId}`} />
              <StatCard label={ko ? 'L2 블록' : 'L2 Block'} value={chain.l2BlockNumber.toLocaleString()} sub={`Chain ID: ${chain.l2ChainId}`} />
            </div>
            <div className="grid grid-cols-3 gap-2 mt-2">
              <StatCard label="TPS" value={chain.l2Tps} sub={`${chain.l2BlockTime}s / block`} />
              <StatCard label={ko ? '트랜잭션' : 'Txs'} value={chain.totalTxCount.toLocaleString()} />
              <StatCard label={ko ? '계정' : 'Accounts'} value={chain.activeAccounts.toLocaleString()} />
            </div>
          </div>

          {/* Proof Progress */}
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <SectionHeader title={ko ? '증명 현황' : 'Proof Progress'} />
            <div className="mt-1 space-y-2">
              {[
                { label: ko ? '최신 배치' : 'Latest Batch', value: chain.latestBatch, color: 'var(--color-text-primary)' },
                { label: ko ? '커밋됨' : 'Committed', value: chain.lastCommittedBatch, color: '#3b82f6' },
                { label: ko ? '검증됨' : 'Verified', value: chain.lastVerifiedBatch, color: 'var(--color-success)' },
              ].map(item => {
                const pct = chain.latestBatch > 0 ? Math.round((item.value / chain.latestBatch) * 100) : 0
                return (
                  <div key={item.label}>
                    <div className="flex justify-between text-[11px] mb-0.5">
                      <span className="text-[var(--color-text-secondary)]">{item.label}</span>
                      <span className="font-mono" style={{ color: item.color }}>#{item.value}</span>
                    </div>
                    <div className="h-1.5 bg-[var(--color-bg-main)] rounded-full overflow-hidden">
                      <div className="h-full rounded-full transition-all" style={{ width: `${pct}%`, backgroundColor: item.color }} />
                    </div>
                  </div>
                )
              })}
            </div>
          </div>

          {/* Hashtags */}
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <SectionHeader title={ko ? '해시태그' : 'Hashtags'} />
            <div className="flex flex-wrap gap-1.5 mt-1">
              {tags.map(tag => (
                <span key={tag} className="text-[11px] bg-[var(--color-tag-bg)] text-[var(--color-tag-text)] px-2 py-0.5 rounded flex items-center gap-1">
                  #{tag}
                  <button
                    onClick={() => setTags(tags.filter(t => t !== tag))}
                    className="text-[var(--color-text-secondary)] hover:text-[var(--color-error)] cursor-pointer text-[10px] leading-none"
                  >×</button>
                </span>
              ))}
              <input
                type="text"
                value={tagInput}
                onChange={e => setTagInput(e.target.value.replace(/\s/g, ''))}
                onKeyDown={e => {
                  if (e.key === 'Enter' && tagInput.trim()) {
                    if (!tags.includes(tagInput.trim())) setTags([...tags, tagInput.trim()])
                    setTagInput('')
                  }
                }}
                placeholder={ko ? '+ 태그 추가' : '+ add tag'}
                className="text-[11px] bg-transparent outline-none w-16 placeholder-[var(--color-text-secondary)]"
              />
            </div>
          </div>
        </>)}

        {/* ═══ TAB 2: Economy (경제 지표) ═══ */}
        {activeTab === 'economy' && (<>
          {/* TVL & Revenue */}
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <SectionHeader title={ko ? '자산 현황' : 'Assets'} />
            <div className="grid grid-cols-2 gap-2 mt-1">
              <StatCard label="TVL" value={econ.tvl} sub={econ.tvlUsd} />
              <StatCard label={ko ? '수수료 수입' : 'Fee Revenue'} value={econ.gasRevenue} />
            </div>
          </div>

          {/* Gas */}
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <SectionHeader title={ko ? '가스비' : 'Gas Prices'} />
            <div className="grid grid-cols-2 gap-2 mt-1">
              <StatCard label={ko ? 'L1 가스' : 'L1 Gas'} value={`${econ.l1GasPrice} gwei`} />
              <StatCard label={ko ? 'L2 가스' : 'L2 Gas'} value={`${econ.l2GasPrice} gwei`} />
            </div>
          </div>

          {/* Bridge */}
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <SectionHeader title={ko ? '브릿지' : 'Bridge'} />
            <div className="grid grid-cols-2 gap-2 mt-1">
              <StatCard label={ko ? '입금' : 'Deposits'} value={econ.bridgeDeposits.toLocaleString()} />
              <StatCard label={ko ? '출금' : 'Withdrawals'} value={econ.bridgeWithdrawals.toLocaleString()} />
            </div>
          </div>

          {/* Token Info */}
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <SectionHeader title={ko ? '토큰 정보' : 'Token Info'} />
            <div className="mt-1 space-y-1.5">
              <KV label={ko ? '네이티브 토큰' : 'Native Token'} value={econ.nativeToken} />
              <div className="flex items-center justify-between text-[11px]">
                <span className="text-[var(--color-text-secondary)]">{ko ? 'L1 토큰 주소' : 'L1 Token'}</span>
                <code className="text-[9px] font-mono text-[#3b82f6] truncate ml-2 max-w-[180px]">{econ.l1TokenAddress}</code>
              </div>
            </div>
          </div>
        </>)}

        {/* ═══ TAB 3: Services (서비스 + 제품 + 컨트랙트 + 설정) ═══ */}
        {activeTab === 'services' && (<>
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
            <div className="px-3 pt-2 pb-1 border-t border-[var(--color-border)]">
              <span className="text-[9px] uppercase tracking-wider text-[var(--color-text-secondary)] font-medium">Tools</span>
            </div>
            {TOOLS_SERVICES.map(svc => {
              const state = svcState(svc.service)
              const running = state === 'running'
              const port = svcPort(svc.service)
              return (
                <div key={svc.service} className="flex items-center gap-2 px-3 py-2 border-t border-[var(--color-border)]">
                  <span className="w-2 h-2 rounded-full flex-shrink-0" style={{ backgroundColor: dotColor(state) }} />
                  <span className="text-[12px] font-medium flex-shrink-0">{svc.label}</span>
                  <span className={`text-[11px] ${running ? 'text-[var(--color-success)]' : 'text-[var(--color-text-secondary)]'}`}>{state}</span>
                  {port && running && (
                    <a href={`http://localhost${port}`} target="_blank" rel="noopener noreferrer"
                      className="text-[10px] font-mono text-[#3b82f6] ml-auto hover:underline" onClick={e => e.stopPropagation()}>
                      {port} ↗
                    </a>
                  )}
                </div>
              )
            })}
          </div>

          {/* Actions */}
          <div className="flex gap-2">
            <button disabled={actionLoading} onClick={() => handleAction('start')}
              className="flex-1 bg-[var(--color-success)] text-black text-xs font-medium py-2 rounded-xl hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-50">
              {actionLoading ? '...' : (ko ? '전체 시작' : 'Start All')}
            </button>
            <button disabled={actionLoading} onClick={() => handleAction('stop')}
              className="flex-1 bg-[var(--color-error)] text-white text-xs font-medium py-2 rounded-xl hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-50">
              {actionLoading ? '...' : (ko ? '전체 중지' : 'Stop All')}
            </button>
          </div>

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
          {(l2.bridgeAddress || l2.proposerAddress) && (
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
              <SectionHeader title={ko ? '컨트랙트' : 'Contracts'} />
              <div className="mt-1 space-y-1.5">
                {l2.bridgeAddress && <KV label="Bridge" value={l2.bridgeAddress} mono />}
                {l2.proposerAddress && <KV label="Proposer" value={l2.proposerAddress} mono />}
              </div>
            </div>
          )}

          {/* Settings */}
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <SectionHeader title={ko ? '설정' : 'Settings'} />
            <div className="mt-1 space-y-1.5">
              <KV label="Chain ID" value={l2.chainId ? String(l2.chainId) : '-'} mono />
              <KV label="L1 RPC" value={l2.l1Rpc || '-'} mono />
              <KV label="Docker" value={l2.dockerProject || '-'} mono />
              <KV label={ko ? '생성일' : 'Created'} value={new Date(l2.createdAt).toLocaleDateString()} />
            </div>
            {/* Public Toggle */}
            <div className="mt-2 pt-2 border-t border-[var(--color-border)] flex items-center justify-between">
              <div>
                <div className="text-[11px] font-medium">{t('myl2.detail.configPublic', lang)}</div>
                <div className="text-[9px] text-[var(--color-text-secondary)]">{t('myl2.detail.configPublicDesc', lang)}</div>
              </div>
              <button
                disabled={publishing || (l2.networkMode === 'local')}
                onClick={async () => {
                  if (!isPublic) {
                    if (!platformLoggedIn) { setPublishError(ko ? 'Platform 로그인 필요' : 'Login required'); return }
                    setPublishing(true); setPublishError('')
                    try {
                      const r = await platformAPI.registerDeployment({ programId: 'ethrex-appchain', name: l2.name, chainId: l2.chainId, rpcUrl: `http://localhost:${l2.rpcPort}` })
                      await platformAPI.activateDeployment(r.deployment.id)
                      setIsPublic(true)
                      await invoke('update_appchain_public', { id: l2.id, isPublic: true })
                      onRefresh?.()
                    } catch (e: unknown) { setPublishError(e instanceof Error ? e.message : String(e)) }
                    finally { setPublishing(false) }
                  } else {
                    setIsPublic(false)
                    try { await invoke('update_appchain_public', { id: l2.id, isPublic: false }); onRefresh?.() }
                    catch { /* ignore */ }
                  }
                }}
                className={`w-10 h-5 rounded-full flex items-center px-0.5 cursor-pointer transition-colors disabled:opacity-50 flex-shrink-0 ${isPublic ? 'bg-[var(--color-accent)]' : 'bg-[var(--color-border)]'}`}
              >
                <div className={`w-4 h-4 bg-white rounded-full transition-transform ${isPublic ? 'translate-x-5' : ''}`} />
              </button>
            </div>
            {publishError && <p className="text-[9px] text-[var(--color-error)] mt-1">{publishError}</p>}
            {publishing && <p className="text-[9px] text-[var(--color-text-secondary)] mt-1">{ko ? '등록 중...' : 'Registering...'}</p>}

            {/* Publish Details (shown when public is on) */}
            {isPublic && (
              <div className="mt-2 pt-2 border-t border-[var(--color-border)] space-y-2">
                <div>
                  <div className="text-[11px] font-medium mb-1">{ko ? '소개글' : 'Description'}</div>
                  <textarea
                    value={publishDesc}
                    onChange={e => setPublishDesc(e.target.value)}
                    placeholder={ko ? '앱체인을 소개하는 글을 작성하세요. 다른 사용자에게 보여집니다.' : 'Describe your appchain. This is shown to other users.'}
                    rows={3}
                    className="w-full bg-[var(--color-bg-main)] rounded-lg px-2.5 py-2 text-[11px] outline-none border border-[var(--color-border)] resize-none"
                  />
                </div>
                <div>
                  <div className="text-[11px] font-medium mb-1">{ko ? '스크린샷' : 'Screenshots'}</div>
                  <div className="flex gap-2 flex-wrap">
                    {publishScreenshots.map((_, i) => (
                      <div key={i} className="relative w-20 h-14 rounded-lg bg-[var(--color-bg-main)] border border-[var(--color-border)] flex items-center justify-center">
                        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--color-text-secondary)]">
                          <rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/>
                        </svg>
                        <button
                          onClick={() => setPublishScreenshots(publishScreenshots.filter((_, j) => j !== i))}
                          className="absolute -top-1 -right-1 w-4 h-4 rounded-full bg-[var(--color-error)] text-white text-[8px] flex items-center justify-center cursor-pointer"
                        >×</button>
                      </div>
                    ))}
                    <button
                      onClick={() => setPublishScreenshots([...publishScreenshots, `screenshot-${Date.now()}`])}
                      className="w-20 h-14 rounded-lg border-2 border-dashed border-[var(--color-border)] flex items-center justify-center text-[var(--color-text-secondary)] hover:border-[#3b82f6] hover:text-[#3b82f6] cursor-pointer transition-colors"
                    >
                      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
                      </svg>
                    </button>
                  </div>
                  <div className="text-[9px] text-[var(--color-text-secondary)] mt-1">
                    {ko ? '앱체인의 화면 캡쳐를 추가하세요 (최대 5장)' : 'Add screenshots of your appchain (max 5)'}
                  </div>
                </div>
              </div>
            )}
          </div>
        </>)}

        {/* ═══ TAB 4: Community (소셜/댓글) ═══ */}
        {activeTab === 'community' && (<>
          {/* Rating & Likes summary */}
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-3">
                {/* Rating */}
                <div className="flex items-center gap-1">
                  {[1, 2, 3, 4, 5].map(star => (
                    <svg key={star} width="14" height="14" viewBox="0 0 24 24"
                      fill={star <= 4 ? '#f59e0b' : 'none'}
                      stroke={star <= 4 ? '#f59e0b' : 'currentColor'}
                      strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
                      className={star <= 4 ? '' : 'text-[var(--color-text-secondary)] opacity-40'}
                    >
                      <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>
                    </svg>
                  ))}
                  <span className="text-[12px] font-semibold ml-0.5">4.2</span>
                  <span className="text-[10px] text-[var(--color-text-secondary)]">(89)</span>
                </div>
                {/* Likes */}
                <div className="flex items-center gap-1 text-[var(--color-text-secondary)]">
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/>
                  </svg>
                  <span className="text-[11px]">248</span>
                </div>
              </div>
              <div className="flex items-center gap-2">
                <span className="text-[10px] text-[var(--color-text-secondary)]">
                  {ko ? `댓글 ${comments.length}개` : `${comments.length} comments`}
                </span>
                <select className="text-[10px] bg-transparent text-[var(--color-text-secondary)] outline-none cursor-pointer">
                  <option>{ko ? '최신순' : 'Newest'}</option>
                  <option>{ko ? '인기순' : 'Popular'}</option>
                </select>
              </div>
            </div>
          </div>

          {/* Comments List */}
          <div className="space-y-2">
            {comments.map(comment => (
              <div key={comment.id} className="bg-[var(--color-bg-sidebar)] rounded-xl border border-[var(--color-border)] overflow-hidden">
                {/* Main comment */}
                <div className="p-3">
                  <div className="flex items-start gap-2.5">
                    <div className="w-7 h-7 rounded-full bg-[var(--color-border)] flex items-center justify-center text-[10px] font-bold flex-shrink-0">
                      {comment.avatar}
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-[12px] font-semibold">{comment.author}</span>
                        <span className="text-[9px] text-[var(--color-text-secondary)]">{comment.time}</span>
                      </div>
                      <p className="text-[12px] mt-1 leading-relaxed">{comment.text}</p>
                      <div className="flex items-center gap-3 mt-2">
                        <button
                          onClick={() => {
                            setComments(comments.map(c => c.id === comment.id ? { ...c, liked: !c.liked, likes: c.liked ? c.likes - 1 : c.likes + 1 } : c))
                          }}
                          className={`flex items-center gap-1 text-[10px] cursor-pointer transition-colors ${comment.liked ? 'text-[#3b82f6]' : 'text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'}`}
                        >
                          <svg width="12" height="12" viewBox="0 0 24 24" fill={comment.liked ? '#3b82f6' : 'none'} stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                            <path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"/>
                          </svg>
                          {comment.likes > 0 && comment.likes}
                        </button>
                        <button
                          onClick={() => setReplyingTo(replyingTo === comment.id ? null : comment.id)}
                          className="text-[10px] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer"
                        >
                          {ko ? '답글' : 'Reply'}
                        </button>
                      </div>
                    </div>
                  </div>
                </div>

                {/* Replies */}
                {comment.replies.length > 0 && (
                  <div className="border-t border-[var(--color-border)] bg-[var(--color-bg-main)]">
                    {comment.replies.map(reply => (
                      <div key={reply.id} className="px-3 py-2.5 ml-6 border-b border-[var(--color-border)] last:border-b-0">
                        <div className="flex items-start gap-2">
                          <div className="w-5 h-5 rounded-full bg-[var(--color-border)] flex items-center justify-center text-[8px] font-bold flex-shrink-0">
                            {reply.avatar}
                          </div>
                          <div className="flex-1 min-w-0">
                            <div className="flex items-center gap-2">
                              <span className="text-[11px] font-semibold">{reply.author}</span>
                              <span className="text-[9px] text-[var(--color-text-secondary)]">{reply.time}</span>
                            </div>
                            <p className="text-[11px] mt-0.5 leading-relaxed">{reply.text}</p>
                            <button
                              onClick={() => {
                                setComments(comments.map(c => c.id === comment.id
                                  ? { ...c, replies: c.replies.map(r => r.id === reply.id ? { ...r, liked: !r.liked, likes: r.liked ? r.likes - 1 : r.likes + 1 } : r) }
                                  : c))
                              }}
                              className={`flex items-center gap-1 text-[9px] mt-1 cursor-pointer transition-colors ${reply.liked ? 'text-[#3b82f6]' : 'text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'}`}
                            >
                              <svg width="10" height="10" viewBox="0 0 24 24" fill={reply.liked ? '#3b82f6' : 'none'} stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                                <path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"/>
                              </svg>
                              {reply.likes > 0 && reply.likes}
                            </button>
                          </div>
                        </div>
                      </div>
                    ))}
                  </div>
                )}

                {/* Reply Input */}
                {replyingTo === comment.id && (
                  <div className="border-t border-[var(--color-border)] p-3 bg-[var(--color-bg-main)]">
                    <div className="flex items-start gap-2 ml-6">
                      <div className="w-5 h-5 rounded-full bg-[var(--color-accent)] flex items-center justify-center text-[8px] font-bold text-[var(--color-accent-text)] flex-shrink-0 mt-0.5">
                        Me
                      </div>
                      <div className="flex-1">
                        {(() => {
                          const submitReply = () => {
                            if (!replyInput.trim()) return
                            const newReply: Comment = {
                              id: `reply-${Date.now()}`, author: 'me', avatar: 'Me', text: replyInput.trim(),
                              time: ko ? '방금' : 'Just now', likes: 0, liked: false, replies: [],
                            }
                            setComments(comments.map(c => c.id === comment.id ? { ...c, replies: [...c.replies, newReply] } : c))
                            setReplyInput('')
                            setReplyingTo(null)
                          }
                          return (<>
                        <input
                          type="text"
                          value={replyInput}
                          onChange={e => setReplyInput(e.target.value)}
                          placeholder={ko ? '답글을 입력하세요...' : 'Write a reply...'}
                          onKeyDown={e => { if (e.key === 'Enter') submitReply() }}
                          className="w-full bg-[var(--color-bg-sidebar)] rounded-lg px-2.5 py-1.5 text-[11px] outline-none border border-[var(--color-border)]"
                          autoFocus
                        />
                        <div className="flex items-center gap-2 mt-1.5">
                          <button
                            onClick={submitReply}
                            disabled={!replyInput.trim()}
                            className="bg-[#3b82f6] text-white text-[10px] font-medium px-3 py-1 rounded-lg hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-40"
                          >
                            {ko ? '등록' : 'Post'}
                          </button>
                          <button
                            onClick={() => { setReplyingTo(null); setReplyInput('') }}
                            className="text-[10px] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer"
                          >
                            {ko ? '취소' : 'Cancel'}
                          </button>
                        </div>
                          </>)
                        })()}
                      </div>
                    </div>
                  </div>
                )}
              </div>
            ))}
          </div>

          {/* Write new comment - bottom */}
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <div className="flex items-start gap-2.5">
              <div className="w-7 h-7 rounded-full bg-[var(--color-accent)] flex items-center justify-center text-[10px] font-bold text-[var(--color-accent-text)] flex-shrink-0 mt-0.5">
                Me
              </div>
              <div className="flex-1">
                <textarea
                  value={commentInput}
                  onChange={e => setCommentInput(e.target.value)}
                  placeholder={ko ? '질문이나 의견을 남겨보세요...' : 'Ask a question or leave a comment...'}
                  rows={2}
                  className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-[12px] outline-none border border-[var(--color-border)] resize-none"
                />
                <div className="flex justify-end mt-1.5">
                  <button
                    onClick={() => {
                      if (!commentInput.trim()) return
                      const newComment: Comment = {
                        id: `new-${Date.now()}`, author: 'me', avatar: 'Me', text: commentInput.trim(),
                        time: ko ? '방금' : 'Just now', likes: 0, liked: false, replies: [],
                      }
                      setComments([newComment, ...comments])
                      setCommentInput('')
                    }}
                    disabled={!commentInput.trim()}
                    className="bg-[#3b82f6] text-white text-[11px] font-medium px-4 py-1.5 rounded-lg hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-40"
                  >
                    {ko ? '등록' : 'Post'}
                  </button>
                </div>
              </div>
            </div>
          </div>
        </>)}

        {/* ═══ TAB 5: Logs ═══ */}
        {activeTab === 'logs' && (
          <div className="bg-black rounded-xl p-4 font-mono text-[11px] text-green-400 h-full min-h-[400px] overflow-auto border border-[var(--color-border)]">
            <div className="text-[var(--color-text-secondary)]">[{l2.name}] {t('myl2.detail.logsPlaceholder', lang)}</div>
            <div className="mt-2 text-gray-500">$ ethrex --chain-id {l2.chainId} --port {l2.rpcPort}</div>
            <div className="text-gray-500">INFO: Starting sequencer...</div>
            <div className="text-gray-500">INFO: Listening on 0.0.0.0:{l2.rpcPort}</div>
            <div className="text-gray-500">INFO: Block #1 produced</div>
            <div className="text-gray-500">INFO: Block #2 produced</div>
            <div className="animate-pulse mt-1">▊</div>
          </div>
        )}

      </div>
    </div>
  )
}
