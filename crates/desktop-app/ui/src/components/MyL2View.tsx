import { useState, useEffect, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { useLang } from '../App'
import { t } from '../i18n'
import L2DetailView from './L2DetailView'

// Deployment from Rust (direct SQLite read)
interface DeploymentFromDB {
  id: string
  program_slug: string
  name: string
  chain_id: number | null
  rpc_url: string | null
  status: string
  deploy_method: string
  docker_project: string | null
  l1_port: number | null
  l2_port: number | null
  proof_coord_port: number | null
  phase: string
  bridge_address: string | null
  proposer_address: string | null
  error_message: string | null
  is_public: number
  created_at: number
}

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
  // Docker deployment fields
  source: 'docker'
  programSlug: string
  phase: string
  l1Port: number | null
  l2Port: number | null
  dockerProject: string | null
  errorMessage: string | null
}

function deploymentToL2Config(d: DeploymentFromDB): L2Config {
  const statusMap: Record<string, L2Config['status']> = {
    running: 'running', stopped: 'stopped', deploying: 'starting',
    configured: 'created', failed: 'error', error: 'error', destroyed: 'stopped',
  }
  return {
    id: d.id,
    name: d.name,
    icon: d.program_slug === 'zk-dex' ? '🔐' : '⛓️',
    chainId: d.chain_id || 0,
    description: `${d.program_slug} · ${d.phase}`,
    status: statusMap[d.status] ?? 'stopped',
    nativeToken: 'TON',
    l1Rpc: d.rpc_url || '',
    rpcPort: d.l2_port || 0,
    sequencerStatus: d.status === 'running' ? 'running' : 'stopped',
    proverStatus: 'stopped',
    hashtags: [],
    isPublic: d.is_public === 1,
    createdAt: new Date(d.created_at).toISOString(),
    networkMode: 'local',
    source: 'docker',
    programSlug: d.program_slug,
    phase: d.phase,
    l1Port: d.l1_port,
    l2Port: d.l2_port,
    dockerProject: d.docker_project,
    errorMessage: d.error_message,
  }
}

const statusDot = (status: string) => {
  if (status === 'running') return 'bg-[var(--color-success)]'
  if (status === 'starting' || status === 'settingup' || status === 'deploying') return 'bg-[var(--color-warning)] animate-pulse'
  if (status === 'created' || status === 'configured') return 'bg-[var(--color-accent)]'
  if (status === 'error' || status === 'failed') return 'bg-[var(--color-error)]'
  return 'bg-[var(--color-text-secondary)]'
}

const statusLabel = (status: string, lang: string) => {
  const labels: Record<string, Record<string, string>> = {
    running: { ko: '실행 중', en: 'Running' },
    stopped: { ko: '중지됨', en: 'Stopped' },
    starting: { ko: '배포 중', en: 'Deploying' },
    created: { ko: '설정됨', en: 'Configured' },
    error: { ko: '오류', en: 'Error' },
  }
  const l = lang === 'ko' ? 'ko' : 'en'
  return labels[status]?.[l] || status
}

const statusFilters = ['all', 'running', 'stopped', 'error'] as const
type StatusFilter = typeof statusFilters[number]

const statusFilterLabel = (filter: StatusFilter, lang: string) => {
  const labels: Record<StatusFilter, Record<string, string>> = {
    all: { ko: '전체', en: 'All' },
    running: { ko: '실행 중', en: 'Running' },
    stopped: { ko: '중지됨', en: 'Stopped' },
    error: { ko: '오류', en: 'Error' },
  }
  return labels[filter][lang === 'ko' ? 'ko' : 'en']
}

