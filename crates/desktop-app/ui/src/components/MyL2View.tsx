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

interface ContainerInfo {
  name: string
  service: string
  state: string
  status: string
  ports: string
  image: string
  id: string
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

const containerStateDot = (state: string) => {
  if (state === 'running') return 'bg-[var(--color-success)]'
  if (state === 'exited') return 'bg-[var(--color-text-secondary)]'
  if (state === 'created' || state === 'restarting') return 'bg-[var(--color-warning)] animate-pulse'
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

const serviceFriendlyName = (service: string): string => {
  const map: Record<string, string> = {
    'tokamak-app-l1': 'L1',
    'tokamak-app-l2': 'L2',
    'tokamak-app-deployer': 'Deployer',
    'tokamak-app-prover': 'Prover',
  }
  return map[service] || service
}

function formatPorts(ports: string): string {
  if (!ports) return '-'
  // Extract host:container port pairs, show concisely
  const matches = ports.match(/(\d+)->\d+\/tcp/g)
  if (matches) {
    return matches.map(m => m.replace('->',':').replace('/tcp','')).join(', ')
  }
  return ports.length > 40 ? ports.substring(0, 37) + '...' : ports
}

export default function MyL2View() {
  const { lang } = useLang()
  const [l2s, setL2s] = useState<L2Config[]>([])
  const [selectedL2, setSelectedL2] = useState<L2Config | null>(null)
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const [containers, setContainers] = useState<Record<string, ContainerInfo[]>>({})
  const [actionLoading, setActionLoading] = useState<string | null>(null)
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null)

  const loadDeployments = useCallback(async () => {
    try {
      const rows = await invoke<DeploymentFromDB[]>('list_docker_deployments')
      setL2s(rows.map(deploymentToL2Config))
    } catch (e) {
      console.error('Failed to load deployments:', e)
    }
  }, [])

  const loadContainers = useCallback(async (id: string) => {
    try {
      const result = await invoke<ContainerInfo[]>('get_docker_containers', { id })
      setContainers(prev => ({ ...prev, [id]: result }))
    } catch (e) {
      console.error('Failed to load containers:', e)
    }
  }, [])

  useEffect(() => {
    loadDeployments()
    const interval = setInterval(loadDeployments, 5000)
    return () => clearInterval(interval)
  }, [loadDeployments])

  // Auto-refresh containers for expanded deployment
  useEffect(() => {
    if (!expandedId) return
    loadContainers(expandedId)
    const interval = setInterval(() => loadContainers(expandedId), 5000)
    return () => clearInterval(interval)
  }, [expandedId, loadContainers])

  const toggleExpand = (id: string) => {
    if (expandedId === id) {
      setExpandedId(null)
    } else {
      setExpandedId(id)
      loadContainers(id)
    }
  }

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

  const handleStop = async (id: string) => {
    setActionLoading(id)
    try {
      await invoke('stop_docker_deployment', { id })
      await loadDeployments()
      if (expandedId === id) await loadContainers(id)
    } catch (e) {
      console.error('Failed to stop:', e)
    } finally {
      setActionLoading(null)
    }
  }

  const handleStart = async (id: string) => {
    setActionLoading(id)
    try {
      await invoke('start_docker_deployment', { id })
      await loadDeployments()
      if (expandedId === id) await loadContainers(id)
    } catch (e) {
      console.error('Failed to start:', e)
    } finally {
      setActionLoading(null)
    }
  }

  const handleDelete = async (id: string) => {
    if (confirmDeleteId !== id) {
      setConfirmDeleteId(id)
      setTimeout(() => setConfirmDeleteId(prev => prev === id ? null : prev), 3000)
      return
    }
    setConfirmDeleteId(null)
    setActionLoading(id)
    try {
      await invoke('delete_docker_deployment', { id })
      if (expandedId === id) setExpandedId(null)
      await loadDeployments()
    } catch (e) {
      console.error('Failed to delete:', e)
    } finally {
      setActionLoading(null)
    }
  }

  if (selectedL2) {
    return <L2DetailView l2={selectedL2} onBack={() => { setSelectedL2(null); loadDeployments() }} onRefresh={loadDeployments} />
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      {/* Header */}
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)] flex items-center justify-between">
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

