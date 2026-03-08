import { useState } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'
import OpenL2DetailView from './OpenL2DetailView'

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

export interface L2Service {
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

  if (selectedL2) {
    return (
      <OpenL2DetailView
        l2={selectedL2}
        onBack={() => setSelectedL2(null)}
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
