import { useState, useEffect, useCallback, useMemo } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLang } from '../App'
import { t } from '../i18n'
import { platformAPI } from '../api/platform'
import type { L2Config } from './MyL2View'
import type { Comment } from '../types/comments'
import CommentSection from './CommentSection'
import L2DetailOverviewTab from './L2DetailOverviewTab'
import L2DetailEconomyTab from './L2DetailEconomyTab'
import L2DetailServicesTab from './L2DetailServicesTab'
import L2DetailLogsTab from './L2DetailLogsTab'

interface Props {
  l2: L2Config
  onBack: () => void
  onRefresh?: () => void
}

type DetailTab = 'overview' | 'economy' | 'services' | 'community' | 'logs'

export interface ContainerInfo {
  name: string
  service: string
  state: string
  status: string
  ports: string
}

// --- Mock data (replace with real API later) ---
export interface ChainMetrics {
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

export interface EconomyMetrics {
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

export interface Product {
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

  const handleAction = async (action: 'start' | 'stop') => {
    setActionLoading(true)
    try {
      await invoke(action === 'stop' ? 'stop_docker_deployment' : 'start_docker_deployment', { id: l2.id })
      await fetchContainers()
      onRefresh?.()
    } catch (e) { console.error(`Failed to ${action}:`, e) }
    finally { setActionLoading(false) }
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

        {activeTab === 'overview' && (
          <L2DetailOverviewTab
            ko={ko} chain={chain}
            tags={tags} setTags={setTags}
            tagInput={tagInput} setTagInput={setTagInput}
          />
        )}

        {activeTab === 'economy' && (
          <L2DetailEconomyTab ko={ko} econ={econ} />
        )}

        {activeTab === 'services' && (
          <L2DetailServicesTab
            l2={l2} ko={ko} containers={containers} products={products}
            actionLoading={actionLoading} handleAction={handleAction}
            isPublic={isPublic} setIsPublic={setIsPublic}
            publishing={publishing} setPublishing={setPublishing}
            publishError={publishError} setPublishError={setPublishError}
            platformLoggedIn={platformLoggedIn}
            publishDesc={publishDesc} setPublishDesc={setPublishDesc}
            publishScreenshots={publishScreenshots} setPublishScreenshots={setPublishScreenshots}
            onRefresh={onRefresh}
          />
        )}

        {/* ═══ TAB 4: Community (소셜/댓글) — not yet extracted ═══ */}
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
            </div>
          </div>

          <CommentSection comments={comments} onCommentsChange={setComments} ko={ko} />
        </>)}

        {activeTab === 'logs' && (
          <L2DetailLogsTab l2={l2} />
        )}

      </div>
    </div>
  )
}
