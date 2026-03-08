import { useState } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'
import type { Comment } from '../types/comments'

const OPEN_L2_MOCK_COMMENTS: Comment[] = [
  {
    id: '1', author: 'alice_dev', avatar: 'A', text: '이 앱체인 TPS가 꽤 높네요! 메인넷에서도 이 정도 나오나요?',
    time: '30분 전', likes: 7, liked: false,
    replies: [
      { id: '1-1', author: 'operator', avatar: 'OP', text: '네, 메인넷에서도 비슷한 수준입니다. 프루버 최적화 덕분이에요.',
        time: '20분 전', likes: 3, liked: false, replies: [] },
    ],
  },
  {
    id: '2', author: 'bob_web3', avatar: 'B', text: 'RPC 연결 가이드가 있나요? MetaMask 설정 방법이 궁금합니다.',
    time: '2시간 전', likes: 4, liked: true,
    replies: [],
  },
  {
    id: '3', author: 'carol_dao', avatar: 'C', text: '브릿지 수수료가 정말 저렴하네요. 다른 L2 대비 경쟁력 있습니다 👍',
    time: '5시간 전', likes: 15, liked: false,
    replies: [
      { id: '3-1', author: 'dave_trader', avatar: 'D', text: '동의합니다. 저도 여기로 옮길 생각 중이에요.',
        time: '4시간 전', likes: 2, liked: false, replies: [] },
      { id: '3-2', author: 'operator', avatar: 'OP', text: '감사합니다! 앞으로도 비용 효율성에 집중하겠습니다.',
        time: '3시간 전', likes: 5, liked: false, replies: [] },
    ],
  },
]

// Mock asset balances per appchain (static data, outside component)
const MOCK_ASSETS: Record<string, { symbol: string; name: string; balance: string; usd: string }[]> = {
  '1': [
    { symbol: 'TON', name: 'Tokamak Network', balance: '1,250.45', usd: '$2,500.90' },
    { symbol: 'ETH', name: 'Ethereum', balance: '0.85', usd: '$2,125.00' },
    { symbol: 'WTON', name: 'Wrapped TON', balance: '500.00', usd: '$1,000.00' },
  ],
  '5': [
    { symbol: 'TON', name: 'Tokamak Network', balance: '320.12', usd: '$640.24' },
    { symbol: 'ETH', name: 'Ethereum', balance: '2.10', usd: '$5,250.00' },
  ],
}

const DEFAULT_ASSETS = [
  { symbol: 'TON', name: 'Tokamak Network', balance: '0.00', usd: '$0.00' },
  { symbol: 'ETH', name: 'Ethereum', balance: '0.00', usd: '$0.00' },
]

interface L2Service {
  id: string
  name: string
  icon: string
  description: string
  operator: string
  members: number
  hashtags: string[]
  status: 'online' | 'offline'
  chainId: number
  rpcUrl: string
  lastActive: string
  tvl: string
  tvlUsd: string
}

