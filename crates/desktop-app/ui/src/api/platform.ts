/**
 * Platform API client for Desktop app.
 * Connects to the Tokamak Platform service (showroom) for:
 * - Program Store browsing
 * - Open Appchain registration
 * - Authentication (OAuth token reuse via OS Keychain)
 */

import { invoke } from '@tauri-apps/api/core'

const DEFAULT_PLATFORM_URL = 'https://platform.tokamak.network'

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

class PlatformAPI {
  private baseUrl: string
  private token: string | null = null

  constructor() {
    this.baseUrl = DEFAULT_PLATFORM_URL
  }

  setBaseUrl(url: string) {
    this.baseUrl = url.replace(/\/$/, '')
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
      headers,
      ...options,
    })
    const data = await resp.json()
    if (!resp.ok) throw new Error(data.error || resp.statusText)
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
    return this.fetch<{ user: PlatformUser }>('/api/auth/me')
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

  async activateDeployment(id: string) {
    return this.fetch<{ deployment: { id: string } }>(`/api/deployments/${id}/activate`, {
      method: 'POST',
    })
  }

  async getMyDeployments() {
    const data = await this.fetch<{ deployments: Array<{ id: string; name: string; phase: string }> }>('/api/deployments')
    return data.deployments
  }
}

export const platformAPI = new PlatformAPI()
