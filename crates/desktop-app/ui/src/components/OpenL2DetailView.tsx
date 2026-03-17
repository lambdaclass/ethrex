import { useState, useEffect } from 'react'
import { t, type Lang } from '../i18n'
import { platformAPI, type StoreAppchain } from '../api/platform'
import { ipfsToHttp } from '../api/ipfs'

interface OpenL2DetailViewProps {
  appchain: StoreAppchain
  onBack: () => void
  ko: boolean
  lang: Lang
}

export default function OpenL2DetailView({ appchain, onBack, ko, lang }: OpenL2DetailViewProps) {
  const [openDetailTab, setOpenDetailTab] = useState<'overview' | 'community'>('overview')
  const [l2Liked, setL2Liked] = useState(false)
  const [l2LikeCount, setL2LikeCount] = useState(0)
  const [userRating, setUserRating] = useState(0)
  const [onlineStatus, setOnlineStatus] = useState<'checking' | 'online' | 'offline'>('checking')
  const [blockNumber, setBlockNumber] = useState<string | null>(null)

  // Community data from API
  const [reviews, setReviews] = useState<Array<{ id: string; wallet_address: string; rating: number; content: string; created_at: number }>>([])
  const [comments, setComments] = useState<Array<{ id: string; wallet_address: string; content: string; parent_id: string | null; created_at: number }>>([])
  const [announcements, setAnnouncements] = useState<Array<{ id: string; title: string; content: string; pinned: number; created_at: number }>>([])
  const [communityLoading, setCommunityLoading] = useState(false)

  const chainId = appchain.chain_id || appchain.l2_chain_id
  const avgRating = appchain.avg_rating
  const ratingCount = appchain.review_count || 0

  // Check RPC status via proxy
  useEffect(() => {
    if (!appchain.rpc_url) {
      setOnlineStatus('offline')
      return
    }
    platformAPI.rpcProxy(appchain.id, 'eth_blockNumber')
      .then(data => {
        if (data?.result) {
          setOnlineStatus('online')
          setBlockNumber(String(parseInt(data.result, 16)))
        } else {
          setOnlineStatus('offline')
        }
      })
      .catch(() => setOnlineStatus('offline'))
  }, [appchain.id, appchain.rpc_url])

  // Load community data when tab changes
  useEffect(() => {
    if (openDetailTab !== 'community') return
    setCommunityLoading(true)
    Promise.all([
      platformAPI.getAppchainReviews(appchain.id).catch(() => ({ reviews: [], reactionCounts: {}, userReactions: [] })),
      platformAPI.getAppchainComments(appchain.id).catch(() => ({ comments: [], reactionCounts: {}, userReactions: [] })),
      platformAPI.getAppchainAnnouncements(appchain.id).catch(() => []),
    ]).then(([reviewData, commentData, announcementData]) => {
      setReviews(reviewData.reviews)
      setComments(commentData.comments)
      setAnnouncements(announcementData)
    }).finally(() => setCommunityLoading(false))
  }, [openDetailTab, appchain.id])

  const detailTabs: { id: 'overview' | 'community'; label: string }[] = [
    { id: 'overview', label: ko ? '개요' : 'Overview' },
    { id: 'community', label: ko ? `커뮤니티 (${ratingCount + (appchain.comment_count || 0)})` : `Community (${ratingCount + (appchain.comment_count || 0)})` },
  ]

  const socialLinks = appchain.social_links || {}

  const safeOpen = (url: string) => {
    try {
      const parsed = new URL(url)
      if (parsed.protocol === 'https:' || parsed.protocol === 'http:') {
        window.open(url, '_blank', 'noopener,noreferrer')
      }
    } catch { /* invalid URL, ignore */ }
  }

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
          <div className="w-12 h-12 rounded-2xl bg-[var(--color-bg-main)] flex items-center justify-center text-2xl border border-[var(--color-border)] flex-shrink-0 overflow-hidden">
            {appchain.screenshots && appchain.screenshots.length > 0 ? (
              <img src={ipfsToHttp(appchain.screenshots[0])} alt="" className="w-full h-full object-cover" />
            ) : (
              <span className="text-[18px] font-bold text-[var(--color-text-secondary)]">{appchain.name.charAt(0).toUpperCase()}</span>
            )}
          </div>
          <div className="flex-1 min-w-0">
            <h1 className="text-[14px] font-bold">{appchain.name}</h1>
            <div className="text-[10px] text-[var(--color-text-secondary)]">
              {appchain.operator_name ? `by ${appchain.operator_name}` : appchain.owner_name ? `by ${appchain.owner_name}` : ''}
              {appchain.stack_type && <span className="ml-1.5 text-[var(--color-tag-text)] bg-[var(--color-tag-bg)] px-1 py-0.5 rounded">{appchain.stack_type}</span>}
            </div>
            <div className="flex items-center gap-3 mt-0.5">
              <div className="flex items-center gap-1">
                <span className={`w-2 h-2 rounded-full ${
                  onlineStatus === 'checking' ? 'bg-yellow-400 animate-pulse' :
                  onlineStatus === 'online' ? 'bg-[var(--color-success)]' : 'bg-[var(--color-text-secondary)]'
                }`} />
                <span className="text-[10px] text-[var(--color-text-secondary)]">
                  {onlineStatus === 'checking' ? '...' : onlineStatus === 'online' ? t('openl2.online', lang) : t('openl2.offline', lang)}
                </span>
              </div>
              {blockNumber && <span className="text-[10px] text-[var(--color-text-secondary)]">Block #{blockNumber}</span>}
              {appchain.network_mode && (
                <span className={`text-[9px] px-1.5 py-0.5 rounded font-medium ${
                  appchain.network_mode === 'mainnet' ? 'bg-green-100 text-green-700' :
                  appchain.network_mode === 'testnet' ? 'bg-blue-100 text-blue-700' :
                  'bg-gray-100 text-gray-600'
                }`}>{appchain.network_mode}</span>
              )}
              {/* Rating inline */}
              {avgRating != null && (
                <div className="flex items-center gap-0.5">
                  <svg width="10" height="10" viewBox="0 0 24 24" fill="#f59e0b" stroke="#f59e0b" strokeWidth="2"><polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/></svg>
                  <span className="text-[10px] font-medium">{avgRating.toFixed(1)}</span>
                  <span className="text-[9px] text-[var(--color-text-secondary)]">({ratingCount})</span>
                </div>
              )}
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
        {appchain.dashboard_url && (
          <>
            <div className="flex-1" />
            <button
              onClick={() => safeOpen(appchain.dashboard_url!)}
              className="flex items-center gap-1 text-[10px] text-[#3b82f6] hover:underline cursor-pointer px-2 py-2"
            >
              {ko ? '대시보드' : 'Dashboard'}
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>
              </svg>
            </button>
          </>
        )}
      </div>

      {/* Tab Content */}
      <div className="flex-1 overflow-y-auto p-3 space-y-3">

        {/* Overview */}
        {openDetailTab === 'overview' && (<>
          {/* Hashtags */}
          {appchain.hashtags && appchain.hashtags.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {appchain.hashtags.map(tag => (
                <span key={tag} className="text-[11px] bg-[var(--color-tag-bg)] px-2 py-0.5 rounded text-[var(--color-tag-text)]">#{tag}</span>
              ))}
            </div>
          )}

          {/* Description */}
          {appchain.description && (
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">{ko ? '소개' : 'About'}</span>
              <p className="text-[12px] mt-1.5 leading-relaxed whitespace-pre-wrap">{appchain.description}</p>
            </div>
          )}

          {/* Screenshots */}
          {appchain.screenshots && appchain.screenshots.length > 0 ? (
            <div>
              <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)] px-1">{ko ? '스크린샷' : 'Screenshots'}</span>
              <div className="flex gap-2 mt-1.5 overflow-x-auto pb-1">
                {appchain.screenshots.map((uri, i) => (
                  <img
                    key={i}
                    src={ipfsToHttp(uri)}
                    alt={`Screenshot ${i + 1}`}
                    className="flex-shrink-0 w-36 h-24 rounded-xl border border-[var(--color-border)] object-cover cursor-pointer hover:opacity-80 transition-opacity"
                    onClick={() => safeOpen(ipfsToHttp(uri))}
                  />
                ))}
              </div>
            </div>
          ) : (
            <div className="text-[11px] text-[var(--color-text-secondary)] text-center py-4">
              {t('openl2.noScreenshots', lang)}
            </div>
          )}

          {/* Quick Stats */}
          <div className="grid grid-cols-3 gap-2">
            <div className="bg-[var(--color-bg-sidebar)] rounded-lg p-2.5 border border-[var(--color-border)]">
              <div className="text-[10px] text-[var(--color-text-secondary)]">{t('openl2.reviews', lang)}</div>
              <div className="text-[14px] font-bold font-mono mt-0.5">{ratingCount}</div>
            </div>
            <div className="bg-[var(--color-bg-sidebar)] rounded-lg p-2.5 border border-[var(--color-border)]">
              <div className="text-[10px] text-[var(--color-text-secondary)]">{t('openl2.comments', lang)}</div>
              <div className="text-[14px] font-bold font-mono mt-0.5">{appchain.comment_count || 0}</div>
            </div>
            <div className="bg-[var(--color-bg-sidebar)] rounded-lg p-2.5 border border-[var(--color-border)]">
              <div className="text-[10px] text-[var(--color-text-secondary)]">{t('openl2.rating', lang)}</div>
              <div className="text-[14px] font-bold font-mono mt-0.5">{avgRating != null ? avgRating.toFixed(1) : '-'}</div>
            </div>
          </div>

          {/* Connection Info */}
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">{t('openl2.connectionInfo', lang)}</span>
            <div className="mt-1.5 space-y-1 text-[11px]">
              {chainId && (
                <div className="flex justify-between">
                  <span className="text-[var(--color-text-secondary)]">Chain ID</span>
                  <span className="font-mono">{chainId}</span>
                </div>
              )}
              {appchain.rpc_url && (
                <div className="flex justify-between">
                  <span className="text-[var(--color-text-secondary)]">RPC</span>
                  <code className="font-mono text-[10px] text-[#3b82f6] truncate ml-2 max-w-[200px]">{appchain.rpc_url}</code>
                </div>
              )}
              {appchain.native_token_symbol && (
                <div className="flex justify-between">
                  <span className="text-[var(--color-text-secondary)]">{t('openl2.nativeToken', lang)}</span>
                  <span>{appchain.native_token_symbol}</span>
                </div>
              )}
              {appchain.l1_chain_id && (
                <div className="flex justify-between">
                  <span className="text-[var(--color-text-secondary)]">L1 Chain</span>
                  <span className="font-mono">{appchain.l1_chain_id === 1 ? 'Mainnet' : appchain.l1_chain_id === 11155111 ? 'Sepolia' : appchain.l1_chain_id}</span>
                </div>
              )}
              {appchain.explorer_url && (
                <div className="flex justify-between">
                  <span className="text-[var(--color-text-secondary)]">Explorer</span>
                  <button onClick={() => safeOpen(appchain.explorer_url!)} className="text-[10px] text-[#3b82f6] hover:underline cursor-pointer truncate ml-2 max-w-[200px]">{appchain.explorer_url}</button>
                </div>
              )}
            </div>
          </div>

          {/* Social Links */}
          {Object.keys(socialLinks).length > 0 && (
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">{ko ? '소셜 링크' : 'Social Links'}</span>
              <div className="mt-1.5 space-y-1">
                {Object.entries(socialLinks).filter(([, v]) => v).map(([key, url]) => (
                  <div key={key} className="flex items-center justify-between text-[11px]">
                    <span className="text-[var(--color-text-secondary)] capitalize">{key}</span>
                    <button onClick={() => safeOpen(url)} className="text-[#3b82f6] hover:underline cursor-pointer truncate ml-2 max-w-[200px] text-[10px]">{url}</button>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* L1 Contracts */}
          {appchain.l1_contracts && Object.keys(appchain.l1_contracts).length > 0 && (
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">L1 Contracts</span>
              <div className="mt-1.5 space-y-1">
                {Object.entries(appchain.l1_contracts).map(([key, addr]) => (
                  <div key={key} className="flex items-center justify-between text-[11px]">
                    <span className="text-[var(--color-text-secondary)] capitalize">{key.replace(/([A-Z])/g, ' $1').trim()}</span>
                    <code className="font-mono text-[9px] text-[var(--color-text-secondary)]">{String(addr).slice(0, 8)}...{String(addr).slice(-6)}</code>
                  </div>
                ))}
              </div>
            </div>
          )}
        </>)}

        {/* Community */}
        {openDetailTab === 'community' && (<>
          {communityLoading ? (
            <div className="flex items-center justify-center py-8 text-[var(--color-text-secondary)]">
              <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-current mr-2" />
              <span className="text-[12px]">{ko ? '로딩 중...' : 'Loading...'}</span>
            </div>
          ) : (<>
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
                {userRating > 0 && <span className="text-[11px] text-[var(--color-text-secondary)] ml-2">{ko ? '(지갑 서명 후 제출 — 준비 중)' : '(Wallet signing required — coming soon)'}</span>}
              </div>
            </div>

            {/* Announcements */}
            {announcements.length > 0 && (
              <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
                <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">{t('openl2.announcements', lang)}</span>
                <div className="mt-1.5 space-y-2">
                  {announcements.map(a => (
                    <div key={a.id} className="bg-[var(--color-bg-main)] rounded-lg p-2.5 border border-[var(--color-border)]">
                      <div className="flex items-center gap-1.5">
                        {a.pinned === 1 && <span className="text-[8px] bg-yellow-100 text-yellow-700 px-1 py-0.5 rounded">PIN</span>}
                        <span className="text-[11px] font-medium">{a.title}</span>
                      </div>
                      <p className="text-[10px] text-[var(--color-text-secondary)] mt-1 whitespace-pre-wrap">{a.content}</p>
                      <div className="text-[8px] text-[var(--color-text-secondary)] mt-1">
                        {new Date(a.created_at).toLocaleDateString()}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Reviews */}
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">
                {t('openl2.reviews', lang)} ({reviews.length})
              </span>
              {reviews.length === 0 ? (
                <p className="text-[11px] text-[var(--color-text-secondary)] mt-2">{ko ? '아직 리뷰가 없습니다' : 'No reviews yet'}</p>
              ) : (
                <div className="mt-1.5 space-y-2">
                  {reviews.map(r => (
                    <div key={r.id} className="bg-[var(--color-bg-main)] rounded-lg p-2.5 border border-[var(--color-border)]">
                      <div className="flex items-center gap-1.5">
                        <div className="w-5 h-5 rounded-full bg-[var(--color-border)] flex items-center justify-center text-[8px] font-bold">
                          {r.wallet_address.slice(2, 4).toUpperCase()}
                        </div>
                        <span className="text-[10px] font-mono text-[var(--color-text-secondary)]">
                          {r.wallet_address.slice(0, 6)}...{r.wallet_address.slice(-4)}
                        </span>
                        <div className="flex items-center gap-0.5">
                          {[1, 2, 3, 4, 5].map(s => (
                            <svg key={s} width="8" height="8" viewBox="0 0 24 24" fill={s <= r.rating ? '#f59e0b' : 'none'} stroke="#f59e0b" strokeWidth="2">
                              <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>
                            </svg>
                          ))}
                        </div>
                      </div>
                      <p className="text-[11px] mt-1">{r.content}</p>
                      <div className="text-[8px] text-[var(--color-text-secondary)] mt-1">
                        {new Date(r.created_at).toLocaleDateString()}
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>

            {/* Comments */}
            <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">
                {t('openl2.comments', lang)} ({comments.length})
              </span>
              {comments.length === 0 ? (
                <p className="text-[11px] text-[var(--color-text-secondary)] mt-2">{ko ? '아직 댓글이 없습니다' : 'No comments yet'}</p>
              ) : (
                <div className="mt-1.5 space-y-2">
                  {comments.map(c => (
                    <div key={c.id} className={`bg-[var(--color-bg-main)] rounded-lg p-2.5 border border-[var(--color-border)] ${c.parent_id ? 'ml-4' : ''}`}>
                      <div className="flex items-center gap-1.5">
                        <div className="w-5 h-5 rounded-full bg-[var(--color-border)] flex items-center justify-center text-[8px] font-bold">
                          {c.wallet_address.slice(2, 4).toUpperCase()}
                        </div>
                        <span className="text-[10px] font-mono text-[var(--color-text-secondary)]">
                          {c.wallet_address.slice(0, 6)}...{c.wallet_address.slice(-4)}
                        </span>
                      </div>
                      <p className="text-[11px] mt-1">{c.content}</p>
                      <div className="text-[8px] text-[var(--color-text-secondary)] mt-1">
                        {new Date(c.created_at).toLocaleDateString()}
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </>)}
        </>)}

      </div>
    </div>
  )
}