const sampleL2s: L2Service[] = [
  {
    id: '1', name: 'Tokamak DEX', icon: '🔄', description: 'Decentralized exchange with AMM pools on Tokamak L2',
    operator: 'tokamak-team', members: 1234, hashtags: ['DeFi', 'DEX', 'AMM', 'Swap'],
    status: 'online', chainId: 17001, rpcUrl: 'https://rpc.dex.tokamak.network', lastActive: '2m ago',
    tvl: '1,250 ETH', tvlUsd: '$3.1M',
  },
  {
    id: '2', name: 'NFT Marketplace L2', icon: '🎨', description: 'NFT minting and trading platform with low gas fees',
    operator: 'nft-builder', members: 856, hashtags: ['NFT', 'Art', 'Marketplace', 'Minting'],
    status: 'online', chainId: 17002, rpcUrl: 'https://rpc.nft.example.com', lastActive: '5m ago',
    tvl: '340 ETH', tvlUsd: '$850K',
  },
  {
    id: '3', name: 'GameFi World', icon: '🎮', description: 'Gaming L2 with on-chain game state and item trading',
    operator: 'gamefi-studio', members: 2341, hashtags: ['Gaming', 'GameFi', 'P2E', 'Items'],
    status: 'online', chainId: 17003, rpcUrl: 'https://rpc.gamefi.example.com', lastActive: '1m ago',
    tvl: '890 ETH', tvlUsd: '$2.2M',
  },
  {
    id: '4', name: 'Social Protocol', icon: '💬', description: 'Decentralized social media protocol on L2',
    operator: 'social-dao', members: 567, hashtags: ['Social', 'DAO', 'Messaging', 'Identity'],
    status: 'offline', chainId: 17004, rpcUrl: 'https://rpc.social.example.com', lastActive: '2h ago',
    tvl: '45 ETH', tvlUsd: '$112K',
  },
  {
    id: '5', name: 'Bridge Hub', icon: '🌉', description: 'Cross-chain bridge aggregator connecting multiple L2s',
    operator: 'bridge-labs', members: 3120, hashtags: ['Bridge', 'CrossChain', 'Interop'],
    status: 'online', chainId: 17005, rpcUrl: 'https://rpc.bridge.example.com', lastActive: '30s ago',
    tvl: '5,600 ETH', tvlUsd: '$14M',
  },
  {
    id: '6', name: 'DeFi Lending', icon: '🏦', description: 'Lending and borrowing protocol with TON collateral',
    operator: 'lending-protocol', members: 912, hashtags: ['DeFi', 'Lending', 'Borrow', 'TON'],
    status: 'online', chainId: 17006, rpcUrl: 'https://rpc.lending.example.com', lastActive: '10m ago',
    tvl: '2,100 ETH', tvlUsd: '$5.2M',
  },
  {
    id: '7', name: 'ZK Privacy L2', icon: '🔒', description: 'Privacy-preserving transactions using ZK proofs',
    operator: 'privacy-labs', members: 445, hashtags: ['Privacy', 'ZK', 'Confidential', 'Anonymous'],
    status: 'offline', chainId: 17007, rpcUrl: 'https://rpc.privacy.example.com', lastActive: '1d ago',
    tvl: '78 ETH', tvlUsd: '$195K',
  },
  {
    id: '8', name: 'AI Agent Hub', icon: '🤖', description: 'L2 specialized for AI agent operations and micro-transactions',
    operator: 'ai-hub-team', members: 1678, hashtags: ['AI', 'Agent', 'Automation', 'MicroTx'],
    status: 'online', chainId: 17008, rpcUrl: 'https://rpc.ai-hub.example.com', lastActive: '15s ago',
    tvl: '720 ETH', tvlUsd: '$1.8M',
  },
]

const popularTags = ['전체', 'DeFi', 'NFT', 'Gaming', 'Bridge', 'Social', 'AI', 'Privacy', 'DAO']

