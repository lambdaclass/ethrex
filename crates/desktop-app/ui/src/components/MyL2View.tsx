import { useState } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'
import L2DetailView from './L2DetailView'
import CreateL2Wizard from './CreateL2Wizard'
import type { NetworkMode } from './CreateL2Wizard'

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

const statusDot = (status: string) => {
  if (status === 'running') return 'bg-[var(--color-success)]'
  if (status === 'starting') return 'bg-[var(--color-warning)]'
  return 'bg-[var(--color-text-secondary)]'
}

interface MyL2ViewProps {
  initialNetwork?: NetworkMode
}

export default function MyL2View({ initialNetwork }: MyL2ViewProps = {}) {
  const { lang } = useLang()
  const [l2s] = useState<L2Config[]>(sampleL2s)
  const [selectedL2, setSelectedL2] = useState<L2Config | null>(null)
  const [showCreate, setShowCreate] = useState(!!initialNetwork)
  const [createNetwork, setCreateNetwork] = useState<NetworkMode | undefined>(initialNetwork)

  if (showCreate) {
    return <CreateL2Wizard onBack={() => { setShowCreate(false); setCreateNetwork(undefined) }} onCreate={() => { setShowCreate(false); setCreateNetwork(undefined) }} initialNetwork={createNetwork} />
  }
  if (selectedL2) {
    return <L2DetailView l2={selectedL2} onBack={() => setSelectedL2(null)} />
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      <div className="px-4 py-3 border-b border-[var(--color-border)] flex items-center justify-between">
        <h1 className="text-base font-semibold">{t('myl2.title', lang)} <span className="text-[var(--color-text-secondary)] text-xs font-normal">{l2s.length}</span></h1>
        <button
          onClick={() => setShowCreate(true)}
          className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] text-xs font-medium px-3 py-1.5 rounded-lg transition-colors cursor-pointer text-[var(--color-accent-text)]"
        >
          + {t('myl2.create', lang)}
        </button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {l2s.map(l2 => (
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
                <span className={`w-2 h-2 rounded-full ${statusDot(l2.status)}`} />
                <span className="text-sm font-medium">{l2.name}</span>
                {l2.isPublic && (
                  <span className="text-[9px] bg-[var(--color-accent)] px-1.5 py-0.5 rounded text-[var(--color-accent-text)] font-medium">
                    {t('myl2.public', lang)}
                  </span>
                )}
              </div>
              <div className="text-[11px] text-[var(--color-text-secondary)] mt-0.5 truncate">{l2.description}</div>
              <div className="flex items-center gap-2 mt-1 text-[10px] text-[var(--color-text-secondary)]">
                <span>#{l2.chainId}</span>
                <span>·</span>
                <span className="flex items-center gap-1">
                  {t('myl2.sequencer', lang)}
                  <span className={`w-1.5 h-1.5 rounded-full ${l2.sequencerStatus === 'running' ? 'bg-[var(--color-success)]' : 'bg-[var(--color-border)]'}`} />
                </span>
                <span className="flex items-center gap-1">
                  {t('myl2.prover', lang)}
                  <span className={`w-1.5 h-1.5 rounded-full ${l2.proverStatus === 'running' ? 'bg-[var(--color-success)]' : 'bg-[var(--color-border)]'}`} />
                </span>
              </div>
            </div>
          </button>
        ))}
      </div>
    </div>
  )
}