function timeAgo(isoDate: string, lang: string): string {
  const diff = Date.now() - new Date(isoDate).getTime()
  const mins = Math.floor(diff / 60000)
  if (mins < 1) return lang === 'ko' ? '방금' : 'just now'
  if (mins < 60) return `${mins}${lang === 'ko' ? '분 전' : 'm ago'}`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}${lang === 'ko' ? '시간 전' : 'h ago'}`
  const days = Math.floor(hours / 24)
  return `${days}${lang === 'ko' ? '일 전' : 'd ago'}`
}

export default function MyL2View() {
  const { lang } = useLang()
  const [l2s, setL2s] = useState<L2Config[]>([])
  const [selectedL2, setSelectedL2] = useState<L2Config | null>(null)
  const [actionLoading, setActionLoading] = useState<string | null>(null)
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null)
  const [searchQuery, setSearchQuery] = useState('')
  const [statusFilter, setStatusFilter] = useState<StatusFilter>('all')

  const loadDeployments = useCallback(async () => {
    try {
      const rows = await invoke<DeploymentFromDB[]>('list_docker_deployments')
      setL2s(rows.map(deploymentToL2Config))
    } catch (e) {
      console.error('Failed to load deployments:', e)
    }
  }, [])

  useEffect(() => {
    loadDeployments()
    const interval = setInterval(loadDeployments, 5000)
    return () => clearInterval(interval)
  }, [loadDeployments])

  const openDeployManager = async () => {
    try {
      const url = await invoke<string>('open_deployment_ui')
      const existing = await WebviewWindow.getByLabel('deploy-manager')
      if (existing) {
        await existing.show()
        await existing.setFocus()
      } else {
        new WebviewWindow('deploy-manager', {
          url,
          title: 'Tokamak L2 Manager',
          width: 1100,
          height: 800,
          minWidth: 800,
          minHeight: 600,
          center: true,
        })
      }
    } catch (e) {
      console.error('Failed to open deployment manager:', e)
    }
  }

  const handleStop = async (e: React.MouseEvent, id: string) => {
    e.stopPropagation()
    setActionLoading(id)
    try {
      await invoke('stop_docker_deployment', { id })
      await loadDeployments()
    } catch (e) {
      console.error('Failed to stop:', e)
    } finally {
      setActionLoading(null)
    }
  }

  const handleStart = async (e: React.MouseEvent, id: string) => {
    e.stopPropagation()
    setActionLoading(id)
    try {
      await invoke('start_docker_deployment', { id })
      await loadDeployments()
    } catch (e) {
      console.error('Failed to start:', e)
    } finally {
      setActionLoading(null)
    }
  }

  const handleDelete = async (e: React.MouseEvent, id: string) => {
    e.stopPropagation()
    if (confirmDeleteId !== id) {
      setConfirmDeleteId(id)
      setTimeout(() => setConfirmDeleteId(prev => prev === id ? null : prev), 3000)
      return
    }
    setConfirmDeleteId(null)
    setActionLoading(id)
    try {
      await invoke('delete_docker_deployment', { id })
      await loadDeployments()
    } catch (e) {
      console.error('Failed to delete:', e)
    } finally {
      setActionLoading(null)
    }
  }

  const filtered = l2s.filter(l2 => {
    const matchesSearch = searchQuery === '' ||
      l2.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
      l2.programSlug.toLowerCase().includes(searchQuery.toLowerCase()) ||
      l2.description.toLowerCase().includes(searchQuery.toLowerCase())
    const matchesStatus = statusFilter === 'all' ||
      (statusFilter === 'running' && l2.status === 'running') ||
      (statusFilter === 'stopped' && (l2.status === 'stopped' || l2.status === 'created')) ||
      (statusFilter === 'error' && (l2.status === 'error'))
    return matchesSearch && matchesStatus
  })

  if (selectedL2) {
    return <L2DetailView l2={selectedL2} onBack={() => { setSelectedL2(null); loadDeployments() }} onRefresh={loadDeployments} />
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      {/* Header */}
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
        <div className="flex items-center justify-between">
          <h1 className="text-base font-semibold">
            {t('myl2.title', lang)} <span className="text-[var(--color-text-secondary)] text-xs font-normal">{l2s.length}</span>
          </h1>
          <button
            onClick={openDeployManager}
            className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] text-xs font-medium px-3 py-1.5 rounded-lg transition-colors cursor-pointer text-[var(--color-accent-text)]"
          >
            {lang === 'ko' ? 'L2 매니저' : 'L2 Manager'}
          </button>
        </div>
        {/* Search & Filter */}
        <div className="flex items-center gap-2 mt-2">
          <div className="relative flex-1">
            <input
              type="text"
              value={searchQuery}
              onChange={e => setSearchQuery(e.target.value)}
              placeholder={lang === 'ko' ? '앱체인 이름으로 검색...' : 'Search by name...'}
              className="w-full bg-[var(--color-bg-sidebar)] rounded-lg px-3 py-2 text-[13px] outline-none placeholder-[var(--color-text-secondary)] border border-[var(--color-border)] pl-8"
            />
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--color-text-secondary)]">
              <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
            </svg>
          </div>
          <select
            value={statusFilter}
            onChange={e => setStatusFilter(e.target.value as StatusFilter)}
            className="bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] rounded-lg px-3 py-2 text-[13px] outline-none cursor-pointer"
          >
            {statusFilters.map(f => (
              <option key={f} value={f}>{statusFilterLabel(f, lang)}</option>
            ))}
          </select>
        </div>
      </div>

      {/* Empty state */}
      {l2s.length === 0 && (
        <div className="flex flex-col items-center justify-center flex-1 text-center px-6">
          <div className="text-3xl mb-3">📦</div>
          <p className="text-sm text-[var(--color-text-secondary)]">
            {lang === 'ko' ? '아직 배포된 앱체인이 없습니다' : 'No deployed appchains yet'}
          </p>
          <button
            onClick={openDeployManager}
            className="mt-3 bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] text-xs font-medium px-4 py-2 rounded-lg transition-colors cursor-pointer text-[var(--color-accent-text)]"
          >
            {lang === 'ko' ? 'L2 매니저 열기' : 'Open L2 Manager'}
          </button>
        </div>
      )}

      {/* Card List */}
      {l2s.length > 0 && (
        <div className="flex-1 overflow-y-auto">
          {filtered.length === 0 ? (
            <div className="flex items-center justify-center h-full text-[var(--color-text-secondary)] text-[13px]">
              {lang === 'ko' ? '검색 결과가 없습니다' : 'No results found'}
            </div>
          ) : (
            filtered.map(l2 => (
              <button
                key={l2.id}
                onClick={() => {
                  if (l2.status === 'starting') {
                    openDeployManager()
                  } else {
                    setSelectedL2(l2)
                  }
                }}
                className="w-full px-4 py-3 flex items-center gap-3 hover:bg-[var(--color-bg-sidebar)] transition-colors cursor-pointer border-b border-[var(--color-border)] text-left"
              >
                {/* Icon */}
                <div className="w-10 h-10 rounded-xl bg-[var(--color-bg-sidebar)] flex items-center justify-center text-xl flex-shrink-0 border border-[var(--color-border)]">
                  {l2.icon}
                </div>

                {/* Info */}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-1.5">
                    <span className="text-sm font-medium truncate">{l2.name}</span>
                    <span className={`w-2 h-2 rounded-full flex-shrink-0 ${statusDot(l2.status)}`} />
                    <span className={`text-[11px] font-medium ${
                      l2.status === 'running' ? 'text-[var(--color-success)]'
                      : l2.status === 'starting' ? 'text-[var(--color-warning)]'
                      : l2.status === 'error' ? 'text-[var(--color-error)]'
                      : 'text-[var(--color-text-secondary)]'
                    }`}>
                      {statusLabel(l2.status, lang)}
                    </span>
                  </div>
                  <div className="text-[11px] text-[var(--color-text-secondary)] truncate mt-0.5">
                    {l2.errorMessage
                      ? <span className="text-[var(--color-error)]">{l2.errorMessage}</span>
                      : <>Chain ID: {l2.chainId || '-'} · {l2.l1Port ? `L1:${l2.l1Port}` : ''}{l2.l1Port && l2.l2Port ? ' · ' : ''}{l2.l2Port ? `L2:${l2.l2Port}` : ''}</>
                    }
                  </div>
                  <div className="flex gap-1 mt-1">
                    <span className="text-[10px] text-[var(--color-tag-text)] bg-[var(--color-tag-bg)] px-1.5 py-0.5 rounded">
                      {l2.programSlug}
                    </span>
                    <span className="text-[10px] text-[var(--color-tag-text)] bg-[var(--color-tag-bg)] px-1.5 py-0.5 rounded">
                      {l2.phase}
                    </span>
                    {l2.isPublic && (
                      <span className="text-[10px] text-[var(--color-tag-text)] bg-[var(--color-tag-bg)] px-1.5 py-0.5 rounded">
                        {t('myl2.public', lang)}
                      </span>
                    )}
                  </div>
                </div>

                {/* Right side: time + actions */}
                <div className="flex flex-col items-end gap-1.5 flex-shrink-0">
                  <span className="text-[11px] text-[var(--color-text-secondary)]">
                    {timeAgo(l2.createdAt, lang)}
                  </span>
                  <div className="flex items-center gap-1">
                    {l2.status === 'running' ? (
                      <span
                        onClick={(e) => handleStop(e, l2.id)}
                        className={`text-[10px] px-2 py-0.5 rounded-md bg-[var(--color-warning)] text-white hover:opacity-80 transition-opacity cursor-pointer ${actionLoading === l2.id ? 'opacity-50' : ''}`}
                      >
                        {actionLoading === l2.id ? '...' : (lang === 'ko' ? '중지' : 'Stop')}
                      </span>
                    ) : l2.status === 'stopped' ? (
                      <span
                        onClick={(e) => handleStart(e, l2.id)}
                        className={`text-[10px] px-2 py-0.5 rounded-md bg-[var(--color-success)] text-white hover:opacity-80 transition-opacity cursor-pointer ${actionLoading === l2.id ? 'opacity-50' : ''}`}
                      >
                        {actionLoading === l2.id ? '...' : (lang === 'ko' ? '시작' : 'Start')}
                      </span>
                    ) : l2.status === 'starting' ? (
                      <span className="text-[10px] px-2 py-0.5 rounded-md bg-[var(--color-warning)]/20 text-[var(--color-warning)]">
                        {lang === 'ko' ? '배포 중' : 'Deploying'}
                      </span>
                    ) : null}
                    <span
                      onClick={(e) => handleDelete(e, l2.id)}
                      className={`text-[10px] px-2 py-0.5 rounded-md text-white hover:opacity-80 transition-all cursor-pointer ${
                        confirmDeleteId === l2.id ? 'bg-red-700 ring-1 ring-red-400' : 'bg-[var(--color-error)]'
                      } ${actionLoading === l2.id ? 'opacity-50' : ''}`}
                    >
                      {actionLoading === l2.id ? '...'
                        : confirmDeleteId === l2.id ? (lang === 'ko' ? '확인' : 'OK')
                        : (lang === 'ko' ? '삭제' : 'Del')}
                    </span>
                  </div>
                </div>
              </button>
            ))
          )}
        </div>
      )}
    </div>
  )
}
