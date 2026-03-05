import { useState } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'

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
}

const sampleL2s: L2Service[] = [
  {
    id: '1', name: 'Tokamak DEX', icon: '🔄', description: 'Decentralized exchange with AMM pools on Tokamak L2',
    operator: 'tokamak-team', members: 1234, hashtags: ['DeFi', 'DEX', 'AMM', 'Swap'],
    status: 'online', chainId: 17001, rpcUrl: 'https://rpc.dex.tokamak.network', lastActive: '2m ago',
  },
  {
    id: '2', name: 'NFT Marketplace L2', icon: '🎨', description: 'NFT minting and trading platform with low gas fees',
    operator: 'nft-builder', members: 856, hashtags: ['NFT', 'Art', 'Marketplace', 'Minting'],
    status: 'online', chainId: 17002, rpcUrl: 'https://rpc.nft.example.com', lastActive: '5m ago',
  },
  {
    id: '3', name: 'GameFi World', icon: '🎮', description: 'Gaming L2 with on-chain game state and item trading',
    operator: 'gamefi-studio', members: 2341, hashtags: ['Gaming', 'GameFi', 'P2E', 'Items'],
    status: 'online', chainId: 17003, rpcUrl: 'https://rpc.gamefi.example.com', lastActive: '1m ago',
  },
  {
    id: '4', name: 'Social Protocol', icon: '💬', description: 'Decentralized social media protocol on L2',
    operator: 'social-dao', members: 567, hashtags: ['Social', 'DAO', 'Messaging', 'Identity'],
    status: 'offline', chainId: 17004, rpcUrl: 'https://rpc.social.example.com', lastActive: '2h ago',
  },
  {
    id: '5', name: 'Bridge Hub', icon: '🌉', description: 'Cross-chain bridge aggregator connecting multiple L2s',
    operator: 'bridge-labs', members: 3120, hashtags: ['Bridge', 'CrossChain', 'Interop'],
    status: 'online', chainId: 17005, rpcUrl: 'https://rpc.bridge.example.com', lastActive: '30s ago',
  },
  {
    id: '6', name: 'DeFi Lending', icon: '🏦', description: 'Lending and borrowing protocol with TON collateral',
    operator: 'lending-protocol', members: 912, hashtags: ['DeFi', 'Lending', 'Borrow', 'TON'],
    status: 'online', chainId: 17006, rpcUrl: 'https://rpc.lending.example.com', lastActive: '10m ago',
  },
  {
    id: '7', name: 'ZK Privacy L2', icon: '🔒', description: 'Privacy-preserving transactions using ZK proofs',
    operator: 'privacy-labs', members: 445, hashtags: ['Privacy', 'ZK', 'Confidential', 'Anonymous'],
    status: 'offline', chainId: 17007, rpcUrl: 'https://rpc.privacy.example.com', lastActive: '1d ago',
  },
  {
    id: '8', name: 'AI Agent Hub', icon: '🤖', description: 'L2 specialized for AI agent operations and micro-transactions',
    operator: 'ai-hub-team', members: 1678, hashtags: ['AI', 'Agent', 'Automation', 'MicroTx'],
    status: 'online', chainId: 17008, rpcUrl: 'https://rpc.ai-hub.example.com', lastActive: '15s ago',
  },
]

const popularTags = ['전체', 'DeFi', 'NFT', 'Gaming', 'Bridge', 'Social', 'AI', 'Privacy', 'DAO']