      {/* Table Header */}
      {l2s.length > 0 && (
        <div className="flex-1 overflow-y-auto">
          <div className="sticky top-0 z-10 bg-[var(--color-bg-sidebar)] border-b border-[var(--color-border)] px-4 py-2 flex items-center text-[10px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider">
            <div className="w-7" />
            <div className="w-6" />
            <div className="flex-[2] min-w-0">Name</div>
            <div className="flex-1">Status</div>
            <div className="flex-1">{lang === 'ko' ? '포트' : 'Ports'}</div>
            <div className="flex-1">{lang === 'ko' ? '단계' : 'Phase'}</div>
            <div className="w-[120px] text-right">{lang === 'ko' ? '작업' : 'Actions'}</div>
          </div>

          {l2s.map(l2 => {
            const isExpanded = expandedId === l2.id
            const l2Containers = containers[l2.id] || []

            return (
              <div key={l2.id}>
                {/* Deployment Row */}
                <div className={`flex items-center px-4 py-2.5 border-b border-[var(--color-border)] hover:bg-[var(--color-bg-sidebar)] transition-colors ${isExpanded ? 'bg-[var(--color-bg-sidebar)]' : ''}`}>
                  {/* Expand toggle */}
                  <button
                    onClick={() => toggleExpand(l2.id)}
                    className="w-7 flex items-center justify-center cursor-pointer text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-transform"
                  >
                    <svg
                      width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                      strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round"
                      className={`transition-transform ${isExpanded ? 'rotate-90' : ''}`}
                    >
                      <polyline points="9 18 15 12 9 6"/>
                    </svg>
                  </button>

                  {/* Status dot */}
                  <div className="w-6 flex items-center justify-center">
                    <span className={`w-2.5 h-2.5 rounded-full ${statusDot(l2.status)}`} />
                  </div>

                  {/* Name - clickable for detail */}
                  <button
                    onClick={() => {
                      if (l2.status === 'starting') {
                        openDeployManager()
                      } else {
                        setSelectedL2(l2)
                      }
                    }}
                    className="flex-[2] min-w-0 text-left cursor-pointer"
                  >
                    <div className="flex items-center gap-2">
                      <span className="text-[13px] font-medium truncate">{l2.name}</span>
                      <span className="text-[9px] bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] px-1.5 py-0.5 rounded font-medium text-[var(--color-text-secondary)] flex-shrink-0">
                        {l2.programSlug}
                      </span>
                    </div>
                  </button>

                  {/* Status */}
                  <div className="flex-1">
                    <span className={`text-[11px] font-medium ${
                      l2.status === 'running' ? 'text-[var(--color-success)]'
                      : l2.status === 'starting' ? 'text-[var(--color-warning)]'
                      : l2.status === 'error' ? 'text-[var(--color-error)]'
                      : 'text-[var(--color-text-secondary)]'
                    }`}>
                      {statusLabel(l2.status, lang)}
                    </span>
                  </div>

                  {/* Ports */}
                  <div className="flex-1 text-[11px] text-[var(--color-text-secondary)]">
                    {l2.l1Port && <span>L1:{l2.l1Port}</span>}
                    {l2.l1Port && l2.l2Port && <span className="mx-1">·</span>}
                    {l2.l2Port && <span>L2:{l2.l2Port}</span>}
                    {!l2.l1Port && !l2.l2Port && '-'}
                  </div>

                  {/* Phase */}
                  <div className="flex-1 text-[11px] text-[var(--color-text-secondary)]">
                    {l2.phase}
                  </div>

                  {/* Actions */}
                  <div className="w-[120px] flex items-center justify-end gap-1.5">
                    {l2.status === 'running' ? (
                      <button
                        onClick={() => handleStop(l2.id)}
                        disabled={actionLoading === l2.id}
                        className="text-[10px] px-2 py-1 rounded-md bg-[var(--color-warning)] text-white hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-50"
                      >
                        {actionLoading === l2.id ? '...' : (lang === 'ko' ? '중지' : 'Stop')}
                      </button>
                    ) : l2.status === 'stopped' ? (
                      <button
                        onClick={() => handleStart(l2.id)}
                        disabled={actionLoading === l2.id}
                        className="text-[10px] px-2 py-1 rounded-md bg-[var(--color-success)] text-white hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-50"
                      >
                        {actionLoading === l2.id ? '...' : (lang === 'ko' ? '시작' : 'Start')}
                      </button>
                    ) : l2.status === 'starting' ? (
                      <button
                        onClick={openDeployManager}
                        className="text-[10px] px-2 py-1 rounded-md bg-[var(--color-warning)]/20 text-[var(--color-warning)] hover:opacity-80 transition-opacity cursor-pointer"
                      >
                        {lang === 'ko' ? '진행 보기' : 'View'}
                      </button>
                    ) : null}
                    <button
                      onClick={() => handleDelete(l2.id)}
                      disabled={actionLoading === l2.id}
                      className={`text-[10px] px-2 py-1 rounded-md text-white hover:opacity-80 transition-all cursor-pointer disabled:opacity-50 ${
                        confirmDeleteId === l2.id ? 'bg-red-700 ring-1 ring-red-400' : 'bg-[var(--color-error)]'
                      }`}
                    >
                      {actionLoading === l2.id ? '...'
                        : confirmDeleteId === l2.id ? (lang === 'ko' ? '확인' : 'OK')
                        : (lang === 'ko' ? '삭제' : 'Del')}
                    </button>
                  </div>
                </div>

                {/* Expanded: Container List */}
                {isExpanded && (
                  <div className="bg-[var(--color-bg-main)]">
                    {l2Containers.length === 0 ? (
                      <div className="px-4 py-3 pl-14 text-[11px] text-[var(--color-text-secondary)] border-b border-[var(--color-border)]">
                        {l2.status === 'starting'
                          ? (lang === 'ko' ? '컨테이너 생성 중...' : 'Creating containers...')
                          : (lang === 'ko' ? '컨테이너 없음' : 'No containers')}
                      </div>
                    ) : (
                      l2Containers.map(c => (
                        <div
                          key={c.id || c.name}
                          className="flex items-center px-4 py-2 pl-14 border-b border-[var(--color-border)]/50 hover:bg-[var(--color-bg-sidebar)]/50 transition-colors text-[11px]"
                        >
                          {/* Container status dot */}
                          <div className="w-6 flex items-center justify-center">
                            <span className={`w-2 h-2 rounded-full ${containerStateDot(c.state)}`} />
                          </div>

                          {/* Service name */}
                          <div className="flex-[2] min-w-0 font-medium">
                            {serviceFriendlyName(c.service)}
                          </div>

                          {/* State */}
                          <div className="flex-1">
                            <span className={`${
                              c.state === 'running' ? 'text-[var(--color-success)]'
                              : c.state === 'exited' ? 'text-[var(--color-text-secondary)]'
                              : 'text-[var(--color-warning)]'
                            }`}>
                              {c.status || c.state}
                            </span>
                          </div>

                          {/* Ports */}
                          <div className="flex-1 text-[var(--color-text-secondary)] truncate">
                            {formatPorts(c.ports)}
                          </div>

                          {/* Image */}
                          <div className="flex-1 text-[var(--color-text-secondary)] truncate">
                            {c.image ? c.image.split('/').pop()?.split(':')[0] || c.image : '-'}
                          </div>

                          {/* Container ID */}
                          <div className="w-[120px] text-right text-[var(--color-text-secondary)] font-mono">
                            {c.id ? c.id.substring(0, 12) : '-'}
                          </div>
                        </div>
                      ))
                    )}
                  </div>
                )}
              </div>
            )
          })}
        </div>
      )}

      {/* Error messages */}
      {l2s.some(l2 => l2.errorMessage) && (
        <div className="px-4 py-2 border-t border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
          {l2s.filter(l2 => l2.errorMessage).map(l2 => (
            <div key={l2.id} className="text-[10px] text-[var(--color-error)] truncate">
              {l2.name}: {l2.errorMessage}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
