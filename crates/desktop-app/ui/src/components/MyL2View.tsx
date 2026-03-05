import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLang } from '../App'
import { t } from '../i18n'
import L2DetailView from './L2DetailView'
import CreateL2Wizard from './CreateL2Wizard'
import SetupProgressView from './SetupProgressView'
import type { NetworkMode } from './CreateL2Wizard'

export interface L2Config {
  id: string
  name: string
  icon: string
  chainId: number
  description: string
  status: 'running' | 'stopped' | 'starting' | 'created' | 'settingup' | 'error'
  nativeToken: string
  l1Rpc: string
  rpcPort: number
  sequencerStatus: 'running' | 'stopped'
  proverStatus: 'running' | 'stopped'
  hashtags: string[]
  isPublic: boolean
  createdAt: string
  networkMode?: string
}

// Backend appchain format
interface AppchainFromBackend {
  id: string
  name: string
  icon: string
  chain_id: number
  description: string
  status: string
  native_token: string
  l1_rpc_url: string
  l2_rpc_port: number
  prover_type: string
  hashtags: string[]
  is_public: boolean
  created_at: string
  network_mode: string
}

function backendToL2Config(a: AppchainFromBackend): L2Config {
  const statusMap: Record<string, L2Config['status']> = {
    created: 'created', settingup: 'starting', running: 'running', stopped: 'stopped', error: 'error'
  }
  return {
    id: a.id, name: a.name, icon: a.icon, chainId: a.chain_id,
    description: a.description, status: statusMap[a.status] ?? 'stopped',
    nativeToken: a.native_token, l1Rpc: a.l1_rpc_url, rpcPort: a.l2_rpc_port,
    sequencerStatus: a.status === 'running' ? 'running' : 'stopped',
    proverStatus: a.prover_type !== 'none' && a.status === 'running' ? 'running' : 'stopped',
    hashtags: a.hashtags, isPublic: a.is_public, createdAt: a.created_at,
    networkMode: a.network_mode,
  }
}

const statusDot = (status: string) => {
  if (status === 'running') return 'bg-[var(--color-success)]'
  if (status === 'starting' || status === 'settingup') return 'bg-[var(--color-warning)]'
  if (status === 'created') return 'bg-[var(--color-accent)]'
  if (status === 'error') return 'bg-[var(--color-error)]'
  return 'bg-[var(--color-text-secondary)]'
}

interface MyL2ViewProps {
  initialNetwork?: NetworkMode
  onNetworkConsumed?: () => void
}

export default function MyL2View({ initialNetwork, onNetworkConsumed }: MyL2ViewProps = {}) {
  const { lang } = useLang()
  const [l2s, setL2s] = useState<L2Config[]>([])
  const [selectedL2, setSelectedL2] = useState<L2Config | null>(null)
  const [showCreate, setShowCreate] = useState(!!initialNetwork)
  const [createNetwork, setCreateNetwork] = useState<NetworkMode | undefined>(initialNetwork)

  // initialNetwork가 바뀌면 위자드 열기
  useEffect(() => {
    if (initialNetwork) {
      setShowCreate(true)
      setCreateNetwork(initialNetwork)
      onNetworkConsumed?.()
    }
  }, [initialNetwork])
  const [setupChain, setSetupChain] = useState<{ id: string; name: string; icon: string } | null>(null)

  const loadAppchains = async () => {
    try {
      const list = await invoke<AppchainFromBackend[]>('list_appchains')
      setL2s(list.map(backendToL2Config))
    } catch {
      // Backend not ready, keep empty
    }
  }

  useEffect(() => { loadAppchains() }, [])

  const handleCreate = async (config: Record<string, string>) => {
    try {
      const result = await invoke<AppchainFromBackend>('create_appchain', { req: {
        name: config.name,
        icon: config.icon,
        chain_id: parseInt(config.chainId) || 17001,
        description: config.description || '',
        network_mode: config.networkMode || 'local',
        l1_rpc_url: config.l1Rpc,
        l2_rpc_port: parseInt(config.rpcPort) || 1729,
        sequencer_mode: config.sequencerMode,
        native_token: config.nativeToken,
        prover_type: config.proverType,
        is_public: config.isPublic === 'true',
        hashtags: config.hashtags || '',
      }})

      // Start setup
      await invoke('start_appchain_setup', { id: result.id })
      setShowCreate(false)
      setCreateNetwork(undefined)
      setSetupChain({ id: result.id, name: result.name, icon: result.icon })
      loadAppchains()
    } catch (e) {
      console.error('Failed to create appchain:', e)
    }
  }

  if (setupChain) {
    return (
      <SetupProgressView
        chainId={setupChain.id}
        chainName={setupChain.name}
        chainIcon={setupChain.icon}
        onDone={() => { setSetupChain(null); loadAppchains() }}
        onCancel={() => { setSetupChain(null); loadAppchains() }}
      />
    )
  }

  if (showCreate) {
    return (
      <CreateL2Wizard
        onBack={() => { setShowCreate(false); setCreateNetwork(undefined) }}
        onCreate={handleCreate}
        initialNetwork={createNetwork}
      />
    )
  }

  if (selectedL2) {
    return <L2DetailView l2={selectedL2} onBack={() => setSelectedL2(null)} onRefresh={loadAppchains} />
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)] flex items-center justify-between">
        <h1 className="text-base font-semibold">{t('myl2.title', lang)} <span className="text-[var(--color-text-secondary)] text-xs font-normal">{l2s.length}</span></h1>
        <button
          onClick={() => setShowCreate(true)}
          className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] text-xs font-medium px-3 py-1.5 rounded-lg transition-colors cursor-pointer text-[var(--color-accent-text)]"
        >
          + {t('myl2.create', lang)}
        </button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {l2s.length === 0 && (
          <div className="flex flex-col items-center justify-center h-full text-center px-6">
            <div className="text-3xl mb-3">📦</div>
            <p className="text-sm text-[var(--color-text-secondary)]">
              {lang === 'ko' ? '아직 앱체인이 없습니다' : 'No appchains yet'}
            </p>
            <button
              onClick={() => setShowCreate(true)}
              className="mt-3 bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] text-xs font-medium px-4 py-2 rounded-lg transition-colors cursor-pointer text-[var(--color-accent-text)]"
            >
              + {t('myl2.create', lang)}
            </button>
          </div>
        )}
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
                {l2.networkMode && (
                  <span className={`text-[9px] px-1.5 py-0.5 rounded font-medium text-white ${
                    l2.networkMode === 'local' ? 'bg-[var(--color-success)]' :
                    l2.networkMode === 'testnet' ? 'bg-[var(--color-warning)]' :
                    'bg-[var(--color-accent)] text-[var(--color-accent-text)]'
                  }`}>
                    {l2.networkMode}
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
