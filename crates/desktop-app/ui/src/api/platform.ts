/**
 * Platform API client for Desktop app.
 * Connects to the Tokamak Platform service (showroom) for:
 * - Program Store browsing
 * - Open Appchain registration
 * - Authentication (OAuth token reuse via OS Keychain)
 */

import { invoke } from '@tauri-apps/api/core'

const DEFAULT_PLATFORM_URL = import.meta.env.VITE_PLATFORM_URL || 'https://tokamak-appchain.vercel.app'

// Keychain-backed token management
export const platformAuth = {
  saveToken: (token: string) => invoke<void>('save_platform_token', { token }),
  getToken: () => invoke<string | null>('get_platform_token'),
  deleteToken: () => invoke<void>('delete_platform_token'),
}

export interface Program {
  id: string
  program_id: string
  name: string
  description: string | null
  category: string
  icon_url: string | null
  status: string
  use_count: number
  is_official: boolean
  created_at: number
}

export interface PlatformUser {
  id: string
  email: string
  name: string
  role: string
  picture: string | null
}

export interface OpenAppchain {
  id: string
  name: string
  chain_id: number
  program_slug: string
  rpc_url: string
  status: string
}

export interface StoreAppchain {
  id: string
  name: string
  description: string | null
  chain_id: number | null
  l2_chain_id: number | null
  l1_chain_id: number | null
  rpc_url: string | null
  status: string
  stack_type: string | null
  network_mode: string | null
  rollup_type: string | null
  explorer_url: string | null
  dashboard_url: string | null
  bridge_url: string | null
  native_token_symbol: string | null
  native_token_decimals: number | null
  native_token_type: string | null
  native_token_l1_address: string | null
  l1_contracts: Record<string, string>
  operator_name: string | null
  operator_website: string | null
  screenshots: string[]
  social_links: Record<string, string>
  hashtags: string[]
  owner_wallet: string | null
  identity_contract: string | null
  avg_rating: number | null
  review_count: number
  comment_count: number
  created_at: number
  // From legacy deployments
  program_name?: string
  program_slug?: string
  owner_name?: string
}

class PlatformAPI {
  private baseUrl: string
  private token: string | null = null

  constructor() {
    this.baseUrl = DEFAULT_PLATFORM_URL
  }

  setBaseUrl(url: string) {
    this.baseUrl = url.replace(/\/$/, '')
  }

  getBaseUrl() {
    return this.baseUrl
  }

  setToken(token: string | null) {
    this.token = token
  }

  private async fetch<T>(path: string, options?: RequestInit): Promise<T> {
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    }
    if (this.token) {
      headers['Authorization'] = `Bearer ${this.token}`
    }

