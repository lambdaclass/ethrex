import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-shell'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { useLang, useTheme } from '../App'
import { t, langNames } from '../i18n'
import { isPinataConfigured, savePinataJWT, deletePinataJWT } from '../api/ipfs'
import type { PlatformUser } from '../api/platform'
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
  const [platformLogging, setPlatformLogging] = useState(false)
  const [platformError, setPlatformError] = useState('')
  const [platformLoginUrl, setPlatformLoginUrl] = useState('')
  const [showLogoutConfirm, setShowLogoutConfirm] = useState(false)

  useEffect(() => {
    loadConfig()
    loadPlatformUser()
    loadTelegramConfig()
  }, [])

  const loadConfig = async () => {
    try {
      const cfg = await invoke<AiConfig>('get_ai_config')
      // If provider is 'tokamak' (removed from UI) but has a key, infer actual provider
      let inferredProvider: string
      if (cfg.provider === 'tokamak' && cfg.api_key) {
        if (cfg.api_key.startsWith('sk-a')) inferredProvider = 'claude'
        else if (cfg.api_key.startsWith('sk-p')) inferredProvider = 'gpt'
        else if (cfg.api_key.startsWith('AIza')) inferredProvider = 'gemini'
        else inferredProvider = 'claude'
      } else if (cfg.provider === 'tokamak') {
        inferredProvider = 'claude'
      } else {
        inferredProvider = cfg.provider
      }
      setProvider(inferredProvider)
      setMaskedKey(cfg.api_key)
      const models: Record<string, string[]> = {
        claude: ['claude-sonnet-4-6', 'claude-opus-4-6', 'claude-haiku-4-5-20251001'],
        gpt: ['gpt-4o', 'gpt-4o-mini'],
        gemini: ['gemini-2.5-pro', 'gemini-2.5-flash'],
      }
      setModel(cfg.model || models[inferredProvider]?.[0] || '')
    } catch {
      // defaults
    }
  }

  const loadPlatformUser = async () => {
    try {
      const user = await invoke<PlatformUser>('get_platform_user')
      setPlatformUser(user)
    } catch {
      setPlatformUser(null)
    }
  }

  const loadTelegramConfig = async () => {
    try {
      const cfg = await invoke<{ bot_token: string; allowed_chat_ids: string; enabled: boolean; system_alerts_enabled?: boolean }>('get_telegram_config')
      setTgMaskedToken(cfg.bot_token || '')
      setTgChatIds(cfg.allowed_chat_ids)
      setTgEnabled(cfg.enabled)
      setTgSystemAlerts(cfg.system_alerts_enabled ?? true)
      const running = await invoke<boolean>('get_telegram_bot_status')
      setTgBotRunning(running)
    } catch {
      // defaults
    }
  }

  const handleTelegramToggle = async (enabled: boolean) => {
    setTgToggling(true)
    setTgResult(null)
    try {
      await invoke<boolean>('toggle_telegram_bot', { enabled })
      setTgEnabled(enabled)
      setTgBotRunning(enabled)
      setTgResult({ ok: true, msg: enabled
        ? (lang === 'ko' ? 'Telegram Bot이 시작되었습니다.' : 'Telegram Bot started.')
        : (lang === 'ko' ? 'Telegram Bot이 중지되었습니다.' : 'Telegram Bot stopped.')
      })
    } catch (e) {
      setTgResult({ ok: false, msg: `${e}` })
    } finally {
      setTgToggling(false)
    }
  }

  const [tgAlertsToggling, setTgAlertsToggling] = useState(false)

  const handleSystemAlertsToggle = async (enabled: boolean) => {
    if (tgAlertsToggling) return
    setTgAlertsToggling(true)
    try {
      await invoke<boolean>('toggle_system_alerts', { enabled })
      setTgSystemAlerts(enabled)
    } catch (e) {
      setTgResult({ ok: false, msg: `${e}` })
    } finally {
      setTgAlertsToggling(false)
    }
  }

  const handleTelegramSave = async () => {
    setTgSaving(true)
    setTgResult(null)
    try {
      const tokenToSend = tgToken.trim() ? tgToken.trim() : (tgMaskedToken ? '__keep__' : '')
      await invoke('save_telegram_config', {
        botToken: tokenToSend,
        allowedChatIds: tgChatIds.trim(),
      })
      setTgResult({ ok: true, msg: t('settings.telegramSaved', lang) })
      setTgToken('')
      await loadTelegramConfig()
    } catch (e) {
      setTgResult({ ok: false, msg: `${e}` })
    } finally {
      setTgSaving(false)
    }
  }

  const handlePlatformLogin = async () => {
    if (platformLogging) return
    setPlatformLogging(true)
    setPlatformError('')
    setPlatformLoginUrl('')
    try {
      const result = await invoke<{ login_url: string; code: string; code_verifier: string }>('start_platform_login')
      setPlatformLoginUrl(result.login_url)

      const token = await invoke<string>('poll_platform_login', {
        code: result.code,
        codeVerifier: result.code_verifier,
      })
      if (token) {
        await loadPlatformUser()
        setPlatformLoginUrl('')
      }
    } catch (e: unknown) {
      const errorStr = e instanceof Error ? e.message : String(e)
      if (errorStr.includes('login_timeout')) {
        setPlatformError(lang === 'ko' ? '로그인 시간이 초과되었습니다. 다시 시도하세요.' : 'Login timed out. Please try again.')
      } else {
        setPlatformError(errorStr)
      }
      setPlatformLoginUrl('')
    } finally {
      setPlatformLogging(false)
    }
  }

  const handlePlatformLogout = async () => {
    try {
      await invoke('delete_platform_token')
      setPlatformUser(null)
      setShowLogoutConfirm(false)
    } catch (e) {
      console.error('Logout failed:', e)
      setPlatformError(`${e}`)
    }
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

  // Telegram Bot
  const [tgToken, setTgToken] = useState('')
  const [tgMaskedToken, setTgMaskedToken] = useState('')
  const [tgChatIds, setTgChatIds] = useState('')
  const [tgEnabled, setTgEnabled] = useState(false)
  const [tgBotRunning, setTgBotRunning] = useState(false)
  const [tgToggling, setTgToggling] = useState(false)
  const [tgSaving, setTgSaving] = useState(false)
  const [tgResult, setTgResult] = useState<{ ok: boolean; msg: string } | null>(null)
  const [tgSystemAlerts, setTgSystemAlerts] = useState(true)

  // Pinata (IPFS)
  const [pinataConfigured, setPinataConfigured] = useState(false)
  const [pinataKey, setPinataKey] = useState('')
  const [pinataSaving, setPinataSaving] = useState(false)
  const [pinataResult, setPinataResult] = useState<{ ok: boolean; msg: string } | null>(null)

  useEffect(() => { isPinataConfigured().then(setPinataConfigured) }, [])

  const handlePinataSave = async () => {
    if (!pinataKey.trim()) return
    setPinataSaving(true)
    setPinataResult(null)
    try {
      await savePinataJWT(pinataKey.trim())
      setPinataConfigured(true)
      setPinataKey('')
      setPinataResult({ ok: true, msg: lang === 'ko' ? '저장됨' : 'Saved' })
    } catch (e) {
      setPinataResult({ ok: false, msg: `${e}` })
    } finally {
      setPinataSaving(false)
    }
  }

  const handlePinataDelete = async () => {
    try {
      await deletePinataJWT()
      setPinataConfigured(false)
      setPinataResult({ ok: true, msg: lang === 'ko' ? '삭제됨' : 'Removed' })
    } catch (e) {
      setPinataResult({ ok: false, msg: `${e}` })
    }
  }

  // Local server (L2 Manager)
  const [serverStatus, setServerStatus] = useState<{ running: boolean; healthy: boolean } | null>(null)
  const [serverRestarting, setServerRestarting] = useState(false)
  const [serverMsg, setServerMsg] = useState<{ ok: boolean; msg: string } | null>(null)

  const loadServerStatus = async () => {
    try {
      const st = await invoke<{ running: boolean; healthy: boolean; url: string; port: number }>('get_local_server_status')
      setServerStatus(st)
    } catch {
      setServerStatus({ running: false, healthy: false })
    }
  }

  useEffect(() => {
    loadServerStatus()
    const iv = setInterval(loadServerStatus, 5000)
    return () => clearInterval(iv)
  }, [])

  const showServerMsg = (msg: { ok: boolean; msg: string }) => {
    setServerMsg(msg)
    setTimeout(() => setServerMsg(null), 3000)
  }

  const handleServerRestart = async () => {
    setServerRestarting(true)
    setServerMsg(null)
    try {
      await invoke('stop_local_server')
      await new Promise(r => setTimeout(r, 1000))
      await invoke<string>('start_local_server')
      await loadServerStatus()
      showServerMsg({ ok: true, msg: lang === 'ko' ? '서버가 재시작되었습니다.' : 'Server restarted.' })
    } catch (e) {
      showServerMsg({ ok: false, msg: `${e}` })
    } finally {
      setServerRestarting(false)
    }
  }

  const handleServerStop = async () => {
    setServerRestarting(true)
    setServerMsg(null)
    try {
      await invoke('stop_local_server')
      // Wait for process to fully terminate before checking status
      await new Promise(r => setTimeout(r, 1000))
      await loadServerStatus()
      showServerMsg({ ok: true, msg: lang === 'ko' ? '서버가 중지되었습니다.' : 'Server stopped.' })
    } catch (e) {
      showServerMsg({ ok: false, msg: `${e}` })
    } finally {
      setServerRestarting(false)
    }
  }

  const handleServerStart = async () => {
    setServerRestarting(true)
    setServerMsg(null)
    try {
      await invoke<string>('start_local_server')
      // Wait for server to become healthy before checking status
      await new Promise(r => setTimeout(r, 2000))
      await loadServerStatus()
      showServerMsg({ ok: true, msg: lang === 'ko' ? '서버가 시작되었습니다.' : 'Server started.' })
    } catch (e) {
      showServerMsg({ ok: false, msg: `${e}` })
    } finally {
      setServerRestarting(false)
    }
  }

  const [fetchedModels, setFetchedModels] = useState<string[]>([])
  const [fetchingModels, setFetchingModels] = useState(false)

  const models: Record<string, string[]> = {
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
      setProvider('claude')
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
                  ? '앱체인을 공개 앱체인으로 퍼블리시할 수 있고, Tokamak AI(토큰 한도만큼)를 사용할 수 있습니다.'
                  : 'You can publish appchains and use Tokamak AI (within token limits).'}
              </p>
              {!showLogoutConfirm ? (
                <button
                  onClick={() => setShowLogoutConfirm(true)}
                  className="w-full border border-[var(--color-error)] text-[var(--color-error)] hover:bg-[var(--color-error)] hover:text-white rounded-lg py-2 text-[13px] font-medium transition-colors cursor-pointer"
                >
                  {lang === 'ko' ? '로그아웃' : 'Logout'}
                </button>
              ) : (
                <div className="space-y-2">
                  <p className="text-[12px] text-[var(--color-error)] font-medium">
                    {lang === 'ko'
                      ? '로그아웃하면 Tokamak AI 연결도 해제됩니다.'
                      : 'Logging out will also disconnect Tokamak AI.'}
                  </p>
                  <div className="flex gap-2">
                    <button
                      onClick={handlePlatformLogout}
                      className="flex-1 bg-[var(--color-error)] text-white rounded-lg py-2 text-[13px] font-medium cursor-pointer"
                    >
                      {lang === 'ko' ? '로그아웃 확인' : 'Confirm Logout'}
                    </button>
                    <button
                      onClick={() => setShowLogoutConfirm(false)}
                      className="flex-1 border border-[var(--color-border)] rounded-lg py-2 text-[13px] cursor-pointer hover:bg-[var(--color-border)]"
                    >
                      {lang === 'ko' ? '취소' : 'Cancel'}
                    </button>
                  </div>
                </div>
              )}
            </div>
          ) : (
            <div className="space-y-3">
              <p className="text-[11px] text-[var(--color-text-secondary)]">
                {lang === 'ko'
                  ? 'Platform 계정으로 로그인하면 앱체인을 공개 앱체인으로 퍼블리시할 수 있고, Tokamak AI(토큰 한도만큼)를 사용할 수 있습니다.'
                  : 'Login with your Platform account to publish appchains and use Tokamak AI (within token limits).'}
              </p>
              <button
                onClick={handlePlatformLogin}
                disabled={platformLogging}
                className="w-full bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] disabled:opacity-40 rounded-lg py-2 text-[13px] font-medium transition-colors cursor-pointer text-[var(--color-accent-text)]"
              >
                {platformLogging
                  ? (lang === 'ko' ? '로그인 대기 중...' : 'Waiting for login...')
                  : (lang === 'ko' ? '브라우저에서 로그인' : 'Login in Browser')}
              </button>
              {platformLoginUrl && (
                <div className="space-y-1">
                  <p className="text-[11px] text-[var(--color-text-secondary)]">
                    {lang === 'ko'
                      ? '브라우저가 열리지 않으면 아래 링크를 클릭하세요:'
                      : 'If browser did not open, click the link below:'}
                  </p>
                  <a
                    href="#"
                    onClick={e => { e.preventDefault(); open(platformLoginUrl) }}
                    className="text-[12px] text-[var(--color-accent)] underline cursor-pointer break-all block"
                  >
                    {lang === 'ko' ? '🔗 로그인 페이지 열기' : '🔗 Open login page'}
                  </a>
                </div>
              )}
              {platformError && (
                <p className="text-[12px] text-[var(--color-error)]">{platformError}</p>
              )}
              <p className="text-[10px] text-[var(--color-text-secondary)]">
                {lang === 'ko'
                  ? '인증 토큰은 안전하게 저장됩니다.'
                  : 'Auth token is stored securely.'}
              </p>
            </div>
          )}
        </section>

        {/* AI Provider */}
        <section className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 space-y-3 border border-[var(--color-border)]">
          <div className="flex items-center justify-between">
            <h2 className="text-[13px] font-medium">{t('settings.aiProvider', lang)}</h2>
            {maskedKey ? (
              <span className="text-[11px] font-medium text-[var(--color-success)] flex items-center gap-1">
                <span className="inline-block w-1.5 h-1.5 rounded-full bg-[var(--color-success)]" />
                {lang === 'ko' ? '연결됨' : 'Connected'}
              </span>
            ) : (
              <span className="text-[11px] font-medium text-[var(--color-text-secondary)] flex items-center gap-1">
                <span className="inline-block w-1.5 h-1.5 rounded-full bg-[var(--color-text-secondary)]" />
                {lang === 'ko' ? '미설정' : 'Not configured'}
              </span>
            )}
          </div>
          <div>
            <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('settings.provider', lang)}</label>
            <select
              value={provider}
              onChange={e => { setProvider(e.target.value); setFetchedModels([]); setModel(models[e.target.value]?.[0] || '') }}
              className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)]"
            >
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
          {maskedKey && (
            <p className="text-[10px] text-[var(--color-text-secondary)] mt-1">
              {lang === 'ko'
                ? 'L2 매니저의 AI Deploy에서도 이 설정을 사용할 수 있습니다.'
                : 'This setting can also be used in L2 Manager AI Deploy.'}
            </p>
          )}
        </section>

        {/* IPFS (Pinata) */}
        <section className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 space-y-3 border border-[var(--color-border)]">
          <h2 className="text-[13px] font-medium">
            {lang === 'ko' ? 'IPFS 설정 (Pinata)' : 'IPFS Settings (Pinata)'}
          </h2>
          <p className="text-[11px] text-[var(--color-text-secondary)]">
            {lang === 'ko'
              ? '스크린샷과 메타데이터를 IPFS에 업로드하려면 Pinata JWT가 필요합니다.'
              : 'Pinata JWT is required to upload screenshots and metadata to IPFS.'}
          </p>
          {pinataConfigured && (
            <div className="flex items-center gap-2">
              <span className="text-[11px] text-[var(--color-success)] font-medium">
                {lang === 'ko' ? '● 설정됨' : '● Configured'}
              </span>
              <button
                onClick={handlePinataDelete}
                className="text-[10px] text-[var(--color-error)] hover:underline cursor-pointer"
              >
                {lang === 'ko' ? '삭제' : 'Remove'}
              </button>
            </div>
          )}
          <div>
            <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">Pinata JWT</label>
            <input
              type="password"
              value={pinataKey}
              onChange={e => setPinataKey(e.target.value)}
              placeholder={pinataConfigured ? (lang === 'ko' ? '새 키로 변경...' : 'Enter new key to change...') : 'eyJhbGciOiJIUzI1NiIs...'}
              className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)] placeholder-[var(--color-text-secondary)]"
            />
          </div>
          <button
            onClick={handlePinataSave}
            disabled={pinataSaving || !pinataKey.trim()}
            className="w-full bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] disabled:opacity-40 rounded-lg py-2 text-[13px] font-medium transition-colors cursor-pointer text-[var(--color-accent-text)]"
          >
            {pinataSaving ? '...' : (lang === 'ko' ? '저장' : 'Save')}
          </button>
          {pinataResult && (
            <p className={`text-[12px] ${pinataResult.ok ? 'text-[var(--color-success)]' : 'text-[var(--color-error)]'}`}>
              {pinataResult.msg}
            </p>
          )}
        </section>

        {/* Telegram Bot */}
        <section className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 space-y-3 border border-[var(--color-border)]">
          <h2 className="text-[13px] font-medium">{t('settings.telegram', lang)}</h2>
          <p className="text-[11px] text-[var(--color-text-secondary)]">
            {t('settings.telegramDesc', lang)}
          </p>
          <div className="flex items-center gap-3">
            <label className={`relative inline-flex items-center ${tgToggling ? 'opacity-50 pointer-events-none' : 'cursor-pointer'}`}>
              <input
                type="checkbox"
                checked={tgEnabled}
                onChange={e => handleTelegramToggle(e.target.checked)}
                disabled={tgToggling || (!tgMaskedToken && !tgToken.trim())}
                className="sr-only peer"
              />
              <div className="w-9 h-5 bg-[var(--color-border)] peer-focus:outline-none rounded-full peer peer-checked:bg-[var(--color-accent)] after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:after:translate-x-full"></div>
            </label>
            <span className="text-[12px]">{t('settings.telegramEnabled', lang)}</span>
            <span className={`text-[11px] font-medium ${tgBotRunning ? 'text-[var(--color-success)]' : 'text-[var(--color-text-secondary)]'}`}>
              {tgBotRunning
                ? (lang === 'ko' ? '● 실행 중' : '● Running')
                : (lang === 'ko' ? '○ 중지됨' : '○ Stopped')}
            </span>
          </div>
          <div className="flex items-center gap-3">
            <label className={`relative inline-flex items-center ${!tgEnabled ? 'opacity-50 pointer-events-none' : 'cursor-pointer'}`}>
              <input
                type="checkbox"
                checked={tgSystemAlerts}
                onChange={e => handleSystemAlertsToggle(e.target.checked)}
                disabled={!tgEnabled || tgAlertsToggling}
                aria-label="System alerts toggle"
                className="sr-only peer"
              />
              <div className="w-9 h-5 bg-[var(--color-border)] peer-focus:outline-none rounded-full peer peer-checked:bg-[var(--color-accent)] after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:after:translate-x-full"></div>
            </label>
            <span className="text-[12px]">{t('settings.systemAlerts', lang)}</span>
          </div>
          <div>
            <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">
              {t('settings.telegramToken', lang)}
              {tgMaskedToken && <span className="ml-2 text-[var(--color-success)]">({tgMaskedToken})</span>}
            </label>
            <input
              type="password"
              value={tgToken}
              onChange={e => setTgToken(e.target.value)}
              placeholder={tgMaskedToken ? (lang === 'ko' ? '변경하려면 새 토큰을 입력하세요...' : 'Enter new token to change...') : t('settings.telegramTokenPlaceholder', lang)}
              className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)] placeholder-[var(--color-text-secondary)]"
            />
            <p className="text-[10px] text-[var(--color-text-secondary)] mt-1">{t('settings.telegramHowTo', lang)}</p>
          </div>
          <div>
            <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">
              {t('settings.telegramChatIds', lang)}
            </label>
            <input
              type="text"
              value={tgChatIds}
              onChange={e => setTgChatIds(e.target.value)}
              placeholder={t('settings.telegramChatIdsPlaceholder', lang)}
              className="w-full bg-[var(--color-bg-main)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)] placeholder-[var(--color-text-secondary)]"
            />
          </div>
          <button
            onClick={handleTelegramSave}
            disabled={tgSaving}
            className="w-full bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] disabled:opacity-40 rounded-lg py-2 text-[13px] font-medium transition-colors cursor-pointer text-[var(--color-accent-text)]"
          >
            {tgSaving ? '...' : t('settings.telegramSave', lang)}
          </button>
          {tgResult && (
            <p className={`text-[12px] ${tgResult.ok ? 'text-[var(--color-success)]' : 'text-[var(--color-error)]'}`}>
              {tgResult.msg}
            </p>
          )}
        </section>

        {/* L2 Manager Server */}
        <section className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 space-y-3 border border-[var(--color-border)]">
          <div className="flex items-center justify-between">
            <h2 className="text-[13px] font-medium">{lang === 'ko' ? 'L2 매니저 서버' : 'L2 Manager Server'}</h2>
            {serverStatus && (
              <span className={`text-[11px] font-medium flex items-center gap-1 ${serverStatus.healthy ? 'text-[var(--color-success)]' : serverStatus.running ? 'text-[var(--color-warning)]' : 'text-[var(--color-error)]'}`}>
                <span className={`inline-block w-1.5 h-1.5 rounded-full ${serverStatus.healthy ? 'bg-[var(--color-success)]' : serverStatus.running ? 'bg-[var(--color-warning)]' : 'bg-[var(--color-error)]'}`} />
                {serverStatus.healthy ? (lang === 'ko' ? '정상' : 'Healthy') : serverStatus.running ? (lang === 'ko' ? '시작 중' : 'Starting') : (lang === 'ko' ? '중지됨' : 'Stopped')}
              </span>
            )}
          </div>
          <p className="text-[11px] text-[var(--color-text-secondary)]">
            {lang === 'ko'
              ? '배포 관리를 담당하는 로컬 서버입니다. 문제가 있으면 재시작하세요.'
              : 'Local server for deployment management. Restart if experiencing issues.'}
          </p>
          <div className="flex gap-2">
            {serverStatus?.running ? (
              <>
                <button
                  onClick={async () => {
                    try {
                      const baseUrl = await invoke<string>('open_deployment_ui')
                      const existing = await WebviewWindow.getByLabel('deploy-manager')
                      if (existing) { await existing.show(); await existing.setFocus(); return }
                      new WebviewWindow('deploy-manager', {
                        url: baseUrl, title: 'Tokamak L2 Manager',
                        width: 1100, height: 800, minWidth: 800, minHeight: 600, center: true,
                      })
                    } catch (e) { console.error('Failed to open manager:', e) }
                  }}
                  className="flex-1 bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] rounded-lg py-2 text-[13px] font-medium transition-colors cursor-pointer text-[var(--color-accent-text)]"
                >
                  {lang === 'ko' ? '열기' : 'Open'}
                </button>
                <button
                  onClick={handleServerRestart}
                  disabled={serverRestarting}
                  className="px-4 py-2 rounded-lg text-[13px] font-medium border border-[var(--color-border)] hover:bg-[var(--color-bg-main)] transition-colors cursor-pointer disabled:opacity-40"
                >
                  {serverRestarting ? '...' : (lang === 'ko' ? '재시작' : 'Restart')}
                </button>
                <button
                  onClick={handleServerStop}
                  disabled={serverRestarting}
                  className="px-4 py-2 rounded-lg text-[13px] font-medium border border-[var(--color-error)] text-[var(--color-error)] hover:bg-[var(--color-error)] hover:text-white transition-colors cursor-pointer disabled:opacity-40"
                >
                  {lang === 'ko' ? '중지' : 'Stop'}
                </button>
              </>
            ) : (
              <button
                onClick={handleServerStart}
                disabled={serverRestarting}
                className="flex-1 bg-[var(--color-success)] hover:opacity-80 disabled:opacity-40 rounded-lg py-2 text-[13px] font-medium transition-colors cursor-pointer text-white"
              >
                {serverRestarting ? '...' : (lang === 'ko' ? '시작' : 'Start')}
              </button>
            )}
          </div>
          {serverMsg && (
            <p className={`text-[12px] ${serverMsg.ok ? 'text-[var(--color-success)]' : 'text-[var(--color-error)]'}`}>
              {serverMsg.msg}
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
