import { useState, useRef, useEffect, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-shell'
import { useLang } from '../App'
import { t } from '../i18n'
import type { ViewType } from '../App'
import type { Lang } from '../i18n'
import type { NetworkMode } from './CreateL2Wizard'

interface Message {
  role: 'user' | 'assistant'
  content: string
}

interface ChatAction {
  action: string
  params: Record<string, string>
}

function parseActions(text: string): { cleanText: string; actions: ChatAction[] } {
  const actionRegex = /\[ACTION:(\w+)(?::([^\]]*))?\]/g
  const actions: ChatAction[] = []
  let match: RegExpExecArray | null
  while ((match = actionRegex.exec(text)) !== null) {
    const params: Record<string, string> = {}
    if (match[2]) {
      match[2].split(',').forEach(p => {
        const [k, v] = p.split('=')
        if (k && v) params[k.trim()] = v.trim()
      })
    }
    actions.push({ action: match[1], params })
  }
  const cleanText = text.replace(actionRegex, '').trim()
  return { cleanText, actions }
}

const actionLabels: Record<string, Record<Lang, string>> = {
  navigate: { ko: '이동', en: 'Go to' },
  create_appchain: { ko: '앱체인 만들기', en: 'Create Appchain' },
  stop_appchain: { ko: '앱체인 중지', en: 'Stop Appchain' },
  open_appchain: { ko: '앱체인 보기', en: 'View Appchain' },
}

const viewLabels: Record<string, Record<Lang, string>> = {
  home: { ko: '홈', en: 'Home' },
  myl2: { ko: '내 앱체인', en: 'My Appchains' },
  store: { ko: '프로그램 스토어', en: 'Program Store' },
  openl2: { ko: '오픈 앱체인', en: 'Open Appchain' },
  wallet: { ko: '지갑', en: 'Wallet' },
  dashboard: { ko: '대시보드', en: 'Dashboard' },
  settings: { ko: '설정', en: 'Settings' },
}

function actionLabel(action: ChatAction, lang: Lang): string {
  const base = actionLabels[action.action]?.[lang] || action.action
  if (action.action === 'navigate' && action.params.view) {
    const view = viewLabels[action.params.view]?.[lang] || action.params.view
    return `${base}: ${view}`
  }
  if (action.action === 'create_appchain' && action.params.network) {
    return `${base} (${action.params.network})`
  }
  return base
}

interface ChatViewProps {
  onNavigate?: (view: ViewType) => void
  onCreateWithNetwork?: (network: NetworkMode) => void
  isVisible?: boolean
}

interface AiConfig {
  provider: string
  api_key: string
  model: string
}

type AiMode = 'tokamak' | 'custom'

interface TokenUsage {
  date: string
  used: number
  limit: number
}