export default function OpenL2View() {
  const { lang } = useLang()
  const [searchQuery, setSearchQuery] = useState('')
  const [selectedTag, setSelectedTag] = useState('전체')
  const [selectedL2, setSelectedL2] = useState<L2Service | null>(null)

  const filtered = sampleL2s.filter(l2 => {
    const matchesSearch = searchQuery === '' ||
      l2.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
      l2.description.toLowerCase().includes(searchQuery.toLowerCase()) ||
      l2.hashtags.some(tag => tag.toLowerCase().includes(searchQuery.toLowerCase()))
    const matchesTag = selectedTag === '전체' || l2.hashtags.includes(selectedTag)
    return matchesSearch && matchesTag
  })

  if (selectedL2) {
    return (
      <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
        {/* Detail Header */}
        <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
          <button
            onClick={() => setSelectedL2(null)}
            className="text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] cursor-pointer"
          >
            ← {t('openl2.back', lang)}
          </button>
        </div>

        {/* Detail Content */}
        <div className="flex-1 overflow-y-auto p-4 space-y-3">
          <div className="flex items-center gap-3">
            <div className="w-12 h-12 rounded-xl bg-[var(--color-bg-sidebar)] flex items-center justify-center text-2xl border border-[var(--color-border)]">
              {selectedL2.icon}
            </div>
            <div>
              <h1 className="text-base font-semibold">{selectedL2.name}</h1>
              <div className="text-[11px] text-[var(--color-text-secondary)]">by {selectedL2.operator}</div>
              <div className="flex items-center gap-2 mt-0.5">
                <span className={`w-2 h-2 rounded-full ${selectedL2.status === 'online' ? 'bg-[var(--color-success)]' : 'bg-[var(--color-text-secondary)]'}`} />
                <span className="text-[11px] text-[var(--color-text-secondary)]">
                  {selectedL2.status === 'online' ? t('openl2.online', lang) : t('openl2.offline', lang)} · {selectedL2.members.toLocaleString()} {t('openl2.users', lang)}
                </span>
              </div>
            </div>
          </div>

          <p className="text-[13px] text-[var(--color-text-secondary)]">{selectedL2.description}</p>

          <div className="flex flex-wrap gap-1.5">
            {selectedL2.hashtags.map(tag => (
              <span key={tag} className="text-[11px] bg-[var(--color-tag-bg)] px-2 py-0.5 rounded text-[var(--color-tag-text)]">
                #{tag}
              </span>
            ))}
          </div>

          {/* Connection Info */}
          <section className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 space-y-2 border border-[var(--color-border)]">
            <h2 className="text-[13px] font-medium">{t('openl2.connectionInfo', lang)}</h2>
            <div className="space-y-1.5 text-[12px]">
              <div className="flex justify-between">
                <span className="text-[var(--color-text-secondary)]">Chain ID</span>
                <span className="font-mono">{selectedL2.chainId}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[var(--color-text-secondary)]">RPC URL</span>
                <span className="font-mono text-[11px]">{selectedL2.rpcUrl}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[var(--color-text-secondary)]">{t('openl2.lastActive', lang)}</span>
                <span>{selectedL2.lastActive}</span>
              </div>
            </div>
          </section>

          {/* Actions */}
          <div className="grid grid-cols-2 gap-2">
            <button className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] rounded-xl py-2.5 text-[13px] font-medium transition-colors cursor-pointer text-[var(--color-accent-text)]">
              {t('openl2.connect', lang)}
            </button>
            <button className="bg-[var(--color-bg-sidebar)] hover:bg-[var(--color-border)] rounded-xl py-2.5 text-[13px] font-medium transition-colors cursor-pointer border border-[var(--color-border)]">
              {t('openl2.addDashboard', lang)}
            </button>
          </div>

          {/* AI Guide */}
          <section className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 space-y-2 border border-[var(--color-border)]">
            <h2 className="text-[13px] font-medium">{t('openl2.aiGuide', lang)}</h2>
            <p className="text-[11px] text-[var(--color-text-secondary)]">
              {t('openl2.aiGuideDesc', lang)}
            </p>
            <button className="bg-[var(--color-bg-main)] hover:bg-[var(--color-border)] rounded-lg px-3 py-1.5 text-[12px] transition-colors cursor-pointer border border-[var(--color-border)]">
              {t('openl2.askAi', lang)}
            </button>
          </section>
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

      {/* L2 List */}
      <div className="flex-1 overflow-y-auto">
        {filtered.length === 0 ? (
          <div className="flex items-center justify-center h-full text-[var(--color-text-secondary)] text-[13px]">
            {t('openl2.noResults', lang)}
          </div>
        ) : (
          filtered.map(l2 => (
            <button
              key={l2.id}
              onClick={() => setSelectedL2(l2)}
              className="w-full px-4 py-3 flex items-center gap-3 hover:bg-[var(--color-bg-sidebar)] transition-colors cursor-pointer border-b border-[var(--color-border)] text-left"
            >
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
              <div className="text-[11px] text-[var(--color-text-secondary)] flex-shrink-0">
                {l2.lastActive}
              </div>
            </button>
          ))
        )}
      </div>
    </div>
  )
}
