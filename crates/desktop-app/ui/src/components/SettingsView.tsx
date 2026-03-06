import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLang, useTheme } from '../App'
import { t, langNames } from '../i18n'
import { platformAPI, type PlatformUser } from '../api/platform'
import type { Lang } from '../i18n'
import type { Theme } from '../App'

interface AiConfig {
  provider: string
  api_key: string
  model: string
}

export default function SettingsView() {
  const { lang, setLang } = useLang()
  const { theme, setTheme } = useTheme()
  const [provider, setProvider] = useState('claude')
  const [apiKey, setApiKey] = useState('')
  const [maskedKey, setMaskedKey] = useState('')
  const [model, setModel] = useState('claude-sonnet-4-6')
  const [saving, setSaving] = useState(false)
  const [saveResult, setSaveResult] = useState<{ ok: boolean; msg: string } | null>(null)

  // Platform account
  const [platformUser, setPlatformUser] = useState<PlatformUser | null>(null)
  const [platformEmail, setPlatformEmail] = useState('')
  const [platformPassword, setPlatformPassword] = useState('')
  const [platformLogging, setPlatformLogging] = useState(false)
  const [platformError, setPlatformError] = useState('')

  useEffect(() => {
    loadConfig()
    loadPlatformUser()
  }, [])

  const loadConfig = async () => {
    try {
      const cfg = await invoke<AiConfig>('get_ai_config')
      setProvider(cfg.provider)
      setMaskedKey(cfg.api_key)
      setModel(cfg.model)
    } catch {
      // defaults
    }
  }

  const loadPlatformUser = async () => {
    const hasToken = await platformAPI.loadToken()
    if (hasToken) {
      try {
        const { user } = await platformAPI.me()
        setPlatformUser(user)
      } catch {
        // Token expired or invalid
        setPlatformUser(null)
      }
    }
  }

  const handlePlatformLogin = async () => {
    if (!platformEmail.trim() || !platformPassword.trim()) return
    setPlatformLogging(true)
    setPlatformError('')
    try {
      const { user } = await platformAPI.login(platformEmail.trim(), platformPassword.trim())
      setPlatformUser(user)
      setPlatformEmail('')
      setPlatformPassword('')
    } catch (e: unknown) {
      setPlatformError(e instanceof Error ? e.message : String(e))
    } finally {
      setPlatformLogging(false)
    }
  }

  const handlePlatformLogout = async () => {
    await platformAPI.logout()
    setPlatformUser(null)
  }

  const handleSave = async () => {
    if (!apiKey.trim() && !maskedKey) return
    setSaving(true)
    setSaveResult(null)
    try {
      await invoke('save_ai_config', {
        provider,
        apiKey: apiKey.trim() || '__keep__',
        model,
      })
      // Only test if new key provided
      if (apiKey.trim()) {
        await invoke<string>('test_ai_connection')
      }
      setSaveResult({ ok: true, msg: t('settings.saved', lang) })
      setApiKey('')
      await loadConfig()
    } catch (e) {
      setSaveResult({ ok: false, msg: `${e}` })
    } finally {
      setSaving(false)
    }
  }

  const [fetchedModels, setFetchedModels] = useState<string[]>([])
  const [fetchingModels, setFetchingModels] = useState(false)

  const models: Record<string, string[]> = {
    tokamak: [],
    claude: ['claude-sonnet-4-6', 'claude-opus-4-6', 'claude-haiku-4-5-20251001'],
    gpt: ['gpt-4o', 'gpt-4o-mini'],
    gemini: ['gemini-2.5-pro', 'gemini-2.5-flash'],
  }

  const fetchModelsForProvider = async () => {
    if (!apiKey.trim()) return
    setFetchingModels(true)
    try {
      const result = await invoke<string[]>('fetch_ai_models', { provider, apiKey: apiKey.trim() })
      setFetchedModels(result)
      if (result.length > 0) setModel(result[0])
    } catch {
      setFetchedModels([])
    } finally {
      setFetchingModels(false)
    }
  }

  const handleDisconnect = async () => {
    try {
      await invoke('disconnect_ai')
      setProvider('tokamak')
      setApiKey('')
      setMaskedKey('')
      setModel('')
      setFetchedModels([])
      setSaveResult({ ok: true, msg: lang === 'ko' ? 'AI 연결이 해제되었습니다.' : 'AI disconnected.' })
    } catch (e) {
      setSaveResult({ ok: false, msg: `${e}` })
    }
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
        <h1 className="text-base font-semibold">{t('settings.title', lang)}</h1>
      </div>

      <div className="flex-1 overflow-y-auto p-4 space-y-3">
        {/* Theme */}
        <section className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 space-y-3 border border-[var(--color-border)]">
          <h2 className="text-[13px] font-medium">{t('settings.theme', lang)}</h2>
          <div className="flex gap-2">
            {([['light', t('settings.themeLight', lang)], ['dark', t('settings.themeDark', lang)]] as [Theme, string][]).map(([code, name]) => (
              <button
                key={code}
                onClick={() => setTheme(code)}
                className={`flex-1 py-2 rounded-lg text-[13px] transition-colors cursor-pointer border ${
                  theme === code
                    ? 'bg-[var(--color-accent)] text-[var(--color-accent-text)] border-[var(--color-accent)]'
                    : 'bg-[var(--color-bg-main)] border-[var(--color-border)] hover:bg-[var(--color-border)]'
                }`}
              >
                {code === 'light' ? '☀️' : '🌙'} {name}
              </button>
            ))}
          </div>
        </section>

        {/* Language */}
        <section className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 space-y-3 border border-[var(--color-border)]">
          <h2 className="text-[13px] font-medium">{t('settings.language', lang)}</h2>
          <div className="flex gap-2">
            {(Object.entries(langNames) as [Lang, string][]).map(([code, name]) => (
              <button
                key={code}
                onClick={() => setLang(code)}
                className={`flex-1 py-2 rounded-lg text-[13px] transition-colors cursor-pointer border ${
                  lang === code
                    ? 'bg-[var(--color-accent)] text-[var(--color-accent-text)] border-[var(--color-accent)]'
                    : 'bg-[var(--color-bg-main)] border-[var(--color-border)] hover:bg-[var(--color-border)]'
                }`}
              >
                {name}
              </button>
            ))}
          </div>
        </section>

        {/* Platform Account */}
        <section className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 space-y-3 border border-[var(--color-border)]">
          <h2 className="text-[13px] font-medium">
            {lang === 'ko' ? 'Platform 계정' : 'Platform Account'}
          </h2>
          {platformUser ? (
            <div className="space-y-3">
              <div className="flex items-center gap-3">
                <div className="w-10 h-10 rounded-full bg-[var(--color-accent)] flex items-center justify-center text-sm font-bold text-[var(--color-accent-text)]">
                  {platformUser.name.charAt(0).toUpperCase()}
                </div>
                <div>
                  <div className="text-[13px] font-medium">{platformUser.name}</div>
                  <div className="text-[11px] text-[var(--color-text-secondary)]">{platformUser.email}</div>
                </div>
              </div>
              <p className="text-[11px] text-[var(--color-text-secondary)]">
                {lang === 'ko'
                  ? '오픈 앱체인 등록, 프로그램 스토어 등 Platform 기능을 사용할 수 있습니다.'
                  : 'You can use Platform features like Open Appchain registration and Program Store.'}
              </p>
              <button
                onClick={handlePlatformLogout}
                className="w-full border border-[var(--color-error)] text-[var(--color-error)] hover:bg-[var(--color-error)] hover:text-white rounded-lg py-2 text-[13px] font-medium transition-colors cursor-pointer"
              >
                {lang === 'ko' ? '로그아웃' : 'Logout'}
              </button>
            </div>
          ) : (
            <div className="space-y-3">
              <p className="text-[11px] text-[var(--color-text-secondary)]">
                {lang === 'ko'
                  ? 'Platform 계정으로 로그인하면 오픈 앱체인 등록 등의 기능을 사용할 수 있습니다.'
                  : 'Login with your Platform account to register Open Appchains and more.'}
              </p>
              <div>
                <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">
                  {lang === 'ko' ? '이메일' : 'Email'}
                </label>
                <input
                  type="email"
                  value={platformEmail}
                  onChange={e => setPlatformEmail(e.target.value)}
                  placeholder="email@example.com"
                  className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)] placeholder-[var(--color-text-secondary)]"
                />
              </div>
              <div>
                <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">
                  {lang === 'ko' ? '비밀번호' : 'Password'}
                </label>
                <input
                  type="password"
                  value={platformPassword}
                  onChange={e => setPlatformPassword(e.target.value)}
                  onKeyDown={e => e.key === 'Enter' && handlePlatformLogin()}
                  placeholder="••••••••"
                  className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)] placeholder-[var(--color-text-secondary)]"
                />
              </div>
              <button
                onClick={handlePlatformLogin}
                disabled={platformLogging || !platformEmail.trim() || !platformPassword.trim()}
                className="w-full bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] disabled:opacity-40 rounded-lg py-2 text-[13px] font-medium transition-colors cursor-pointer text-[var(--color-accent-text)]"
              >
                {platformLogging
                  ? (lang === 'ko' ? '로그인 중...' : 'Logging in...')
                  : (lang === 'ko' ? '로그인' : 'Login')}
              </button>
              {platformError && (
                <p className="text-[12px] text-[var(--color-error)]">{platformError}</p>
              )}
              <p className="text-[10px] text-[var(--color-text-secondary)]">
                {lang === 'ko'
                  ? '인증 토큰은 OS 키체인에 안전하게 저장됩니다.'
                  : 'Auth token is securely stored in the OS keychain.'}
              </p>
            </div>
          )}
        </section>

        {/* AI Provider */}
        <section className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 space-y-3 border border-[var(--color-border)]">
          <h2 className="text-[13px] font-medium">{t('settings.aiProvider', lang)}</h2>
          <div>
            <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('settings.provider', lang)}</label>
            <select
              value={provider}
              onChange={e => { setProvider(e.target.value); setFetchedModels([]); setModel(models[e.target.value]?.[0] || '') }}
              className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)]"
            >
              <option value="tokamak">Tokamak AI</option>
              <option value="claude">Claude (Anthropic)</option>
              <option value="gpt">GPT (OpenAI)</option>
              <option value="gemini">Gemini (Google)</option>
            </select>
          </div>
          <div>
            <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">
              {t('settings.apiKey', lang)}
              {maskedKey && <span className="ml-2 text-[var(--color-success)]">({maskedKey})</span>}
            </label>
            <input
              type="password"
              value={apiKey}
              onChange={e => setApiKey(e.target.value)}
              placeholder={maskedKey ? t('settings.apiKeyKeep', lang) : t('settings.apiKeyPlaceholder', lang)}
              className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)] placeholder-[var(--color-text-secondary)]"
            />
            <p className="text-[10px] text-[var(--color-text-secondary)] mt-1">{t('chat.keySecure', lang)}</p>
          </div>
          <div>
            <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">
              {t('settings.model', lang)}
              {fetchedModels.length > 0 && <span className="ml-1 text-[var(--color-success)]">({fetchedModels.length})</span>}
            </label>
            <div className="flex gap-2">
              <select
                value={model}
                onChange={e => setModel(e.target.value)}
                className="flex-1 bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)]"
              >
                {(fetchedModels.length > 0 ? fetchedModels : (models[provider] || [])).map(m => (
                  <option key={m} value={m}>{m}</option>
                ))}
              </select>
              {provider !== 'claude' && (
                <button
                  onClick={fetchModelsForProvider}
                  disabled={!apiKey.trim() || fetchingModels}
                  className="px-3 py-2 rounded-lg text-[12px] bg-[var(--color-bg-main)] border border-[var(--color-border)] hover:bg-[var(--color-border)] disabled:opacity-40 cursor-pointer whitespace-nowrap"
                >
                  {fetchingModels ? '...' : t('chat.fetchModels', lang)}
                </button>
              )}
            </div>
          </div>
          <button
            onClick={handleSave}
            disabled={saving}
            className="w-full bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] disabled:opacity-40 rounded-lg py-2 text-[13px] font-medium transition-colors cursor-pointer text-[var(--color-accent-text)]"
          >
            {saving ? t('settings.testing', lang) : t('settings.saveAi', lang)}
          </button>
          {maskedKey && (
            <button
              onClick={handleDisconnect}
              className="w-full border border-[var(--color-error)] text-[var(--color-error)] hover:bg-[var(--color-error)] hover:text-white rounded-lg py-2 text-[13px] font-medium transition-colors cursor-pointer"
            >
              {t('chat.disconnect', lang)}
            </button>
          )}
          {saveResult && (
            <p className={`text-[12px] ${saveResult.ok ? 'text-[var(--color-success)]' : 'text-[var(--color-error)]'}`}>
              {saveResult.msg}
            </p>
          )}
        </section>

        {/* Node Config */}
        <section className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 space-y-3 border border-[var(--color-border)]">
          <h2 className="text-[13px] font-medium">{t('settings.nodeConfig', lang)}</h2>
          <div>
            <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('settings.binaryPath', lang)}</label>
            <input type="text" placeholder="/usr/local/bin/ethrex"
              className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)] placeholder-[var(--color-text-secondary)]" />
          </div>
        </section>
      </div>
    </div>
  )
}