export default function ChatView({ onNavigate, onCreateWithNetwork, isVisible }: ChatViewProps) {
  const { lang } = useLang()
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState('')
  const [loading, setLoading] = useState(false)
  const [aiMode, setAiMode] = useState<AiMode>('tokamak')
  const [tokenUsage, setTokenUsage] = useState<TokenUsage | null>(null)

  // Custom AI state
  const [hasKey, setHasKey] = useState<boolean | null>(null)
  const [keyInput, setKeyInput] = useState('')
  const [savingKey, setSavingKey] = useState(false)
  const [config, setConfig] = useState<AiConfig | null>(null)
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null)
  const [selectedModel, setSelectedModel] = useState('claude-sonnet-4-6')
  const [fetchedModels, setFetchedModels] = useState<string[]>([])
  const [fetchingModels, setFetchingModels] = useState(false)
  const [showDisconnect, setShowDisconnect] = useState(false)
  const [setupError, setSetupError] = useState('')
  const messagesEndRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    loadAiMode()
  }, [])

  // Refresh login/usage state when chat becomes visible
  // (e.g., after logging in/out from Settings)
  const prevVisible = useRef(false)
  const platformUserRef = useRef(platformUser)
  platformUserRef.current = platformUser

  useEffect(() => {
    if (isVisible && !prevVisible.current && aiMode === 'tokamak') {
      refreshTokamakState()
    }
    prevVisible.current = !!isVisible
  }, [isVisible, aiMode])

  const refreshTokamakState = async () => {
    const wasLoggedIn = !!platformUserRef.current
    try {
      const user = await invoke<{ name: string; email: string }>('get_platform_user')
      if (!platformUserRef.current && user) {
        setMessages(prev => [...prev, {
          role: 'assistant',
          content: lang === 'ko'
            ? `✅ ${user.name} (${user.email})으로 로그인되었습니다.`
            : `✅ Logged in as ${user.name} (${user.email}).`
        }])
      }
      setPlatformUser(user)
      const usage = await invoke<TokenUsage>('get_token_usage')
      setTokenUsage(usage)
    } catch {
      if (wasLoggedIn) {
        setMessages(prev => [...prev, {
          role: 'assistant',
          content: lang === 'ko'
            ? '🔒 로그아웃되었습니다. Tokamak AI를 사용하려면 다시 로그인하세요.'
            : '🔒 Logged out. Please log in again to use Tokamak AI.'
        }])
      }
      setPlatformUser(null)
      setTokenUsage(null)
    }
  }

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages])

  const loadAiMode = async () => {
    try {
      const mode = await invoke<AiMode>('get_ai_mode')
      setAiMode(mode)
      if (mode === 'tokamak') {
        await loadTokamakUsage()
      } else {
        await checkApiKey()
      }
    } catch {
      // Default to tokamak mode
      setAiMode('tokamak')
      if (messages.length === 0) {
        setMessages([{ role: 'assistant', content: t('chat.welcome.tokamak', lang) }])
      }
    }
  }

  const loadTokamakUsage = async () => {
    try {
      const usage = await invoke<TokenUsage>('get_token_usage')
      setTokenUsage(usage)
      try {
        const user = await invoke<{ name: string; email: string }>('get_platform_user')
        setPlatformUser(user)
      } catch {
        setPlatformUser(null)
      }
      setMessages(prev => prev.length === 0
        ? [{ role: 'assistant', content: t('chat.welcome.tokamak', lang) }]
        : prev
      )
    } catch (e) {
      const errorStr = `${e}`
      if (errorStr.includes('login_required')) {
        setPlatformUser(null)
        setMessages(prev => prev.length === 0
          ? [{ role: 'assistant', content: t('chat.loginRequired', lang) }]
          : prev
        )
      }
    }
  }

  const switchMode = async (mode: AiMode) => {
    try {
      await invoke('set_ai_mode', { mode })
      setAiMode(mode)
      setMessages([])
      setShowDisconnect(false)
      if (mode === 'tokamak') {
        await loadTokamakUsage()
      } else {
        await checkApiKey()
      }
    } catch {}
  }

  const checkApiKey = async () => {
    try {
      const result = await invoke<boolean>('has_ai_key')
      setHasKey(result)
      if (result) {
        const cfg = await invoke<AiConfig>('get_ai_config')
        setConfig(cfg)
        if (messages.length === 0) {
          setMessages([{ role: 'assistant', content: t('chat.welcome.connected', lang) }])
        }
      }
    } catch {
      setHasKey(false)
    }
  }

  const fetchModels = async (provider: string, key: string) => {
    if (!key.trim()) return
    setFetchingModels(true)
    setSetupError('')
    try {
      const models = await invoke<string[]>('fetch_ai_models', { provider, apiKey: key.trim() })
      setFetchedModels(models)
      if (models.length > 0) setSelectedModel(models[0])
    } catch (e) {
      setFetchedModels([])
      setSetupError(`${e}`)
    } finally {
      setFetchingModels(false)
    }
  }

  const saveApiKey = async () => {
    if (!keyInput.trim()) return
    setSavingKey(true)
    setSetupError('')
    try {
      await invoke('save_ai_config', {
        provider: selectedProvider || 'claude',
        apiKey: keyInput.trim(),
        model: selectedModel,
      })
      const response = await invoke<string>('test_ai_connection')
      setHasKey(true)
      const cfg = await invoke<AiConfig>('get_ai_config')
      setConfig(cfg)
      setMessages([{ role: 'assistant', content: response }])
      setKeyInput('')
      setSetupError('')
    } catch (e) {
      setSetupError(`${t('chat.keyError', lang)}\n${e}`)
    } finally {
      setSavingKey(false)
    }
  }

  const [loggingIn, setLoggingIn] = useState(false)
  const [platformUser, setPlatformUser] = useState<{ name: string; email: string } | null>(null)

  const handlePlatformLogin = async () => {
    if (loggingIn) return
    setLoggingIn(true)
    try {
      // Step 1: Get login URL
      const result = await invoke<{ login_url: string; code: string; code_verifier: string }>('start_platform_login')
      setMessages(prev => [...prev, {
        role: 'assistant',
        content: lang === 'ko'
          ? `아래 링크를 클릭하여 로그인하세요:\n__link__${result.login_url}`
          : `Click the link below to login:\n__link__${result.login_url}`
      }])

      // Step 2: Poll for token
      const token = await invoke<string>('poll_platform_login', {
        code: result.code,
        codeVerifier: result.code_verifier,
      })
      if (token) {
        // poll_platform_login already stored the token in AiProvider memory,
        // so get_platform_user and get_token_usage will use the correct token.
        let userName = ''
        let userEmail = ''
        try {
          const user = await invoke<{ name: string; email: string }>('get_platform_user')
          setPlatformUser(user)
          userName = user.name
          userEmail = user.email
        } catch {
          // user info fetch failed
        }
        let usageInfo = ''
        try {
          const usage = await invoke<TokenUsage>('get_token_usage')
          setTokenUsage(usage)
          usageInfo = lang === 'ko'
            ? `\n📊 토큰 사용량: ${usage.used.toLocaleString()} / ${usage.limit.toLocaleString()}`
            : `\n📊 Token usage: ${usage.used.toLocaleString()} / ${usage.limit.toLocaleString()}`
        } catch {
          // usage fetch failed
        }
        const loginMsg = userName
          ? (lang === 'ko'
              ? `✅ ${userName} (${userEmail})으로 로그인되었습니다.${usageInfo}`
              : `✅ Logged in as ${userName} (${userEmail}).${usageInfo}`)
          : t('chat.loginSuccess', lang)
        setMessages(prev => [...prev, { role: 'assistant', content: loginMsg }])
      }
    } catch (e) {
      const errorStr = `${e}`
      if (errorStr.includes('login_timeout')) {
        setMessages(prev => [...prev, { role: 'assistant', content: t('chat.loginTimeout', lang) }])
      } else {
        console.error('Login error:', e)
        setMessages(prev => [...prev, { role: 'assistant', content: t('chat.loginError', lang) }])
      }
    } finally {
      setLoggingIn(false)
    }
  }

  const executeAction = useCallback((action: ChatAction) => {
    switch (action.action) {
      case 'navigate':
        if (action.params.view && onNavigate) {
          onNavigate(action.params.view as ViewType)
        }
        break
      case 'create_appchain':
        if (onCreateWithNetwork) {
          onCreateWithNetwork((action.params.network || 'local') as NetworkMode)
        }
        break
      case 'stop_appchain':
        if (action.params.id) {
          invoke('stop_appchain', { id: action.params.id }).catch(console.error)
        }
        break
      case 'open_appchain':
        if (action.params.id && onNavigate) {
          onNavigate('myl2')
        }
        break
    }
  }, [onNavigate, onCreateWithNetwork])

  const sendMessage = async () => {
    if (!input.trim() || loading) return
    const userMsg: Message = { role: 'user', content: input }
    const newMessages = [...messages, userMsg]
    setMessages(newMessages)
    setInput('')
    setLoading(true)
    try {
      const context = await invoke<Record<string, unknown>>('get_chat_context')
      const contextJson = JSON.stringify(context)

      const welcomeKeys = [t('chat.welcome.connected', lang), t('chat.welcome.tokamak', lang)]
      const apiMessages = newMessages
        .filter(m => !welcomeKeys.includes(m.content))
        .map(m => ({ role: m.role, content: m.content }))
      const response = await invoke<{ role: string; content: string }>('send_chat_message', {
        messages: apiMessages,
        context: contextJson,
      })
      setMessages(prev => [...prev, { role: 'assistant', content: response.content }])

      // Refresh token usage after Tokamak AI call
      if (aiMode === 'tokamak') {
        const usage = await invoke<TokenUsage>('get_token_usage')
        setTokenUsage(usage)
      }
    } catch (e) {
      const errorStr = `${e}`
      if (errorStr.includes('login_required')) {
        setMessages(prev => [...prev, { role: 'assistant', content: t('chat.loginRequired', lang) }])
      } else if (errorStr.includes('daily_limit_exceeded')) {
        setMessages(prev => [...prev, { role: 'assistant', content: t('chat.dailyLimitExceeded', lang) }])
      } else {
        setMessages(prev => [...prev, { role: 'assistant', content: `Error: ${e}` }])
      }
    } finally {
      setLoading(false)
    }
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendMessage() }
  }

  // Loading state
  if (aiMode === 'custom' && hasKey === null) {
    return (
      <div className="flex flex-col h-full bg-[var(--color-bg-main)] items-center justify-center">
        <div className="w-6 h-6 border-2 border-[var(--color-accent)] border-t-transparent rounded-full animate-spin" />
      </div>
    )
  }

  // Provider info (for custom mode setup)
  const providers: Record<string, { name: string; icon: string; models: string[]; placeholder: string; guide: Record<Lang, string[]> }> = {
    claude: {
      name: 'Claude (Anthropic)',
      icon: '🟠',
      models: ['claude-sonnet-4-6', 'claude-opus-4-6', 'claude-haiku-4-5-20251001'],
      placeholder: 'sk-ant-api03-...',
      guide: {
        ko: [
          '1. 아래 링크에서 로그인',
          '__link__https://platform.claude.com/dashboard',
          '2. API Keys 메뉴로 이동',
          '3. "Create Key" 버튼 클릭',
          '4. 생성된 키를 복사하여 아래에 붙여넣기',
        ],
        en: [
          '1. Sign in at the link below',
          '__link__https://platform.claude.com/dashboard',
          '2. Navigate to API Keys',
          '3. Click "Create Key" button',
          '4. Copy the key and paste it below',
        ],
      },
    },
    gpt: {
      name: 'GPT (OpenAI)',
      icon: '🟢',
      models: ['gpt-4o', 'gpt-4o-mini'],
      placeholder: 'sk-proj-...',
      guide: {
        ko: [
          '1. 아래 링크에서 로그인',
          '__link__https://platform.openai.com/api-keys',
          '2. "Create new secret key" 클릭',
          '3. 생성된 키를 복사하여 아래에 붙여넣기',
        ],
        en: [
          '1. Sign in at the link below',
          '__link__https://platform.openai.com/api-keys',
          '2. Click "Create new secret key"',
          '3. Copy the key and paste it below',
        ],
      },
    },
    gemini: {
      name: 'Gemini (Google)',
      icon: '🔵',
      models: ['gemini-2.5-pro', 'gemini-2.5-flash'],
      placeholder: 'AIza...',
      guide: {
        ko: [
          '1. 아래 링크에서 로그인',
          '__link__https://aistudio.google.com/apikey',
          '2. "Create API key" 버튼 클릭',
          '3. 생성된 키를 복사하여 아래에 붙여넣기',
        ],
        en: [
          '1. Sign in at the link below',
          '__link__https://aistudio.google.com/apikey',
          '2. Click "Create API key" button',
          '3. Copy the key and paste it below',
        ],
      },
    },
  }

  // Mode toggle button (top-right)
  const ModeToggle = () => (
    <button
      onClick={() => switchMode(aiMode === 'tokamak' ? 'custom' : 'tokamak')}
      className="text-[11px] px-3 py-1.5 rounded-lg border border-[var(--color-border)] hover:bg-[var(--color-border)] transition-colors cursor-pointer text-[var(--color-text-secondary)] whitespace-nowrap"
    >
      {aiMode === 'tokamak' ? t('chat.switchToMyAi', lang) : t('chat.switchToTokamak', lang)}
    </button>
  )

  // Custom mode: API key setup screen
  if (aiMode === 'custom' && !hasKey) {
    const prov = selectedProvider ? providers[selectedProvider] : null

    return (
      <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
        <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)] flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <div className="w-9 h-9 rounded-full bg-[var(--color-accent)] flex items-center justify-center text-sm">🤖</div>
            <div>
              <div className="text-sm font-semibold">{t('chat.title', lang)}</div>
              <div className="text-[11px] text-[var(--color-text-secondary)]">{t('chat.myAi', lang)} · {t('chat.notConnected', lang)}</div>
            </div>
          </div>
          <ModeToggle />
        </div>

        <div className="flex-1 overflow-y-auto p-4">
          <div className="max-w-sm mx-auto space-y-4">
            <div className="text-center space-y-2 pt-4">
              <div className="w-14 h-14 rounded-2xl bg-[var(--color-accent)] flex items-center justify-center text-xl mx-auto">🤖</div>
              <h2 className="text-base font-semibold">{t('chat.setupTitle', lang)}</h2>
              <p className="text-[12px] text-[var(--color-text-secondary)]">{t('chat.setupDesc', lang)}</p>
            </div>

            {/* Provider Selection */}
            <div className="space-y-2">
              <h3 className="text-[12px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider">
                {t('chat.step1Provider', lang)}
              </h3>
              <div className="space-y-2">
                {Object.entries(providers).map(([key, p]) => (
                  <button
                    key={key}
                    onClick={() => { setSelectedProvider(key); setSelectedModel(p.models[0]); setSetupError(''); setFetchedModels([]) }}
                    className={`w-full flex items-center gap-3 px-4 py-3 rounded-xl border transition-colors cursor-pointer text-left ${
                      selectedProvider === key
                        ? 'bg-[var(--color-accent)] text-[var(--color-accent-text)] border-[var(--color-accent)]'
                        : 'bg-[var(--color-bg-sidebar)] border-[var(--color-border)] hover:bg-[var(--color-border)]'
                    }`}
                  >
                    <span className="text-lg">{p.icon}</span>
                    <div>
                      <div className="text-[13px] font-medium">{p.name}</div>
                      <div className={`text-[11px] ${selectedProvider === key ? 'opacity-80' : 'text-[var(--color-text-secondary)]'}`}>
                        {p.models[0]}
                      </div>
                    </div>
                    {selectedProvider === key && (
                      <svg className="ml-auto w-5 h-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                        <polyline points="20 6 9 17 4 12"/>
                      </svg>
                    )}
                  </button>
                ))}
              </div>
            </div>

            {/* Guide & Key Input */}
            {prov && (
              <div className="space-y-3 animate-[fadeIn_0.2s_ease-in]">
                <div className="space-y-2">
                  <h3 className="text-[12px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider">
                    {t('chat.step2Guide', lang)}
                  </h3>
                  <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 border border-[var(--color-border)] space-y-1.5">
                    {prov.guide[lang].map((step, i) =>
                      step.startsWith('__link__') ? (
                        <p key={i} className="pl-4">
                          <button
                            onClick={() => open(step.replace('__link__', ''))}
                            className="text-[12px] text-blue-600 dark:text-blue-400 underline cursor-pointer hover:opacity-80"
                          >
                            {step.replace('__link__', '')}
                          </button>
                        </p>
                      ) : (
                        <p key={i} className="text-[12px] text-[var(--color-text-primary)] leading-relaxed">{step}</p>
                      )
                    )}
                  </div>
                </div>

                <div>
                  <h3 className="text-[12px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider mb-2">
                    {t('chat.step3Key', lang)}
                  </h3>
                  <div className="flex gap-2">
                    <input
                      type="password"
                      value={keyInput}
                      onChange={e => setKeyInput(e.target.value)}
                      placeholder={prov.placeholder}
                      className="flex-1 bg-[var(--color-bg-sidebar)] rounded-xl px-4 py-3 text-[13px] outline-none border border-[var(--color-border)] placeholder-[var(--color-text-secondary)]"
                      onKeyDown={e => { if (e.key === 'Enter' && selectedProvider !== 'claude') fetchModels(selectedProvider!, keyInput) }}
                    />
                    {selectedProvider !== 'claude' && (
                      <button
                        onClick={() => fetchModels(selectedProvider!, keyInput)}
                        disabled={!keyInput.trim() || fetchingModels}
                        className="px-4 py-3 rounded-xl text-[12px] font-medium bg-[var(--color-bg-sidebar)] border border-[var(--color-border)] hover:bg-[var(--color-border)] disabled:opacity-40 cursor-pointer whitespace-nowrap"
                      >
                        {fetchingModels ? '...' : t('chat.fetchModels', lang)}
                      </button>
                    )}
                  </div>
                </div>

                <div>
                  <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">
                    {t('settings.model', lang)}
                    {fetchedModels.length > 0 && <span className="ml-1 text-[var(--color-success)]">({fetchedModels.length})</span>}
                  </label>
                  <select
                    value={selectedModel}
                    onChange={e => setSelectedModel(e.target.value)}
                    className="w-full bg-[var(--color-bg-sidebar)] rounded-xl px-4 py-2.5 text-[13px] outline-none border border-[var(--color-border)]"
                  >
                    {(fetchedModels.length > 0 ? fetchedModels : prov.models).map(m => (
                      <option key={m} value={m}>{m}</option>
                    ))}
                  </select>
                </div>

                <button
                  onClick={saveApiKey}
                  disabled={!keyInput.trim() || savingKey || !selectedModel}
                  className="w-full bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] disabled:opacity-40 rounded-xl px-4 py-3 text-[13px] font-medium transition-colors cursor-pointer text-[var(--color-accent-text)]"
                >
                  {savingKey ? t('chat.connecting', lang) : t('chat.connect', lang)}
                </button>

                {setupError && (
                  <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-xl p-3">
                    <p className="text-[12px] text-red-600 dark:text-red-400 whitespace-pre-wrap">{setupError}</p>
                  </div>
                )}

                <p className="text-[10px] text-[var(--color-text-secondary)] text-center">
                  {t('chat.keySecure', lang)}
                </p>
              </div>
            )}
          </div>
        </div>
      </div>
    )
  }

  // Chat screen (both Tokamak AI and Custom connected)
  const tokamakLoggedIn = aiMode === 'tokamak' && platformUser
  const headerSubtitle = aiMode === 'tokamak'
    ? (platformUser
        ? `${platformUser.email}`
        : (lang === 'ko' ? '로그인 필요' : 'Login required'))
    : `${config?.provider === 'claude' ? 'Claude' : config?.provider} · ${config?.model}`

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      {/* Header */}
      <div className="px-4 py-3 border-b border-[var(--color-border)] flex items-center justify-between bg-[var(--color-bg-sidebar)]">
        <div className="flex items-center gap-2.5">
          <div className="w-9 h-9 rounded-full bg-[var(--color-accent)] flex items-center justify-center text-sm">🤖</div>
          <div>
            <div className="text-sm font-semibold">{t('chat.title', lang)}</div>
            <div className={`text-[11px] ${tokamakLoggedIn || aiMode === 'custom' ? 'text-[var(--color-success)]' : 'text-[var(--color-text-secondary)]'}`}>{headerSubtitle}</div>
          </div>
        </div>
        <div className="flex items-center gap-1.5">
          {aiMode === 'tokamak' && tokenUsage && (
            <span className="text-[10px] text-[var(--color-text-secondary)] mr-1">
              🔑 {tokenUsage.used.toLocaleString()}/{tokenUsage.limit.toLocaleString()}
            </span>
          )}
          {aiMode === 'custom' && !showDisconnect && (
            <button
              onClick={() => setShowDisconnect(true)}
              className="text-[11px] px-2 py-1 rounded-lg border border-[var(--color-border)] hover:bg-[var(--color-border)] transition-colors cursor-pointer text-[var(--color-text-secondary)] mr-1"
            >
              {t('chat.changeProvider', lang)}
            </button>
          )}
          {aiMode === 'custom' && showDisconnect && (
            <div className="flex items-center gap-1 mr-1">
              <button
                onClick={async () => {
                  try {
                    await invoke('disconnect_ai')
                    setHasKey(false)
                    setConfig(null)
                    setMessages([])
                    setSelectedProvider(null)
                    setShowDisconnect(false)
                  } catch {}
                }}
                className="text-[10px] px-2 py-1 rounded-lg bg-[var(--color-error)] text-white cursor-pointer"
              >
                {t('chat.disconnect', lang)}
              </button>
              <button
                onClick={() => setShowDisconnect(false)}
                className="text-[10px] px-2 py-1 rounded-lg border border-[var(--color-border)] hover:bg-[var(--color-border)] cursor-pointer text-[var(--color-text-secondary)]"
              >
                {lang === 'ko' ? '취소' : 'Cancel'}
              </button>
            </div>
          )}
          <ModeToggle />
        </div>
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-y-auto px-4 py-3 space-y-2.5 bg-[var(--color-bg-chat)]">
        {messages.map((msg, i) => {
          const { cleanText, actions } = msg.role === 'assistant'
            ? parseActions(msg.content)
            : { cleanText: msg.content, actions: [] }

          return (
            <div key={i} className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}>
              <div className="max-w-[80%] space-y-1.5">
                <div
                  className={`rounded-2xl px-3 py-2 text-[13px] whitespace-pre-wrap leading-relaxed shadow-sm ${
                    msg.role === 'user'
                      ? 'bg-[var(--color-bubble-user)] text-[var(--color-accent-text)] rounded-br-sm'
                      : 'bg-[var(--color-bubble-ai)] text-[var(--color-text-primary)] rounded-bl-sm'
                  }`}
                >
                  {cleanText.split('\n').map((line, li) =>
                    line.startsWith('__link__') ? (
                      <span key={li}>
                        {li > 0 && '\n'}
                        <a
                          href="#"
                          onClick={e => { e.preventDefault(); open(line.replace('__link__', '')) }}
                          className="text-[var(--color-accent)] underline cursor-pointer break-all"
                        >
                          {lang === 'ko' ? '🔗 로그인 페이지 열기' : '🔗 Open login page'}
                        </a>
                      </span>
                    ) : (
                      <span key={li}>{li > 0 && '\n'}{line}</span>
                    )
                  )}
                </div>
                {actions.length > 0 && (
                  <div className="flex flex-wrap gap-1.5">
                    {actions.map((action, j) => (
                      <button
                        key={j}
                        onClick={() => executeAction(action)}
                        className="inline-flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium bg-[var(--color-accent)] text-[var(--color-accent-text)] hover:bg-[var(--color-accent-hover)] transition-colors cursor-pointer shadow-sm"
                      >
                        {actionLabel(action, lang)}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </div>
          )
        })}
        {loading && (
          <div className="flex justify-start">
            <div className="bg-[var(--color-bubble-ai)] rounded-2xl rounded-bl-sm px-3 py-2 text-[13px] shadow-sm">
              <span className="animate-pulse text-[var(--color-text-secondary)]">{t('chat.thinking', lang)}</span>
            </div>
          </div>
        )}
        <div ref={messagesEndRef} />
      </div>

      {/* Input */}
      <div className="px-3 py-2.5 border-t border-[var(--color-border)] bg-[var(--color-bg-main)]">
        <div className="flex gap-2 items-end">
          <textarea
            value={input}
            onChange={e => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={t('chat.placeholder', lang)}
            rows={1}
            className="flex-1 bg-[var(--color-bg-sidebar)] rounded-xl px-3 py-2 text-[13px] outline-none resize-none max-h-24 placeholder-[var(--color-text-secondary)] border border-[var(--color-border)]"
          />
          <button
            onClick={sendMessage}
            disabled={loading || !input.trim()}
            className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] disabled:opacity-40 rounded-xl px-4 py-2 text-[13px] font-medium transition-colors cursor-pointer text-[var(--color-accent-text)]"
          >
            {t('chat.send', lang)}
          </button>
        </div>
      </div>
    </div>
  )
}