    const resp = await window.fetch(`${this.baseUrl}${path}`, {
      ...options,
      headers: { ...headers, ...(options?.headers as Record<string, string> || {}) },
    })
    const data = await resp.json()
    if (!resp.ok) {
      if (resp.status === 401 && this.token) {
        this.token = null
        platformAuth.deleteToken().catch(() => {})
      }
      throw new Error(data.error || resp.statusText)
    }
    return data
  }

  // ============================================================
  // Auth
  // ============================================================

  /** Load token from OS Keychain on startup */
  async loadToken() {
    const token = await platformAuth.getToken()
    if (token) this.token = token
    return !!token
  }

  async login(email: string, password: string) {
    const data = await this.fetch<{ token: string; user: PlatformUser }>('/api/auth/login', {
      method: 'POST',
      body: JSON.stringify({ email, password }),
    })
    this.token = data.token
    await platformAuth.saveToken(data.token)
    return data
  }

  async loginWithGoogle(idToken: string) {
    const data = await this.fetch<{ token: string; user: PlatformUser }>('/api/auth/google', {
      method: 'POST',
      body: JSON.stringify({ idToken }),
    })
    this.token = data.token
    await platformAuth.saveToken(data.token)
    return data
  }

  async me() {
    const data = await this.fetch<PlatformUser>('/api/auth/me')
    return { user: data }
  }

  async logout() {
    try {
      await this.fetch<{ ok: boolean }>('/api/auth/logout', { method: 'POST' })
    } catch {
      // Ignore errors (e.g., network down)
    }
    this.token = null
    await platformAuth.deleteToken()
  }

  isAuthenticated() {
    return !!this.token
  }

  // ============================================================
  // Program Store (public, no auth needed)
  // ============================================================

  async getPrograms(params?: { category?: string; search?: string }) {
    const qs = params ? new URLSearchParams(params as Record<string, string>).toString() : ''
    const data = await this.fetch<{ programs: Program[] }>(`/api/store/programs${qs ? `?${qs}` : ''}`)
    return data.programs
  }

  async getProgram(id: string) {
    const data = await this.fetch<{ program: Program }>(`/api/store/programs/${id}`)
    return data.program
  }

  async getCategories() {
    const data = await this.fetch<{ categories: string[] }>('/api/store/categories')
    return data.categories
  }

  async getFeaturedPrograms() {
    const data = await this.fetch<{ programs: Program[] }>('/api/store/featured')
    return data.programs
  }

  // ============================================================
  // Deployments (auth required - for Open Appchain registration)
  // ============================================================

  async registerDeployment(data: {
    programId: string
    name: string
    chainId?: number
    rpcUrl?: string
    config?: Record<string, unknown>
  }) {
    return this.fetch<{ deployment: { id: string } }>('/api/deployments', {
      method: 'POST',
      body: JSON.stringify(data),
    })
  }

  async updateDeployment(id: string, fields: {
    description?: string
    screenshots?: string
    explorer_url?: string
    dashboard_url?: string
    social_links?: string
    l1_chain_id?: number
    network_mode?: string
    bridge_address?: string
    proposer_address?: string
    rpc_url?: string
    status?: string
  }) {
    return this.fetch<{ deployment: { id: string } }>(`/api/deployments/${id}`, {
      method: 'PUT',
      body: JSON.stringify(fields),
    })
  }

  async activateDeployment(id: string) {
    return this.fetch<{ deployment: { id: string } }>(`/api/deployments/${id}/activate`, {
      method: 'POST',
    })
  }

  async getMyDeployments() {
    const data = await this.fetch<{ deployments: Array<{ id: string; name: string; phase: string }> }>('/api/deployments')
    return data.deployments
  }

  async getPublicAppchain(id: string) {
    const data = await this.fetch<{
      appchain: {
        id: string
        name: string
        description: string | null
        explorer_url: string | null
        dashboard_url: string | null
        social_links: Record<string, string>
        screenshots: string[]
      }
    }>(`/api/store/appchains/${id}`)
    return data.appchain
  }

  // ============================================================
  // Store — Public Appchain Browsing
  // ============================================================

  async getPublicAppchains(params?: { search?: string; limit?: number; offset?: number; stack_type?: string; l1_chain_id?: string }) {
    const qs = new URLSearchParams()
    if (params?.search) qs.set('search', params.search)
    if (params?.limit) qs.set('limit', String(params.limit))
    if (params?.offset) qs.set('offset', String(params.offset))
    if (params?.stack_type) qs.set('stack_type', params.stack_type)
    if (params?.l1_chain_id) qs.set('l1_chain_id', params.l1_chain_id)
    const q = qs.toString()
    const data = await this.fetch<{ appchains: StoreAppchain[] }>(`/api/store/appchains${q ? `?${q}` : ''}`)
    return data.appchains
  }

  async getAppchainDetail(id: string) {
    const data = await this.fetch<{ appchain: StoreAppchain }>(`/api/store/appchains/${id}`)
    return data.appchain
  }

  async getAppchainReviews(id: string) {
    return this.fetch<{
      reviews: Array<{ id: string; wallet_address: string; rating: number; content: string; created_at: number }>
      reactionCounts: Record<string, number>
      userReactions: string[]
    }>(`/api/store/appchains/${id}/reviews`)
  }

  async getAppchainComments(id: string) {
    return this.fetch<{
      comments: Array<{ id: string; wallet_address: string; content: string; parent_id: string | null; created_at: number }>
      reactionCounts: Record<string, number>
      userReactions: string[]
    }>(`/api/store/appchains/${id}/comments`)
  }

  async getAppchainAnnouncements(id: string) {
    const data = await this.fetch<{
      announcements: Array<{ id: string; title: string; content: string; pinned: number; created_at: number }>
    }>(`/api/store/appchains/${id}/announcements`)
    return data.announcements
  }

  async toggleBookmark(id: string) {
    return this.fetch<{ bookmarked: boolean }>(`/api/store/appchains/${id}/bookmark`, { method: 'POST' })
  }

  async getUserBookmarks() {
    const data = await this.fetch<{ bookmarks: string[] }>('/api/store/bookmarks')
    return data.bookmarks
  }

  async rpcProxy(id: string, method: string) {
    return this.fetch<{ result: string }>(`/api/store/appchains/${id}/rpc-proxy`, {
      method: 'POST',
      body: JSON.stringify({ method, params: [] }),
    })
  }

  // ============================================================
  // Metadata Push — Push to GitHub metadata repository
  // ============================================================

  async pushMetadata(deploymentId: string) {
    return this.fetch<{ success: boolean; path?: string }>(`/api/deployments/${deploymentId}/push-metadata`, {
      method: 'POST',
    })
  }

  async deleteMetadata(deploymentId: string) {
    return this.fetch<{ success: boolean }>(`/api/deployments/${deploymentId}/delete-metadata`, {
      method: 'POST',
    })
  }

  // ============================================================
  // Screenshot Upload — Upload images to Platform server
  // ============================================================

  async uploadScreenshots(deploymentId: string, files: File[]): Promise<{ urls: string[]; screenshots: string[] }> {
    const formData = new FormData()
    for (const file of files) {
      formData.append('screenshots', file)
    }

    const headers: Record<string, string> = {}
    if (this.token) {
      headers['Authorization'] = `Bearer ${this.token}`
    }

    const resp = await window.fetch(`${this.baseUrl}/api/deployments/${deploymentId}/screenshots`, {
      method: 'POST',
      headers,
      body: formData,
    })
    const data = await resp.json()
    if (!resp.ok) throw new Error(data.error || resp.statusText)
    return data
  }
}

export const platformAPI = new PlatformAPI()
