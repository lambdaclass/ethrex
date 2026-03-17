import { useState, useEffect, useCallback, useRef } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'
import { platformAPI, type StoreAppchain } from '../api/platform'
import { ipfsToHttp } from '../api/ipfs'
import OpenL2DetailView from './OpenL2DetailView'

const BOOKMARK_STORAGE_KEY = 'tokamak_openl2_bookmarks'

function loadLocalBookmarks(): Set<string> {
  try {
    const raw = localStorage.getItem(BOOKMARK_STORAGE_KEY)
    return raw ? new Set(JSON.parse(raw)) : new Set()
  } catch { return new Set() }
}

function saveLocalBookmarks(ids: Set<string>) {
  localStorage.setItem(BOOKMARK_STORAGE_KEY, JSON.stringify([...ids]))
}

interface RegisterForm {
  name: string
  rpcUrl: string
  chainId: string
  description: string
  l1ChainId: string
}

export default function OpenL2View() {
  const { lang } = useLang()
  const ko = lang === 'ko'
  const [searchQuery, setSearchQuery] = useState('')
  const [selectedTag, setSelectedTag] = useState('전체')
  const [selectedAppchain, setSelectedAppchain] = useState<StoreAppchain | null>(null)
  const [listTab, setListTab] = useState<'all' | 'favorites' | 'bookmarks'>('all')
  const [favoriteIds, setFavoriteIds] = useState<Set<string>>(new Set())
  const [bookmarkedIds, setBookmarkedIds] = useState<Set<string>>(loadLocalBookmarks)
  const [walletAddress, setWalletAddress] = useState('')
  const [walletInput, setWalletInput] = useState('')
  const [editingWallet, setEditingWallet] = useState(false)

  // API state
  const [appchains, setAppchains] = useState<StoreAppchain[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')

  // Registration modal state
  const [showRegister, setShowRegister] = useState(false)
  const [registerForm, setRegisterForm] = useState<RegisterForm>({ name: '', rpcUrl: '', chainId: '', description: '', l1ChainId: '' })
  const [registering, setRegistering] = useState(false)
  const [registerError, setRegisterError] = useState('')
  const [registerSuccess, setRegisterSuccess] = useState(false)

  // Dynamic tags extracted from fetched data
  const [dynamicTags, setDynamicTags] = useState<string[]>([])

  const fetchAppchains = useCallback(async (search?: string) => {
    setLoading(true)
    setError('')
    try {
      const data = await platformAPI.getPublicAppchains({ search: search || undefined, limit: 100 })
      setAppchains(data)
      // Extract unique hashtags
      const tagSet = new Set<string>()
      for (const a of data) {
        if (a.hashtags) a.hashtags.forEach(tag => tagSet.add(tag))
      }
      setDynamicTags([...tagSet].slice(0, 20))
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setLoading(false)
    }
  }, [])

  // Debounce search to avoid firing API call on every keystroke
  const searchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(() => {
    if (searchTimerRef.current) clearTimeout(searchTimerRef.current)
    searchTimerRef.current = setTimeout(() => fetchAppchains(searchQuery), 400)
    return () => { if (searchTimerRef.current) clearTimeout(searchTimerRef.current) }
  }, [searchQuery, fetchAppchains])

  // Load bookmarks from Platform if authenticated
  useEffect(() => {
    if (platformAPI.isAuthenticated()) {
      platformAPI.getUserBookmarks().then(ids => setBookmarkedIds(new Set(ids))).catch(() => {})
    }
  }, [])

  const toggleFavorite = (id: string) => {
    setFavoriteIds(prev => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const toggleBookmark = async (id: string) => {
    if (platformAPI.isAuthenticated()) {
      try {
        const result = await platformAPI.toggleBookmark(id)
        setBookmarkedIds(prev => {
          const next = new Set(prev)
          if (result.bookmarked) next.add(id)
          else next.delete(id)
          return next
        })
        return
      } catch { /* fall through to local */ }
    }
    // Local fallback (unauthenticated or API failure)
    setBookmarkedIds(prev => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      saveLocalBookmarks(next)
      return next
    })
  }

  const handleRegister = async () => {
    if (!registerForm.name.trim() || !registerForm.rpcUrl.trim()) {
      setRegisterError(ko ? '이름과 RPC URL은 필수입니다' : 'Name and RPC URL are required')
      return
    }
    if (/^https?:\/\/(localhost|127\.0\.0\.1|0\.0\.0\.0)(:|\/|$)/.test(registerForm.rpcUrl.trim())) {
      setRegisterError(ko ? '외부에서 접근 가능한 RPC URL이 필요합니다 (localhost 불가)' : 'A publicly accessible RPC URL is required (localhost not allowed)')
      return
    }
    if (!platformAPI.isAuthenticated()) {
      setRegisterError(ko ? 'Platform 로그인이 필요합니다' : 'Platform login required')
      return
    }
    setRegistering(true)
    setRegisterError('')
    try {
      const r = await platformAPI.registerDeployment({
        programId: 'evm-l2',
        name: registerForm.name.trim(),
        chainId: registerForm.chainId ? (parseInt(registerForm.chainId, 10) || undefined) : undefined,
        rpcUrl: registerForm.rpcUrl.trim(),
      })
      const platformId = r.deployment.id
      // Update with additional fields
      await platformAPI.updateDeployment(platformId, {
        description: registerForm.description.trim() || undefined,
        l1_chain_id: registerForm.l1ChainId ? (parseInt(registerForm.l1ChainId, 10) || undefined) : undefined,
        rpc_url: registerForm.rpcUrl.trim(),
      })
      await platformAPI.activateDeployment(platformId)
      // Push metadata to GitHub repo (non-blocking)
      platformAPI.pushMetadata(platformId).catch(err => console.warn('[register] metadata push failed:', err))
      setShowRegister(false)
      setRegisterForm({ name: '', rpcUrl: '', chainId: '', description: '', l1ChainId: '' })
      setRegisterSuccess(true)
      setTimeout(() => setRegisterSuccess(false), 3000)
      fetchAppchains(searchQuery)
    } catch (err) {
      setRegisterError(err instanceof Error ? err.message : String(err))
    } finally {
      setRegistering(false)
    }
  }

  const allTags = ['전체', ...dynamicTags]

  const filtered = appchains.filter(a => {
    if (listTab === 'favorites' && !favoriteIds.has(a.id)) return false
    if (listTab === 'bookmarks' && !bookmarkedIds.has(a.id)) return false
    const matchesTag = selectedTag === '전체' || (a.hashtags && a.hashtags.includes(selectedTag))
    return matchesTag
  })

  if (selectedAppchain) {
    return (
      <OpenL2DetailView
        appchain={selectedAppchain}
        onBack={() => setSelectedAppchain(null)}
        ko={ko}
        lang={lang}
      />
    )
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      {/* Header */}
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
        <div className="flex items-center justify-between">
          <h1 className="text-base font-semibold">{t('openl2.title', lang)}</h1>
          <button
            onClick={() => setShowRegister(true)}
            className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] text-xs font-medium px-3 py-1.5 rounded-lg transition-colors cursor-pointer text-[var(--color-accent-text)]"
          >
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
          {dynamicTags.length > 0 && (
            <select
              value={selectedTag}
              onChange={e => setSelectedTag(e.target.value)}
              className="bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] rounded-lg px-3 py-2 text-[13px] outline-none cursor-pointer"
            >
              {allTags.map(tag => (
                <option key={tag} value={tag}>
                  {tag === '전체' ? t('openl2.all', lang) : `#${tag}`}
                </option>
              ))}
            </select>
          )}
        </div>
      </div>

      {/* Success toast */}
      {registerSuccess && (
        <div className="px-4 py-2 bg-green-50 border-b border-green-200 text-[11px] text-green-700 font-medium flex items-center gap-1.5">
          <span className="text-green-500">✓</span>
          {ko ? '앱체인이 성공적으로 등록되었습니다!' : 'Appchain registered successfully!'}
        </div>
      )}

      {/* All / Favorites / Bookmarks tab */}
      <div className="flex border-b border-[var(--color-border)] px-1">
        {([
          { id: 'all' as const, label: ko ? '전체' : 'All' },
          { id: 'favorites' as const, label: ko ? `관심 (${favoriteIds.size})` : `Favorites (${favoriteIds.size})` },
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
                  if (e.key === 'Enter' && /^0x[0-9a-fA-F]{40}$/.test(walletInput.trim())) {
                    setWalletAddress(walletInput.trim()); setEditingWallet(false)
                  }
                }}
                className="flex-1 bg-[var(--color-bg-main)] rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none border border-[var(--color-border)]"
                autoFocus
              />
              <button
                onClick={() => { if (/^0x[0-9a-fA-F]{40}$/.test(walletInput.trim())) { setWalletAddress(walletInput.trim()); setEditingWallet(false) } }}
                disabled={!/^0x[0-9a-fA-F]{40}$/.test(walletInput.trim())}
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
                <span className="text-[9px] text-[var(--color-success)]">{ko ? '연결됨' : 'Connected'}</span>
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
        {loading ? (
          <div className="flex items-center justify-center h-full text-[var(--color-text-secondary)] text-[13px]">
            <div className="flex items-center gap-2">
              <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-current" />
              {t('openl2.loading', lang)}
            </div>
          </div>
        ) : error ? (
          <div className="flex flex-col items-center justify-center h-full text-[var(--color-text-secondary)] text-[13px] gap-2">
            <div>{t('openl2.loadError', lang)}</div>
            <div className="text-[10px] text-[var(--color-error)]">{error}</div>
            <button onClick={fetchAppchains} className="text-[11px] text-[#3b82f6] hover:underline cursor-pointer">
              {t('openl2.retry', lang)}
            </button>
          </div>
        ) : filtered.length === 0 ? (
          <div className="flex items-center justify-center h-full text-[var(--color-text-secondary)] text-[13px]">
            {t('openl2.noResults', lang)}
          </div>
        ) : (
          filtered.map(a => {
            const chainId = a.chain_id || a.l2_chain_id
            return (
              <div key={a.id} className="border-b border-[var(--color-border)]">
                <div className="w-full px-4 py-3 flex items-center gap-3 hover:bg-[var(--color-bg-sidebar)] transition-colors">
                  <div onClick={() => setSelectedAppchain(a)} className="flex items-center gap-3 flex-1 min-w-0 cursor-pointer">
                    {/* Icon: first screenshot or letter fallback */}
                    <div className="w-10 h-10 rounded-xl bg-[var(--color-bg-sidebar)] flex items-center justify-center text-xl flex-shrink-0 border border-[var(--color-border)] overflow-hidden">
                      {a.screenshots && a.screenshots.length > 0 ? (
                        <img src={ipfsToHttp(a.screenshots[0])} alt="" className="w-full h-full object-cover" />
                      ) : (
                        <span className="text-[14px] font-bold text-[var(--color-text-secondary)]">{a.name.charAt(0).toUpperCase()}</span>
                      )}
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-1.5">
                        <span className="text-sm font-medium truncate">{a.name}</span>
                        {a.native_token_symbol && a.native_token_symbol !== 'ETH' && (
                          <span className="text-[9px] bg-[var(--color-tag-bg)] text-[var(--color-tag-text)] px-1 py-0.5 rounded">{a.native_token_symbol}</span>
                        )}
                        {chainId && <span className="text-[10px] text-[var(--color-text-secondary)]">#{chainId}</span>}
                      </div>
                      <div className="text-[11px] text-[var(--color-text-secondary)] truncate mt-0.5">
                        {a.description || (a.operator_name ? `by ${a.operator_name}` : (a.owner_name ? `by ${a.owner_name}` : ''))}
                      </div>
                      {a.hashtags && a.hashtags.length > 0 && (
                        <div className="flex gap-1 mt-1">
                          {a.hashtags.slice(0, 3).map(tag => (
                            <span key={tag} className="text-[10px] text-[var(--color-tag-text)] bg-[var(--color-tag-bg)] px-1.5 py-0.5 rounded">
                              #{tag}
                            </span>
                          ))}
                          {a.hashtags.length > 3 && (
                            <span className="text-[10px] text-[var(--color-text-secondary)]">+{a.hashtags.length - 3}</span>
                          )}
                        </div>
                      )}
                    </div>
                  </div>
                  <div className="flex flex-col items-end gap-1 flex-shrink-0">
                    {/* Rating */}
                    {a.avg_rating != null && (
                      <div className="flex items-center gap-0.5">
                        <svg width="10" height="10" viewBox="0 0 24 24" fill="#f59e0b" stroke="#f59e0b" strokeWidth="2"><polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/></svg>
                        <span className="text-[10px] font-medium">{a.avg_rating.toFixed(1)}</span>
                        <span className="text-[9px] text-[var(--color-text-secondary)]">({a.review_count})</span>
                      </div>
                    )}
                    {a.network_mode && (
                      <span className={`text-[8px] px-1.5 py-0.5 rounded font-medium ${
                        a.network_mode === 'mainnet' ? 'bg-green-100 text-green-700' :
                        a.network_mode === 'testnet' ? 'bg-blue-100 text-blue-700' :
                        'bg-gray-100 text-gray-600'
                      }`}>{a.network_mode}</span>
                    )}
                    <div className="flex items-center gap-1.5 mt-0.5">
                      <button
                        onClick={(e) => { e.stopPropagation(); toggleFavorite(a.id) }}
                        className="cursor-pointer"
                        title={ko ? '관심' : 'Favorite'}
                      >
                        <svg width="13" height="13" viewBox="0 0 24 24"
                          fill={favoriteIds.has(a.id) ? '#ef4444' : 'none'}
                          stroke={favoriteIds.has(a.id) ? '#ef4444' : 'currentColor'}
                          strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
                          className={favoriteIds.has(a.id) ? '' : 'text-[var(--color-text-secondary)] hover:text-[#ef4444]'}
                        >
                          <path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/>
                        </svg>
                      </button>
                      <button
                        onClick={(e) => { e.stopPropagation(); toggleBookmark(a.id) }}
                        className="cursor-pointer"
                        title={ko ? '북마크' : 'Bookmark'}
                      >
                        <svg width="13" height="13" viewBox="0 0 24 24"
                          fill={bookmarkedIds.has(a.id) ? '#3b82f6' : 'none'}
                          stroke={bookmarkedIds.has(a.id) ? '#3b82f6' : 'currentColor'}
                          strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
                          className={bookmarkedIds.has(a.id) ? '' : 'text-[var(--color-text-secondary)] hover:text-[#3b82f6]'}
                        >
                          <path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"/>
                        </svg>
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            )
          })
        )}
      </div>

      {/* Registration Modal */}
      {showRegister && (
        <div className="fixed inset-0 bg-black/40 flex items-center justify-center z-50" onClick={() => setShowRegister(false)}>
          <div className="bg-[var(--color-bg-main)] rounded-2xl border border-[var(--color-border)] w-[380px] max-h-[80vh] overflow-y-auto shadow-xl" onClick={e => e.stopPropagation()}>
            <div className="px-4 py-3 border-b border-[var(--color-border)]">
              <h2 className="text-[14px] font-semibold">{ko ? '내 앱체인 등록' : 'Register My Appchain'}</h2>
              <p className="text-[10px] text-[var(--color-text-secondary)] mt-0.5">
                {ko ? '이미 운영 중인 앱체인을 오픈 앱체인에 등록합니다' : 'Register an existing appchain to the Open Appchain directory'}
              </p>
            </div>
            <div className="p-4 space-y-3">
              {/* Name */}
              <div>
                <label className="text-[10px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider">{ko ? '앱체인 이름' : 'Appchain Name'} *</label>
                <input
                  type="text"
                  value={registerForm.name}
                  onChange={e => setRegisterForm(f => ({ ...f, name: e.target.value }))}
                  placeholder={ko ? '예: My DEX Chain' : 'e.g. My DEX Chain'}
                  className="w-full mt-1 bg-[var(--color-bg-sidebar)] rounded-lg px-2.5 py-2 text-[12px] outline-none border border-[var(--color-border)]"
                />
              </div>
              {/* RPC URL */}
              <div>
                <label className="text-[10px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider">RPC URL *</label>
                <input
                  type="text"
                  value={registerForm.rpcUrl}
                  onChange={e => setRegisterForm(f => ({ ...f, rpcUrl: e.target.value }))}
                  placeholder="https://rpc.my-chain.example.com"
                  className="w-full mt-1 bg-[var(--color-bg-sidebar)] rounded-lg px-2.5 py-2 text-[12px] font-mono outline-none border border-[var(--color-border)]"
                />
              </div>
              {/* Chain ID */}
              <div>
                <label className="text-[10px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider">Chain ID</label>
                <input
                  type="number"
                  value={registerForm.chainId}
                  onChange={e => setRegisterForm(f => ({ ...f, chainId: e.target.value }))}
                  placeholder="17001"
                  className="w-full mt-1 bg-[var(--color-bg-sidebar)] rounded-lg px-2.5 py-2 text-[12px] font-mono outline-none border border-[var(--color-border)]"
                />
              </div>
              {/* L1 Chain ID */}
              <div>
                <label className="text-[10px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider">L1 Chain ID</label>
                <select
                  value={registerForm.l1ChainId}
                  onChange={e => setRegisterForm(f => ({ ...f, l1ChainId: e.target.value }))}
                  className="w-full mt-1 bg-[var(--color-bg-sidebar)] rounded-lg px-2.5 py-2 text-[12px] outline-none border border-[var(--color-border)] cursor-pointer"
                >
                  <option value="">{ko ? '선택...' : 'Select...'}</option>
                  <option value="1">Ethereum Mainnet (1)</option>
                  <option value="11155111">Sepolia (11155111)</option>
                  <option value="17000">Holesky (17000)</option>
                </select>
              </div>
              {/* Description */}
              <div>
                <label className="text-[10px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider">{ko ? '소개글' : 'Description'}</label>
                <textarea
                  value={registerForm.description}
                  onChange={e => setRegisterForm(f => ({ ...f, description: e.target.value }))}
                  placeholder={ko ? '앱체인을 소개하는 글을 작성하세요' : 'Describe your appchain'}
                  rows={3}
                  className="w-full mt-1 bg-[var(--color-bg-sidebar)] rounded-lg px-2.5 py-2 text-[12px] outline-none border border-[var(--color-border)] resize-none"
                />
              </div>

              {registerError && <p className="text-[10px] text-[var(--color-error)]">{registerError}</p>}

              <div className="flex gap-2 pt-1">
                <button
                  onClick={() => setShowRegister(false)}
                  className="flex-1 py-2 rounded-lg border border-[var(--color-border)] text-[11px] font-medium hover:bg-[var(--color-bg-sidebar)] cursor-pointer transition-colors"
                >{ko ? '취소' : 'Cancel'}</button>
                <button
                  onClick={handleRegister}
                  disabled={registering || !registerForm.name.trim() || !registerForm.rpcUrl.trim()}
                  className="flex-1 py-2 rounded-lg bg-[var(--color-accent)] text-[var(--color-accent-text)] text-[11px] font-medium cursor-pointer disabled:opacity-50 transition-colors"
                >
                  {registering ? (ko ? '등록 중...' : 'Registering...') : (ko ? '등록' : 'Register')}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
