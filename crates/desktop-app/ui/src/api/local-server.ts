import { invoke } from '@tauri-apps/api/core'

export interface LocalServerStatus {
  running: boolean
  healthy: boolean
  url: string
  port: number
}

export interface Deployment {
  id: string
  program_slug: string
  name: string
  chain_id: number | null
  rpc_url: string | null
  status: string
  deploy_method: string
  config: string | null
  host_id: string | null
  docker_project: string | null
  deploy_dir: string | null
  l1_port: number | null
  l2_port: number | null
  proof_coord_port: number | null
  phase: string
  bridge_address: string | null
  proposer_address: string | null
  error_message: string | null
  tools_l1_explorer_port: number | null
  tools_l2_explorer_port: number | null
  tools_bridge_ui_port: number | null
  tools_db_port: number | null
  tools_metrics_port: number | null
  is_public: number
  created_at: number
}

export interface Host {
  id: string
  name: string
  hostname: string
  port: number
  username: string
  auth_method: string
  status: string
  last_tested: number | null
  created_at: number
}

// Tauri commands (process management)
export const startLocalServer = () => invoke<string>('start_local_server')
export const stopLocalServer = () => invoke<void>('stop_local_server')
export const getLocalServerStatus = () => invoke<LocalServerStatus>('get_local_server_status')
export const openDeploymentUI = () => invoke<void>('open_deployment_ui')

// Direct API calls to local-server (REST)
class LocalServerAPI {
  private baseUrl = 'http://127.0.0.1:5002'

  async setPort(port: number) {
    this.baseUrl = `http://127.0.0.1:${port}`
  }

  private async fetch<T>(path: string, options?: RequestInit): Promise<T> {
    const resp = await window.fetch(`${this.baseUrl}${path}`, {
      headers: { 'Content-Type': 'application/json' },
      ...options,
    })
    if (!resp.ok) {
      const err = await resp.json().catch(() => ({ error: resp.statusText }))
      throw new Error(err.error || resp.statusText)
    }
    return resp.json()
  }

  // Health
  health() {
    return this.fetch<{ status: string; version: string }>('/api/health')
  }

  // Deployments
  listDeployments() {
    return this.fetch<{ deployments: Deployment[] }>('/api/deployments')
  }

  getDeployment(id: string) {
    return this.fetch<{ deployment: Deployment }>(`/api/deployments/${id}`)
  }

  createDeployment(data: { programSlug?: string; name: string; chainId?: number; config?: Record<string, unknown> }) {
    return this.fetch<{ deployment: Deployment }>('/api/deployments', {
      method: 'POST',
      body: JSON.stringify(data),
    })
  }

  deleteDeployment(id: string) {
    return this.fetch<{ ok: boolean }>(`/api/deployments/${id}`, { method: 'DELETE' })
  }

  // Lifecycle
  provisionDeployment(id: string) {
    return this.fetch<{ deployment: Deployment }>(`/api/deployments/${id}/provision`, { method: 'POST' })
  }

  stopDeployment(id: string) {
    return this.fetch<{ deployment: Deployment }>(`/api/deployments/${id}/stop`, { method: 'POST' })
  }

  startDeployment(id: string) {
    return this.fetch<{ deployment: Deployment }>(`/api/deployments/${id}/start`, { method: 'POST' })
  }

  destroyDeployment(id: string) {
    return this.fetch<{ deployment: Deployment }>(`/api/deployments/${id}/destroy`, { method: 'POST' })
  }

  // Monitoring
  getDeploymentStatus(id: string) {
    return this.fetch<Record<string, unknown>>(`/api/deployments/${id}/status`)
  }

  // Logs (SSE)
  streamLogs(id: string, onEvent: (data: Record<string, unknown>) => void): EventSource {
    const es = new EventSource(`${this.baseUrl}/api/deployments/${id}/logs/stream`)
    es.onmessage = (e) => {
      try { onEvent(JSON.parse(e.data)) } catch {}
    }
    return es
  }

  // Events (SSE)
  streamEvents(id: string, onEvent: (data: Record<string, unknown>) => void): EventSource {
    const es = new EventSource(`${this.baseUrl}/api/deployments/${id}/events`)
    es.onmessage = (e) => {
      try { onEvent(JSON.parse(e.data)) } catch {}
    }
    return es
  }

  // Testnet utilities
  checkRpc(rpcUrl: string) {
    return this.fetch<{ ok: boolean; chainId: number; chainName: string; blockNumber: number }>('/api/deployments/testnet/check-rpc', {
      method: 'POST',
      body: JSON.stringify({ rpcUrl }),
    })
  }

  listKeychainAccounts() {
    return this.fetch<{ accounts: string[] }>('/api/deployments/keychain/accounts')
  }

  resolveKeys(data: { rpcUrl: string; deployerKey: string; committerKey?: string; proofCoordinatorKey?: string; bridgeOwnerKey?: string }) {
    return this.fetch<{
      roles: Record<string, { address: string; balance: string; label: string; error?: string }>
      gasPriceGwei: string
      estimatedDeployCostEth: string
      deployerSufficient: boolean
    }>('/api/deployments/testnet/resolve-keys', {
      method: 'POST',
      body: JSON.stringify(data),
    })
  }

  estimateGas(rpcUrl: string) {
    return this.fetch<{
      chainId: number; chainName: string; gasPriceGwei: string
      breakdown: Record<string, { gas: number; label: string; detail: string; interval: string | null; costEth: string }>
      totalGas: string; totalCostEth: string
    }>('/api/deployments/testnet/estimate-gas', {
      method: 'POST',
      body: JSON.stringify({ rpcUrl }),
    })
  }

  checkImage(slug: string) {
    return this.fetch<{ exists: boolean; image: string | null }>(`/api/deployments/check-image/${encodeURIComponent(slug)}`)
  }

  checkBalance(data: { rpcUrl: string; address: string; role?: string }) {
    return this.fetch<{
      address: string; role: string; balanceEth: string; chainId: number
      gasPriceGwei: string; estimatedGas: number; gasLabel: string
      estimatedCostEth: string; sufficient: boolean
    }>('/api/deployments/testnet/check-balance', {
      method: 'POST',
      body: JSON.stringify(data),
    })
  }

  // Hosts
  listHosts() {
    return this.fetch<{ hosts: Host[] }>('/api/hosts')
  }

  createHost(data: { name: string; hostname: string; port?: number; username: string; authMethod?: string; privateKey?: string }) {
    return this.fetch<{ host: Host }>('/api/hosts', {
      method: 'POST',
      body: JSON.stringify(data),
    })
  }

  testHost(id: string) {
    return this.fetch<{ ok: boolean; docker: boolean }>(`/api/hosts/${id}/test`, { method: 'POST' })
  }

  deleteHost(id: string) {
    return this.fetch<{ ok: boolean }>(`/api/hosts/${id}`, { method: 'DELETE' })
  }
}

export const localServerAPI = new LocalServerAPI()
