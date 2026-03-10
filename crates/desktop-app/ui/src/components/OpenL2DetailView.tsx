import { useState } from 'react'
import { t, type Lang } from '../i18n'
import type { Comment } from '../types/comments'
import type { L2Service } from './OpenL2View'
import CommentSection from './CommentSection'

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

interface OpenL2DetailViewProps {
  l2: L2Service
  onBack: () => void
  ko: boolean
  lang: Lang
}

export default function OpenL2DetailView({ l2, onBack, ko, lang }: OpenL2DetailViewProps) {
  const [openDetailTab, setOpenDetailTab] = useState<'overview' | 'community'>('overview')
  const [l2Liked, setL2Liked] = useState(false)
  const [l2LikeCount, setL2LikeCount] = useState(248)
  const [userRating, setUserRating] = useState(0)
  const [comments, setComments] = useState<Comment[]>(() => [...OPEN_L2_MOCK_COMMENTS])

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
          onClick={onBack}
          className="text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer mb-2"
        >
          ← {t('openl2.back', lang)}
        </button>
        {/* Hero */}
        <div className="flex items-start gap-3">
          <div className="w-12 h-12 rounded-2xl bg-[var(--color-bg-main)] flex items-center justify-center text-2xl border border-[var(--color-border)] flex-shrink-0">
            {l2.icon}
          </div>
          <div className="flex-1 min-w-0">
            <h1 className="text-[14px] font-bold">{l2.name}</h1>
            <div className="text-[10px] text-[var(--color-text-secondary)]">by {l2.operator}</div>
            <div className="flex items-center gap-3 mt-0.5">
              <div className="flex items-center gap-1">
                <span className={`w-2 h-2 rounded-full ${l2.status === 'online' ? 'bg-[var(--color-success)]' : 'bg-[var(--color-text-secondary)]'}`} />
                <span className="text-[10px] text-[var(--color-text-secondary)]">
                  {l2.status === 'online' ? t('openl2.online', lang) : t('openl2.offline', lang)}
                </span>
              </div>
              <span className="text-[10px] text-[var(--color-text-secondary)]">{l2.members.toLocaleString()} {t('openl2.users', lang)}</span>
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
          onClick={() => window.open(`http://dashboard.example.com/${l2.chainId}`, '_blank')}
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

        {/* Overview */}
        {openDetailTab === 'overview' && (<>
          {/* Hashtags */}
          <div className="flex flex-wrap gap-1.5">
            {l2.hashtags.map(tag => (
              <span key={tag} className="text-[11px] bg-[var(--color-tag-bg)] px-2 py-0.5 rounded text-[var(--color-tag-text)]">#{tag}</span>
            ))}
          </div>

          {/* Description */}
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">{ko ? '소개' : 'About'}</span>
            <p className="text-[12px] mt-1.5 leading-relaxed">{l2.description}</p>
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
              <div className="text-[14px] font-bold font-mono mt-0.5">{l2.tvlUsd}</div>
              <div className="text-[9px] text-[var(--color-text-secondary)]">{l2.tvl}</div>
            </div>
            <div className="bg-[var(--color-bg-sidebar)] rounded-lg p-2.5 border border-[var(--color-border)]">
              <div className="text-[10px] text-[var(--color-text-secondary)]">{ko ? '사용자' : 'Users'}</div>
              <div className="text-[14px] font-bold font-mono mt-0.5">{l2.members.toLocaleString()}</div>
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
                <span className="font-mono">{l2.chainId}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[var(--color-text-secondary)]">RPC</span>
                <code className="font-mono text-[10px] text-[#3b82f6]">{l2.rpcUrl}</code>
              </div>
              <div className="flex justify-between">
                <span className="text-[var(--color-text-secondary)]">{ko ? '네이티브 토큰' : 'Native Token'}</span>
                <span>TON</span>
              </div>
            </div>
          </div>
        </>)}

        {/* Community */}
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

          <CommentSection comments={comments} onCommentsChange={setComments} ko={ko} />
        </>)}

      </div>
    </div>
  )
}