export default function OpenL2View() {
  const { lang } = useLang()
  const ko = lang === 'ko'
  const [searchQuery, setSearchQuery] = useState('')
  const [selectedTag, setSelectedTag] = useState('전체')
  const [selectedL2, setSelectedL2] = useState<L2Service | null>(null)
  const [listTab, setListTab] = useState<'all' | 'favorites' | 'bookmarks'>('all')
  const [favoriteIds, setFavoriteIds] = useState<Set<string>>(new Set(['1', '3', '8'])) // mock: some pre-liked
  const [bookmarkedIds, setBookmarkedIds] = useState<Set<string>>(new Set(['1', '5'])) // mock: some pre-bookmarked
  const [walletAddress, setWalletAddress] = useState('') // single shared address for all bookmarks
  const [walletInput, setWalletInput] = useState('')
  const [editingWallet, setEditingWallet] = useState(false)

  const toggleFavorite = (id: string) => {
    setFavoriteIds(prev => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const toggleBookmark = (id: string) => {
    setBookmarkedIds(prev => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const filtered = sampleL2s.filter(l2 => {
    if (listTab === 'favorites' && !favoriteIds.has(l2.id)) return false
    if (listTab === 'bookmarks' && !bookmarkedIds.has(l2.id)) return false
    const matchesSearch = searchQuery === '' ||
      l2.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
      l2.description.toLowerCase().includes(searchQuery.toLowerCase()) ||
      l2.hashtags.some(tag => tag.toLowerCase().includes(searchQuery.toLowerCase()))
    const matchesTag = selectedTag === '전체' || l2.hashtags.includes(selectedTag)
    return matchesSearch && matchesTag
  })

  // Detail view state
  const [comments, setComments] = useState<Comment[]>(() => [...OPEN_L2_MOCK_COMMENTS])
  const [commentInput, setCommentInput] = useState('')
  const [replyingTo, setReplyingTo] = useState<string | null>(null)
  const [replyInput, setReplyInput] = useState('')
  const [l2Liked, setL2Liked] = useState(false)
  const [l2LikeCount, setL2LikeCount] = useState(248)
  const [userRating, setUserRating] = useState(0)
  const [openDetailTab, setOpenDetailTab] = useState<'overview' | 'community'>('overview')

  if (selectedL2) {
    const mockScreenshots = [
      { label: ko ? '메인 화면' : 'Main Screen', color: '#3b82f6' },
      { label: ko ? '거래 화면' : 'Trading', color: '#8b5cf6' },
      { label: ko ? '브릿지' : 'Bridge', color: '#10b981' },
    ]
    const avgRating = 4.2
    const ratingCount = 89
    const detailTabs: { id: 'overview' | 'community'; label: string }[] = [
      { id: 'overview', label: ko ? '개요' : 'Overview' },
      { id: 'community', label: ko ? '커뮤니티' : 'Community' },
    ]

    return (
      <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
        {/* Header */}
        <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
          <button
            onClick={() => { setSelectedL2(null); setReplyingTo(null); setReplyInput(''); setCommentInput(''); setOpenDetailTab('overview') }}
            className="text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer mb-2"
          >
            ← {t('openl2.back', lang)}
          </button>
          {/* Hero */}
          <div className="flex items-start gap-3">
            <div className="w-12 h-12 rounded-2xl bg-[var(--color-bg-main)] flex items-center justify-center text-2xl border border-[var(--color-border)] flex-shrink-0">
              {selectedL2.icon}
            </div>
            <div className="flex-1 min-w-0">
              <h1 className="text-[14px] font-bold">{selectedL2.name}</h1>
              <div className="text-[10px] text-[var(--color-text-secondary)]">by {selectedL2.operator}</div>
              <div className="flex items-center gap-3 mt-0.5">
                <div className="flex items-center gap-1">
                  <span className={`w-2 h-2 rounded-full ${selectedL2.status === 'online' ? 'bg-[var(--color-success)]' : 'bg-[var(--color-text-secondary)]'}`} />
                  <span className="text-[10px] text-[var(--color-text-secondary)]">
                    {selectedL2.status === 'online' ? t('openl2.online', lang) : t('openl2.offline', lang)}
                  </span>
                </div>
                <span className="text-[10px] text-[var(--color-text-secondary)]">{selectedL2.members.toLocaleString()} {t('openl2.users', lang)}</span>
                {/* Rating inline */}
                <div className="flex items-center gap-0.5">
                  <svg width="10" height="10" viewBox="0 0 24 24" fill="#f59e0b" stroke="#f59e0b" strokeWidth="2"><polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/></svg>
                  <span className="text-[10px] font-medium">{avgRating}</span>
                  <span className="text-[9px] text-[var(--color-text-secondary)]">({ratingCount})</span>
                </div>
              </div>
            </div>
            {/* Like */}
            <button
              onClick={() => { setL2Liked(!l2Liked); setL2LikeCount(l2Liked ? l2LikeCount - 1 : l2LikeCount + 1) }}
              className={`flex flex-col items-center gap-0.5 px-2 py-1.5 rounded-xl border transition-colors cursor-pointer flex-shrink-0 ${
                l2Liked ? 'border-[#ef4444] bg-[#ef4444]/10 text-[#ef4444]' : 'border-[var(--color-border)] text-[var(--color-text-secondary)] hover:border-[#ef4444] hover:text-[#ef4444]'
              }`}
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill={l2Liked ? '#ef4444' : 'none'} stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/>
              </svg>
              <span className="text-[9px] font-medium">{l2LikeCount}</span>
            </button>
          </div>
        </div>

        {/* Tabs */}
        <div className="flex border-b border-[var(--color-border)] px-1">
          {detailTabs.map(tab => (
            <button key={tab.id} onClick={() => setOpenDetailTab(tab.id)}
              className={`px-2.5 py-2 text-[12px] transition-colors cursor-pointer border-b-2 ${
                openDetailTab === tab.id
                  ? 'border-[var(--color-text-primary)] text-[var(--color-text-primary)] font-medium'
                  : 'border-transparent text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'
              }`}
            >{tab.label}</button>
          ))}
          {/* Dashboard link in tab bar */}
          <div className="flex-1" />
          <button
            onClick={() => window.open(`http://dashboard.example.com/${selectedL2.chainId}`, '_blank')}
            className="flex items-center gap-1 text-[10px] text-[#3b82f6] hover:underline cursor-pointer px-2 py-2"
          >
            {ko ? '대시보드' : 'Dashboard'}
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>
            </svg>
          </button>
        </div>

        {/* Tab Content */}
        <div className="flex-1 overflow-y-auto p-3 space-y-3">

          {/* ═══ Overview ═══ */}
          {openDetailTab === 'overview' && (<>
            {/* Hashtags */}
            <div className="flex flex-wrap gap-1.5">
              {selectedL2.hashtags.map(tag => (
                <span key={tag} className="text-[11px] bg-[var(--color-tag-bg)] px-2 py-0.5 rounded text-[var(--color-tag-text)]">#{tag}</span>
              ))}
            </div>

            {/* Description */}
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">{ko ? '소개' : 'About'}</span>
              <p className="text-[12px] mt-1.5 leading-relaxed">{selectedL2.description}</p>
              <p className="text-[12px] mt-2 leading-relaxed text-[var(--color-text-secondary)]">
                {ko
                  ? '이 앱체인은 Tokamak Network 기반의 L2 롤업으로, 고성능 트랜잭션 처리와 낮은 수수료를 제공합니다. ZK 증명을 통해 L1의 보안성을 그대로 유지합니다.'
                  : 'This appchain is an L2 rollup built on Tokamak Network, offering high-performance transaction processing with low fees. ZK proofs maintain full L1 security guarantees.'}
              </p>
            </div>

            {/* Screenshots */}
            <div>
              <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)] px-1">{ko ? '스크린샷' : 'Screenshots'}</span>
              <div className="flex gap-2 mt-1.5 overflow-x-auto pb-1">
                {mockScreenshots.map((s, i) => (
                  <div key={i} className="flex-shrink-0 w-36 h-24 rounded-xl border border-[var(--color-border)] flex items-center justify-center" style={{ backgroundColor: `${s.color}15` }}>
                    <div className="text-center">
                      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke={s.color} strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" className="mx-auto mb-1">
                        <rect x="2" y="3" width="20" height="14" rx="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/>
                      </svg>
                      <span className="text-[9px]" style={{ color: s.color }}>{s.label}</span>
                    </div>
                  </div>
                ))}
              </div>
            </div>

            {/* Quick Stats */}
            <div className="grid grid-cols-3 gap-2">
              <div className="bg-[var(--color-bg-sidebar)] rounded-lg p-2.5 border border-[var(--color-border)]">
                <div className="text-[10px] text-[var(--color-text-secondary)]">TVL</div>
                <div className="text-[14px] font-bold font-mono mt-0.5">{selectedL2.tvlUsd}</div>
                <div className="text-[9px] text-[var(--color-text-secondary)]">{selectedL2.tvl}</div>
              </div>
              <div className="bg-[var(--color-bg-sidebar)] rounded-lg p-2.5 border border-[var(--color-border)]">
                <div className="text-[10px] text-[var(--color-text-secondary)]">{ko ? '사용자' : 'Users'}</div>
                <div className="text-[14px] font-bold font-mono mt-0.5">{selectedL2.members.toLocaleString()}</div>
              </div>
              <div className="bg-[var(--color-bg-sidebar)] rounded-lg p-2.5 border border-[var(--color-border)]">
                <div className="text-[10px] text-[var(--color-text-secondary)]">TPS</div>
                <div className="text-[14px] font-bold font-mono mt-0.5">12.4</div>
                <div className="text-[9px] text-[var(--color-text-secondary)]">2s / block</div>
              </div>
            </div>

            {/* Gas & Bridge */}
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">{ko ? '가스 · 브릿지' : 'Gas & Bridge'}</span>
              <div className="grid grid-cols-4 gap-2 mt-1.5">
                <div className="bg-[var(--color-bg-main)] rounded-lg p-2 border border-[var(--color-border)]">
                  <div className="text-[9px] text-[var(--color-text-secondary)]">{ko ? 'L2 가스' : 'L2 Gas'}</div>
                  <div className="text-[12px] font-semibold font-mono mt-0.5">0.001</div>
                  <div className="text-[8px] text-[var(--color-text-secondary)]">gwei</div>
                </div>
                <div className="bg-[var(--color-bg-main)] rounded-lg p-2 border border-[var(--color-border)]">
                  <div className="text-[9px] text-[var(--color-text-secondary)]">{ko ? '수수료' : 'Revenue'}</div>
                  <div className="text-[12px] font-semibold font-mono mt-0.5">2.18</div>
                  <div className="text-[8px] text-[var(--color-text-secondary)]">TON</div>
                </div>
                <div className="bg-[var(--color-bg-main)] rounded-lg p-2 border border-[var(--color-border)]">
                  <div className="text-[9px] text-[var(--color-text-secondary)]">{ko ? '입금' : 'Deposits'}</div>
                  <div className="text-[12px] font-semibold font-mono mt-0.5">342</div>
                </div>
                <div className="bg-[var(--color-bg-main)] rounded-lg p-2 border border-[var(--color-border)]">
                  <div className="text-[9px] text-[var(--color-text-secondary)]">{ko ? '출금' : 'Withdraw'}</div>
                  <div className="text-[12px] font-semibold font-mono mt-0.5">89</div>
                </div>
              </div>
            </div>

            {/* Connection Info */}
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">{t('openl2.connectionInfo', lang)}</span>
              <div className="mt-1.5 space-y-1 text-[11px]">
                <div className="flex justify-between">
                  <span className="text-[var(--color-text-secondary)]">Chain ID</span>
                  <span className="font-mono">{selectedL2.chainId}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-[var(--color-text-secondary)]">RPC</span>
                  <code className="font-mono text-[10px] text-[#3b82f6]">{selectedL2.rpcUrl}</code>
                </div>
                <div className="flex justify-between">
                  <span className="text-[var(--color-text-secondary)]">{ko ? '네이티브 토큰' : 'Native Token'}</span>
                  <span>TON</span>
                </div>
              </div>
            </div>
          </>)}

          {/* ═══ Community ═══ */}
          {openDetailTab === 'community' && (<>
            {/* My Rating */}
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">{ko ? '내 평점' : 'Rate this Appchain'}</span>
              <div className="flex items-center gap-1.5 mt-1.5">
                {[1, 2, 3, 4, 5].map(star => (
                  <button key={star} onClick={() => setUserRating(star === userRating ? 0 : star)} className="cursor-pointer">
                    <svg width="22" height="22" viewBox="0 0 24 24"
                      fill={star <= userRating ? '#f59e0b' : 'none'}
                      stroke={star <= userRating ? '#f59e0b' : 'currentColor'}
                      strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
                      className={star <= userRating ? '' : 'text-[var(--color-text-secondary)] opacity-40 hover:opacity-70'}
                    >
                      <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>
                    </svg>
                  </button>
                ))}
                {userRating > 0 && <span className="text-[11px] text-[var(--color-text-secondary)] ml-2">{ko ? '감사합니다!' : 'Thanks!'}</span>}
              </div>
            </div>

            {/* Comments header */}
            <div className="flex items-center justify-between px-1">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">
                {ko ? `댓글 ${comments.length}개` : `${comments.length} comments`}
              </span>
              <select className="text-[10px] bg-transparent text-[var(--color-text-secondary)] outline-none cursor-pointer">
                <option>{ko ? '최신순' : 'Newest'}</option>
                <option>{ko ? '인기순' : 'Popular'}</option>
              </select>
            </div>

            {/* Comments List */}
            <div className="space-y-2">
              {comments.map(comment => (
                <div key={comment.id} className="bg-[var(--color-bg-sidebar)] rounded-xl border border-[var(--color-border)] overflow-hidden">
                  <div className="p-3">
                    <div className="flex items-start gap-2.5">
                      <div className="w-7 h-7 rounded-full bg-[var(--color-border)] flex items-center justify-center text-[10px] font-bold flex-shrink-0">{comment.avatar}</div>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="text-[12px] font-semibold">{comment.author}</span>
                          <span className="text-[9px] text-[var(--color-text-secondary)]">{comment.time}</span>
                        </div>
                        <p className="text-[12px] mt-1 leading-relaxed">{comment.text}</p>
                        <div className="flex items-center gap-3 mt-2">
                          <button onClick={() => setComments(comments.map(c => c.id === comment.id ? { ...c, liked: !c.liked, likes: c.liked ? c.likes - 1 : c.likes + 1 } : c))}
                            className={`flex items-center gap-1 text-[10px] cursor-pointer transition-colors ${comment.liked ? 'text-[#3b82f6]' : 'text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'}`}>
                            <svg width="12" height="12" viewBox="0 0 24 24" fill={comment.liked ? '#3b82f6' : 'none'} stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                              <path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"/>
                            </svg>
                            {comment.likes > 0 && comment.likes}
                          </button>
                          <button onClick={() => setReplyingTo(replyingTo === comment.id ? null : comment.id)}
                            className="text-[10px] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer">{ko ? '답글' : 'Reply'}</button>
                        </div>
                      </div>
                    </div>
                  </div>
                  {comment.replies.length > 0 && (
                    <div className="border-t border-[var(--color-border)] bg-[var(--color-bg-main)]">
                      {comment.replies.map(reply => (
                        <div key={reply.id} className="px-3 py-2.5 ml-6 border-b border-[var(--color-border)] last:border-b-0">
                          <div className="flex items-start gap-2">
                            <div className="w-5 h-5 rounded-full bg-[var(--color-border)] flex items-center justify-center text-[8px] font-bold flex-shrink-0">{reply.avatar}</div>
                            <div className="flex-1 min-w-0">
                              <div className="flex items-center gap-2">
                                <span className="text-[11px] font-semibold">{reply.author}</span>
                                <span className="text-[9px] text-[var(--color-text-secondary)]">{reply.time}</span>
                              </div>
                              <p className="text-[11px] mt-0.5 leading-relaxed">{reply.text}</p>
                              <button onClick={() => setComments(comments.map(c => c.id === comment.id ? { ...c, replies: c.replies.map(r => r.id === reply.id ? { ...r, liked: !r.liked, likes: r.liked ? r.likes - 1 : r.likes + 1 } : r) } : c))}
                                className={`flex items-center gap-1 text-[9px] mt-1 cursor-pointer transition-colors ${reply.liked ? 'text-[#3b82f6]' : 'text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'}`}>
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
                  {replyingTo === comment.id && (
                    <div className="border-t border-[var(--color-border)] p-3 bg-[var(--color-bg-main)]">
                      <div className="flex items-start gap-2 ml-6">
                        <div className="w-5 h-5 rounded-full bg-[var(--color-accent)] flex items-center justify-center text-[8px] font-bold text-[var(--color-accent-text)] flex-shrink-0 mt-0.5">Me</div>
                        <div className="flex-1">
                          {(() => {
                            const submitReply = () => {
                              if (!replyInput.trim()) return
                              const newReply: Comment = { id: `reply-${Date.now()}`, author: 'me', avatar: 'Me', text: replyInput.trim(), time: ko ? '방금' : 'Just now', likes: 0, liked: false, replies: [] }
                              setComments(comments.map(c => c.id === comment.id ? { ...c, replies: [...c.replies, newReply] } : c))
                              setReplyInput(''); setReplyingTo(null)
                            }
                            return (<>
                          <input type="text" value={replyInput} onChange={e => setReplyInput(e.target.value)}
                            placeholder={ko ? '답글을 입력하세요...' : 'Write a reply...'}
                            onKeyDown={e => { if (e.key === 'Enter') submitReply() }}
                            className="w-full bg-[var(--color-bg-sidebar)] rounded-lg px-2.5 py-1.5 text-[11px] outline-none border border-[var(--color-border)]" autoFocus />
                          <div className="flex items-center gap-2 mt-1.5">
                            <button onClick={submitReply} disabled={!replyInput.trim()} className="bg-[#3b82f6] text-white text-[10px] font-medium px-3 py-1 rounded-lg hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-40">{ko ? '등록' : 'Post'}</button>
                            <button onClick={() => { setReplyingTo(null); setReplyInput('') }} className="text-[10px] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer">{ko ? '취소' : 'Cancel'}</button>
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

            {/* Write comment - bottom */}
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
              <div className="flex items-start gap-2.5">
                <div className="w-7 h-7 rounded-full bg-[var(--color-accent)] flex items-center justify-center text-[10px] font-bold text-[var(--color-accent-text)] flex-shrink-0 mt-0.5">Me</div>
                <div className="flex-1">
                  <textarea value={commentInput} onChange={e => setCommentInput(e.target.value)}
                    placeholder={ko ? '질문이나 의견을 남겨보세요...' : 'Ask a question or leave a comment...'} rows={2}
                    className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-[12px] outline-none border border-[var(--color-border)] resize-none" />
                  <div className="flex justify-end mt-1.5">
                    <button onClick={() => {
                      if (!commentInput.trim()) return
                      const newComment: Comment = { id: `new-${Date.now()}`, author: 'me', avatar: 'Me', text: commentInput.trim(), time: ko ? '방금' : 'Just now', likes: 0, liked: false, replies: [] }
                      setComments([newComment, ...comments]); setCommentInput('')
                    }} disabled={!commentInput.trim()}
                      className="bg-[#3b82f6] text-white text-[11px] font-medium px-4 py-1.5 rounded-lg hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-40"
                    >
                      {ko ? '등록' : 'Post'}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </>)}

        </div>
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      {/* Header */}
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
        <div className="flex items-center justify-between">
          <h1 className="text-base font-semibold">{t('openl2.title', lang)}</h1>
          <button className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] text-xs font-medium px-3 py-1.5 rounded-lg transition-colors cursor-pointer text-[var(--color-accent-text)]">
            + {t('openl2.registerMyL2', lang)}
          </button>
        </div>
        <div className="flex items-center gap-2 mt-2">
          <div className="relative flex-1">
            <input
              type="text"
              value={searchQuery}
              onChange={e => setSearchQuery(e.target.value)}
              placeholder={t('openl2.searchPlaceholder', lang)}
              className="w-full bg-[var(--color-bg-sidebar)] rounded-lg px-3 py-2 text-[13px] outline-none placeholder-[var(--color-text-secondary)] border border-[var(--color-border)] pl-8"
            />
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--color-text-secondary)]">
              <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
            </svg>
          </div>
          <select
            value={selectedTag}
            onChange={e => setSelectedTag(e.target.value)}
            className="bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] rounded-lg px-3 py-2 text-[13px] outline-none cursor-pointer"
          >
            {popularTags.map(tag => (
              <option key={tag} value={tag}>
                {tag === '전체' ? t('openl2.all', lang) : `#${tag}`}
              </option>
            ))}
          </select>
        </div>
      </div>

      {/* All / Favorites / Bookmarks tab */}
      <div className="flex border-b border-[var(--color-border)] px-1">
        {([
          { id: 'all' as const, label: ko ? '전체' : 'All' },
          { id: 'favorites' as const, label: ko ? `관심 (${favoriteIds.size})` : `Favorites (${favoriteIds.size})`, icon: '♥' },
          { id: 'bookmarks' as const, label: ko ? `북마크 (${bookmarkedIds.size})` : `Bookmarks (${bookmarkedIds.size})` },
        ]).map(tab => (
          <button key={tab.id} onClick={() => setListTab(tab.id)}
            className={`px-3 py-2 text-[12px] transition-colors cursor-pointer border-b-2 ${
              listTab === tab.id
                ? 'border-[var(--color-text-primary)] text-[var(--color-text-primary)] font-medium'
                : 'border-transparent text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'
            }`}
          >{tab.label}</button>
        ))}
      </div>

      {/* Shared wallet address bar for bookmarks tab */}
      {listTab === 'bookmarks' && (
        <div className="px-4 py-2.5 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
          {!walletAddress && !editingWallet ? (
            <button
              onClick={() => { setEditingWallet(true); setWalletInput('') }}
              className="flex items-center gap-1.5 text-[11px] text-[#3b82f6] hover:underline cursor-pointer"
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>
              </svg>
              {ko ? '주소를 등록하면 북마크한 앱체인의 자산을 확인할 수 있습니다' : 'Register your address to view assets across bookmarked appchains'}
            </button>
          ) : editingWallet ? (
            <div className="flex items-center gap-2">
              <input
                type="text"
                value={walletInput}
                onChange={e => setWalletInput(e.target.value)}
                placeholder="0x..."
                onKeyDown={e => {
                  if (e.key === 'Enter' && walletInput.trim()) {
                    setWalletAddress(walletInput.trim()); setEditingWallet(false)
                  }
                }}
                className="flex-1 bg-[var(--color-bg-main)] rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none border border-[var(--color-border)]"
                autoFocus
              />
              <button
                onClick={() => { if (walletInput.trim()) { setWalletAddress(walletInput.trim()); setEditingWallet(false) } }}
                disabled={!walletInput.trim()}
                className="bg-[#3b82f6] text-white text-[10px] font-medium px-3 py-1.5 rounded-lg hover:opacity-80 cursor-pointer disabled:opacity-40"
              >{ko ? '등록' : 'Save'}</button>
              <button
                onClick={() => setEditingWallet(false)}
                className="text-[10px] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer"
              >{ko ? '취소' : 'Cancel'}</button>
            </div>
          ) : (
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-1.5">
                <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="#3b82f6" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>
                </svg>
                <span className="text-[11px] font-mono text-[var(--color-text-secondary)]">
                  {walletAddress.slice(0, 6)}...{walletAddress.slice(-4)}
                </span>
                <span className="text-[9px] text-[var(--color-success)]">● {ko ? '연결됨' : 'Connected'}</span>
              </div>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => { setEditingWallet(true); setWalletInput(walletAddress) }}
                  className="text-[9px] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer"
                >{ko ? '변경' : 'Change'}</button>
                <button
                  onClick={() => setWalletAddress('')}
                  className="text-[9px] text-[var(--color-text-secondary)] hover:text-[#ef4444] cursor-pointer"
                >{ko ? '해제' : 'Disconnect'}</button>
              </div>
            </div>
          )}
        </div>
      )}

      {/* L2 List */}
      <div className="flex-1 overflow-y-auto">
        {filtered.length === 0 ? (
          <div className="flex items-center justify-center h-full text-[var(--color-text-secondary)] text-[13px]">
            {t('openl2.noResults', lang)}
          </div>
        ) : (
          filtered.map(l2 => (
            <div key={l2.id} className="border-b border-[var(--color-border)]">
              <div className="w-full px-4 py-3 flex items-center gap-3 hover:bg-[var(--color-bg-sidebar)] transition-colors">
                <div onClick={() => setSelectedL2(l2)} className="flex items-center gap-3 flex-1 min-w-0 cursor-pointer">
                  <div className="w-10 h-10 rounded-xl bg-[var(--color-bg-sidebar)] flex items-center justify-center text-xl flex-shrink-0">
                    {l2.icon}
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-1.5">
                      <span className="text-sm font-medium truncate">{l2.name}</span>
                      <span className="text-[11px] text-[var(--color-text-secondary)]">{l2.members.toLocaleString()}</span>
                      <span className={`w-2 h-2 rounded-full flex-shrink-0 ${l2.status === 'online' ? 'bg-[var(--color-success)]' : 'bg-[var(--color-text-secondary)]'}`} />
                    </div>
                    <div className="text-[11px] text-[var(--color-text-secondary)] truncate mt-0.5">
                      {l2.description}
                    </div>
                    <div className="flex gap-1 mt-1">
                      {l2.hashtags.slice(0, 3).map(tag => (
                        <span key={tag} className="text-[10px] text-[var(--color-tag-text)] bg-[var(--color-tag-bg)] px-1.5 py-0.5 rounded">
                          #{tag}
                        </span>
                      ))}
                      {l2.hashtags.length > 3 && (
                        <span className="text-[10px] text-[var(--color-text-secondary)]">+{l2.hashtags.length - 3}</span>
                      )}
                    </div>
                  </div>
                </div>
                <div className="flex flex-col items-end gap-1 flex-shrink-0">
                  <div className="text-[11px] font-semibold font-mono">{l2.tvlUsd}</div>
                  <div className="text-[9px] text-[var(--color-text-secondary)]">TVL</div>
                  <div className="flex items-center gap-1.5 mt-0.5">
                    <button
                      onClick={(e) => { e.stopPropagation(); toggleFavorite(l2.id) }}
                      className="cursor-pointer"
                      title={ko ? '관심' : 'Favorite'}
                    >
                      <svg width="13" height="13" viewBox="0 0 24 24"
                        fill={favoriteIds.has(l2.id) ? '#ef4444' : 'none'}
                        stroke={favoriteIds.has(l2.id) ? '#ef4444' : 'currentColor'}
                        strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
                        className={favoriteIds.has(l2.id) ? '' : 'text-[var(--color-text-secondary)] hover:text-[#ef4444]'}
                      >
                        <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/>
                      </svg>
                    </button>
                    <button
                      onClick={(e) => { e.stopPropagation(); toggleBookmark(l2.id) }}
                      className="cursor-pointer"
                      title={ko ? '북마크' : 'Bookmark'}
                    >
                      <svg width="13" height="13" viewBox="0 0 24 24"
                        fill={bookmarkedIds.has(l2.id) ? '#3b82f6' : 'none'}
                        stroke={bookmarkedIds.has(l2.id) ? '#3b82f6' : 'currentColor'}
                        strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
                        className={bookmarkedIds.has(l2.id) ? '' : 'text-[var(--color-text-secondary)] hover:text-[#3b82f6]'}
                      >
                        <path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"/>
                      </svg>
                    </button>
                  </div>
                </div>
              </div>

              {/* Asset balances shown when address is registered in bookmarks tab */}
              {listTab === 'bookmarks' && walletAddress && (
                <div className="px-4 pb-3 bg-[var(--color-bg-sidebar)]">
                  <div className="bg-[var(--color-bg-main)] rounded-lg border border-[var(--color-border)] divide-y divide-[var(--color-border)]">
                    {(MOCK_ASSETS[l2.id] || DEFAULT_ASSETS).map(asset => (
                      <div key={asset.symbol} className="flex items-center justify-between px-3 py-1.5">
                        <div className="flex items-center gap-2">
                          <div className="w-5 h-5 rounded-full bg-[var(--color-border)] flex items-center justify-center text-[8px] font-bold">{asset.symbol[0]}</div>
                          <div>
                            <div className="text-[11px] font-medium">{asset.symbol}</div>
                            <div className="text-[9px] text-[var(--color-text-secondary)]">{asset.name}</div>
                          </div>
                        </div>
                        <div className="text-right">
                          <div className="text-[11px] font-mono font-medium">{asset.balance}</div>
                          <div className="text-[9px] text-[var(--color-text-secondary)]">{asset.usd}</div>
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          ))
        )}
      </div>
    </div>
  )
}
