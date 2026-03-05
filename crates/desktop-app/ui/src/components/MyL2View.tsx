import { useState } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'
import L2DetailView from './L2DetailView'
import CreateL2Wizard from './CreateL2Wizard'

export interface L2Config {
  id: string
  name: string
  icon: string
  chainId: number
  description: string
  status: 'running' | 'stopped' | 'starting'
  nativeToken: string
  l1Rpc: string
  rpcPort: number
  sequencerStatus: 'running' | 'stopped'
  proverStatus: 'running' | 'stopped'
  hashtags: string[]
  isPublic: boolean
  createdAt: string
}

const sampleL2s: L2Config[] = [
  {
    id: '1', name: 'DEX Chain', icon: '🔄', chainId: 17001,
    description: 'Low-fee decentralized exchange',
    status: 'running', nativeToken: 'TON', l1Rpc: 'http://localhost:8545',
    rpcPort: 8547, sequencerStatus: 'running', proverStatus: 'running',
    hashtags: ['DeFi', 'DEX'], isPublic: true, createdAt: '2024-01-15',
  },
  {
    id: '2', name: 'NFT Chain', icon: '🎨', chainId: 17002,
    description: 'NFT minting and marketplace',
    status: 'stopped', nativeToken: 'TON', l1Rpc: 'http://localhost:8545',
    rpcPort: 8548, sequencerStatus: 'stopped', proverStatus: 'stopped',
    hashtags: ['NFT', 'Art'], isPublic: false, createdAt: '2024-02-20',
  },
  {
    id: '3', name: 'Test Chain', icon: '🧪', chainId: 17003,
    description: 'Development and testing',
    status: 'starting', nativeToken: 'TON', l1Rpc: 'http://localhost:8545',
    rpcPort: 8549, sequencerStatus: 'running', proverStatus: 'stopped',
    hashtags: ['Dev', 'Test'], isPublic: false, createdAt: '2024-03-01',
  },
]

const statusIcon = (status: string) => {
  if (status === 'running') return '🟢'
  if (status === 'starting') return '🟡'
  return '🔴'
}

const statusColor = (status: string) => {
  if (status === 'running') return 'var(--color-success)'
  if (status === 'starting') return 'var(--color-warning)'
  return 'var(--color-text-secondary)'
}

export default function MyL2View() {
  const { lang } = useLang()
  const [l2s] = useState<L2Config[]>(sampleL2s)
  const [selectedL2, setSelectedL2] = useState<L2Config | null>(null)
  const [showCreate, setShowCreate] = useState(false)

  if (showCreate) {
    return <CreateL2Wizard onBack={() => setShowCreate(false)} onCreate={(config) => {
      console.log('Create L2:', config)
      setShowCreate(false)
    }} />
  }

  if (selectedL2) {
    return <L2DetailView l2={selectedL2} onBack={() => setSelectedL2(null)} />
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-chat)]">
      {/* Header */}
      <div className="px-6 py-4 border-b border-[var(--color-border)] flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold">{t('myl2.title', lang)}</h1>
          <p className="text-xs text-[var(--color-text-secondary)] mt-0.5">
            {l2s.length} {t('myl2.chains', lang)}
          </p>
        </div>
        <button
          onClick={() => setShowCreate(true)}
          className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] text-sm px-4 py-2 rounded-lg transition-colors cursor-pointer"
        >
          + {t('myl2.create', lang)}
        </button>
      </div>

      {/* L2 List */}
      <div className="flex-1 overflow-y-auto">
        {l2s.map(l2 => (
          <button
            key={l2.id}
            onClick={() => setSelectedL2(l2)}
            className="w-full px-6 py-4 flex items-center gap-4 hover:bg-[var(--color-border)] transition-colors cursor-pointer border-b border-[var(--color-border)] text-left"
          >
            {/* Icon */}
            <div className="w-12 h-12 rounded-xl bg-[var(--color-bubble-ai)] flex items-center justify-center text-2xl flex-shrink-0">
              {l2.icon}
            </div>

            {/* Info */}
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2">
                <span className="font-medium">{l2.name}</span>
                <span className="text-xs text-[var(--color-text-secondary)]">#{l2.chainId}</span>
                {l2.isPublic && (
                  <span className="text-[10px] bg-[var(--color-accent)] px-1.5 py-0.5 rounded text-white">
                    {t('myl2.public', lang)}
                  </span>
                )}
              </div>
              <div className="text-xs text-[var(--color-text-secondary)] mt-0.5">{l2.description}</div>
              <div className="flex items-center gap-3 mt-1.5">
                <span className="flex items-center gap-1 text-[10px]" style={{ color: statusColor(l2.sequencerStatus) }}>
                  {statusIcon(l2.sequencerStatus)} {t('myl2.sequencer', lang)}
                </span>
                <span className="flex items-center gap-1 text-[10px]" style={{ color: statusColor(l2.proverStatus) }}>
                  {statusIcon(l2.proverStatus)} {t('myl2.prover', lang)}
                </span>
              </div>
            </div>

            {/* Status */}
            <div className="flex flex-col items-end flex-shrink-0">
              <span className="text-xs" style={{ color: statusColor(l2.status) }}>
                {statusIcon(l2.status)} {t(`myl2.status.${l2.status}`, lang)}
              </span>
              <span className="text-[10px] text-[var(--color-text-secondary)] mt-1">
                {l2.nativeToken}
              </span>
            </div>
          </button>
        ))}
      </div>
    </div>
  )
}
