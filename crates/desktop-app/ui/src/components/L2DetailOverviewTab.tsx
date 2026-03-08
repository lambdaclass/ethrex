import { SectionHeader, StatCard } from './ui-atoms'
import type { ChainMetrics } from './L2DetailView'

interface Props {
  ko: boolean
  chain: ChainMetrics
  tags: string[]
  setTags: (tags: string[]) => void
  tagInput: string
  setTagInput: (v: string) => void
}

export default function L2DetailOverviewTab({ ko, chain, tags, setTags, tagInput, setTagInput }: Props) {
  return (
    <>
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
    </>
  )
}
