import { useState } from 'react'
import { SectionHeader, StatCard } from './ui-atoms'
import type { L2Config } from './MyL2View'
import type { ChainMetrics } from './L2DetailView'

const AVAILABLE_TAGS = ['defi', 'gaming', 'nft', 'dao', 'social', 'bridge', 'dex', 'lending', 'zk', 'privacy', 'ai', 'rwa', 'staking', 'oracle']

interface Props {
  ko: boolean
  l2: L2Config
  chain: ChainMetrics
  tags: string[]
  setTags: (tags: string[]) => void
  onRefresh?: () => void
}

export default function L2DetailOverviewTab({ ko, l2, chain, tags, setTags, onRefresh }: Props) {
  const [showTagPicker, setShowTagPicker] = useState(false)
  const [draftTags, setDraftTags] = useState<string[]>([])

  return (
    <>
      {/* Open Appchain Status */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <div className="flex items-center gap-2">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--color-text-secondary)]">
            <circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>
          </svg>
          <span className="text-[11px] font-medium">{ko ? '오픈 앱체인' : 'Open Appchain'}</span>
          {l2.isPublic
            ? <span className="text-[9px] px-2 py-0.5 rounded-full bg-[var(--color-success)]/15 text-[var(--color-success)] font-medium">{ko ? '공개 중' : 'Public'}</span>
            : <span className="text-[9px] px-2 py-0.5 rounded-full bg-[var(--color-bg-main)] text-[var(--color-text-secondary)] border border-[var(--color-border)]">{ko ? '비공개' : 'Private'}</span>
          }
        </div>
      </div>

      {/* Chain Status */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <SectionHeader title={ko ? '체인 현황' : 'Chain Status'} />
        <div className="grid grid-cols-2 gap-2 mt-1">
          <StatCard label={ko ? 'L1 블록' : 'L1 Block'} value={chain.l1BlockNumber.toLocaleString()} sub={`Chain ID: ${chain.l1ChainId}${chain.l1ChainId === 11155111 ? ' (Sepolia)' : chain.l1ChainId === 17000 ? ' (Holesky)' : chain.l1ChainId === 1 ? ' (Mainnet)' : ''}`} />
          <StatCard label={ko ? 'L2 블록' : 'L2 Block'} value={chain.l2BlockNumber.toLocaleString()} sub={`Chain ID: ${chain.l2ChainId || l2.chainId}`} />
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
            <span key={tag} className="text-[10px] bg-[#3b82f6] text-white px-2 py-0.5 rounded">#{tag}</span>
          ))}
          {tags.length === 0 && <span className="text-[10px] text-[var(--color-text-secondary)]">{ko ? '태그 없음' : 'No tags'}</span>}
          <button
            onClick={() => { setDraftTags([...tags]); setShowTagPicker(true) }}
            className="text-[10px] text-[var(--color-text-secondary)] hover:text-[#3b82f6] cursor-pointer bg-transparent border-none"
          >+ {ko ? '편집' : 'edit'}</button>
        </div>
        {showTagPicker && (
          <div className="mt-2 pt-2 border-t border-[var(--color-border)]">
            <div className="flex flex-wrap gap-1.5">
              {AVAILABLE_TAGS.map(tag => {
                const selected = draftTags.includes(tag)
                return (
                  <button
                    key={tag}
                    onClick={() => setDraftTags(selected ? draftTags.filter(t => t !== tag) : [...draftTags, tag])}
                    className={`text-[10px] px-2 py-0.5 rounded cursor-pointer transition-colors border ${
                      selected
                        ? 'bg-[#3b82f6] text-white border-[#3b82f6]'
                        : 'bg-[var(--color-bg-main)] text-[var(--color-text-secondary)] border-[var(--color-border)] hover:border-[#3b82f6] hover:text-[#3b82f6]'
                    }`}
                  >#{tag}</button>
                )
              })}
            </div>
            <div className="flex gap-2 mt-2">
              <button
                onClick={async () => {
                  setTags(draftTags)
                  setShowTagPicker(false)
                  try {
                    const base = `http://127.0.0.1:${import.meta.env.VITE_LOCAL_SERVER_PORT || 5002}`
                    await fetch(`${base}/api/deployments/${l2.id}`, {
                      method: 'PUT',
                      headers: { 'Content-Type': 'application/json' },
                      body: JSON.stringify({ hashtags: JSON.stringify(draftTags) }),
                    })
                    onRefresh?.()
                  } catch (e) { console.error('Failed to save hashtags:', e) }
                }}
                className="text-[10px] px-3 py-1 rounded-lg bg-[var(--color-success)] text-black font-medium cursor-pointer hover:opacity-80"
              >{ko ? '저장' : 'Save'}</button>
              <button
                onClick={() => setShowTagPicker(false)}
                className="text-[10px] px-3 py-1 rounded-lg text-[var(--color-text-secondary)] cursor-pointer hover:opacity-80"
              >{ko ? '취소' : 'Cancel'}</button>
            </div>
          </div>
        )}
      </div>
    </>
  )
}
