import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLang } from '../App'

type DelegationLevel = 'monitor' | 'operate' | 'full'

interface DelegationOption {
  level: DelegationLevel
  labelKo: string
  labelEn: string
  descKo: string
  descEn: string
  needsWallet: boolean
}

const DELEGATION_LEVELS: DelegationOption[] = [
  {
    level: 'monitor', labelKo: '모니터링', labelEn: 'Monitoring',
    descKo: '감시 + 이상 감지 시 알림 발송 및 조치 제안. 운영자 승인 후 실행합니다.',
    descEn: 'Monitors, sends alerts, and suggests actions. Executes only after operator approval.',
    needsWallet: false,
  },
  {
    level: 'operate', labelKo: '자동 운영', labelEn: 'Auto-Operate',
    descKo: '서비스 시작/중지/재시작, 가스 파라미터 조정을 AI가 자동으로 수행합니다.',
    descEn: 'AI automatically starts/stops/restarts services and adjusts parameters.',
    needsWallet: false,
  },
  {
    level: 'full', labelKo: '전체 위임', labelEn: 'Full Delegation',
    descKo: '온체인 트랜잭션(배치 커밋, 브릿지 등)까지 AI가 자동 집행합니다.',
    descEn: 'AI executes on-chain transactions (batch commits, bridge ops, etc.).',
    needsWallet: true,
  },
]

interface PermissionCategory {
  id: string
  labelKo: string
  labelEn: string
  minLevel: DelegationLevel
  items: { id: string; labelKo: string; labelEn: string }[]
}

const PERMISSIONS: PermissionCategory[] = [
  {
    id: 'monitoring', labelKo: '감시 · 제안', labelEn: 'Monitor & Suggest', minLevel: 'monitor',
    items: [
      { id: 'health_check', labelKo: '서비스 헬스체크', labelEn: 'Service health check' },
      { id: 'chain_metrics', labelKo: '체인 지표 수집', labelEn: 'Chain metrics collection' },
      { id: 'log_analysis', labelKo: '에러 로그 분석', labelEn: 'Error log analysis' },
      { id: 'alert', labelKo: '텔레그램 알림 발송', labelEn: 'Telegram alert dispatch' },
      { id: 'suggest', labelKo: '이상 감지 시 조치 제안', labelEn: 'Suggest actions on anomalies' },
    ],
  },
  {
    id: 'infra', labelKo: '인프라 제어', labelEn: 'Infrastructure Control', minLevel: 'operate',
    items: [
      { id: 'restart_service', labelKo: '서비스 자동 재시작', labelEn: 'Auto-restart services' },
      { id: 'scale', labelKo: '리소스 스케일링', labelEn: 'Resource scaling' },
      { id: 'config_adjust', labelKo: '가스 파라미터 조정', labelEn: 'Gas parameter adjustment' },
    ],
  },
  {
    id: 'onchain', labelKo: '온체인 액션', labelEn: 'On-chain Actions', minLevel: 'full',
    items: [
      { id: 'batch_commit', labelKo: '배치 커밋 트랜잭션', labelEn: 'Batch commit transactions' },
      { id: 'proof_submit', labelKo: '증명 제출 트랜잭션', labelEn: 'Proof submission transactions' },
      { id: 'bridge_ops', labelKo: '브릿지 운영', labelEn: 'Bridge operations' },
    ],
  },
]

const LEVEL_ORDER: DelegationLevel[] = ['monitor', 'operate', 'full']

// Mock: recent AI actions
const MOCK_AI_ACTIONS = [
  { time: '2분 전', action: 'L2 Node 자동 재시작', type: 'operate' as const },
  { time: '15분 전', action: '블록 생산 지연 알림 발송', type: 'monitor' as const },
  { time: '1시간 전', action: '가스비 조정 제안 → 승인 대기', type: 'suggest' as const },
  { time: '3시간 전', action: 'Prover 헬스체크 이상 감지', type: 'monitor' as const },
]

const MOCK_AI_ACTIONS_EN = [
  { time: '2m ago', action: 'Auto-restarted L2 Node', type: 'operate' as const },
  { time: '15m ago', action: 'Sent block delay alert', type: 'monitor' as const },
  { time: '1h ago', action: 'Suggested gas adjustment → awaiting approval', type: 'suggest' as const },
  { time: '3h ago', action: 'Detected Prover health issue', type: 'monitor' as const },
]

export default function WalletView() {
  const { lang } = useLang()
  const ko = lang === 'ko'
  const [savedLevel, setSavedLevel] = useState<DelegationLevel>('monitor') // persisted level
  const [pendingFull, setPendingFull] = useState(false) // true when full is selected but not yet saved
  const [walletAddress, setWalletAddress] = useState('')
  const [walletInput, setWalletInput] = useState('')
  const [spendingLimit, setSpendingLimit] = useState('1.0')
  const [aiMode, setAiMode] = useState<string>('tokamak')
  const [hasOwnKey, setHasOwnKey] = useState(false)
  // Inline AI setup
  const [aiProvider, setAiProvider] = useState('claude')
  const [aiApiKey, setAiApiKey] = useState('')
  const [aiModel, setAiModel] = useState('claude-sonnet-4-6')
  const [aiSaving, setAiSaving] = useState(false)
  const [aiResult, setAiResult] = useState<{ ok: boolean; msg: string } | null>(null)

  // The display level: shows 'full' when pending, otherwise savedLevel
  const level = pendingFull ? 'full' as DelegationLevel : savedLevel

  const loadAiStatus = () => {
    invoke<{ mode: string }>('get_ai_mode').then(m => setAiMode(m.mode ?? 'tokamak')).catch(() => {})
    invoke<boolean>('has_ai_key').then(ok => setHasOwnKey(ok)).catch(() => {})
  }

  useEffect(() => { loadAiStatus() }, [])

  const canFullDelegate = aiMode !== 'tokamak' && hasOwnKey

  const handleAiSave = async () => {
    if (!aiApiKey.trim()) return
    setAiSaving(true); setAiResult(null)
    try {
      await invoke('save_ai_config', { provider: aiProvider, apiKey: aiApiKey.trim(), model: aiModel })
      await invoke<string>('test_ai_connection')
      setAiResult({ ok: true, msg: ko ? 'AI 연결 성공!' : 'AI connected!' })
      setAiApiKey('')
      loadAiStatus()
    } catch (e) {
      setAiResult({ ok: false, msg: `${e}` })
    } finally { setAiSaving(false) }
  }

  const AI_PROVIDERS = [
    { value: 'claude', label: 'Claude (Anthropic)' },
    { value: 'openai', label: 'OpenAI (GPT)' },
    { value: 'custom', label: ko ? '커스텀 URL' : 'Custom URL' },
  ]

  const AI_MODELS: Record<string, { value: string; label: string }[]> = {
    claude: [
      { value: 'claude-sonnet-4-6', label: 'Claude Sonnet 4.6' },
      { value: 'claude-opus-4-6', label: 'Claude Opus 4.6' },
      { value: 'claude-haiku-4-5-20251001', label: 'Claude Haiku 4.5' },
    ],
    openai: [
      { value: 'gpt-4o', label: 'GPT-4o' },
      { value: 'gpt-4o-mini', label: 'GPT-4o Mini' },
    ],
    custom: [
      { value: 'default', label: 'Default' },
    ],
  }

  const selectedOption = DELEGATION_LEVELS.find(d => d.level === level)!
  const levelIdx = LEVEL_ORDER.indexOf(level)
  const actions = ko ? MOCK_AI_ACTIONS : MOCK_AI_ACTIONS_EN

  const isPermissionActive = (minLevel: DelegationLevel) => {
    return LEVEL_ORDER.indexOf(savedLevel) >= LEVEL_ORDER.indexOf(minLevel)
  }

  const isPermissionPending = (minLevel: DelegationLevel) => {
    return pendingFull && !isPermissionActive(minLevel) && LEVEL_ORDER.indexOf('full') >= LEVEL_ORDER.indexOf(minLevel)
  }

  const actionTypeColor = (type: string) => {
    if (type === 'monitor') return 'var(--color-success)'
    if (type === 'suggest') return '#3b82f6'
    if (type === 'operate') return 'var(--color-warning)'
    return 'var(--color-error)'
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      {/* Header */}
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
        <h1 className="text-base font-semibold">{ko ? 'AI 위임 관리' : 'AI Delegation'}</h1>
        <p className="text-[11px] text-[var(--color-text-secondary)] mt-0.5">
          {ko ? 'AI에게 앱체인 운영을 어디까지 맡길지 설정합니다' : 'Configure how much control to delegate to AI'}
        </p>
      </div>

      <div className="flex-1 overflow-y-auto p-3 space-y-3">

        {/* Delegation Level Selector */}
        <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
          <div className="pb-1">
            <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">
              {ko ? '위임 수준' : 'Delegation Level'}
            </span>
          </div>

          {/* Level bar */}
          <div className="flex items-center gap-1 mt-2 mb-3">
            {DELEGATION_LEVELS.map((opt, i) => {
              const handleClick = () => {
                if (opt.level === 'full') {
                  // Open setup flow — don't save yet
                  setPendingFull(true)
                } else {
                  // Monitor/Operate save immediately
                  setSavedLevel(opt.level)
                  setPendingFull(false)
                }
              }
              return (
                <button
                  key={opt.level}
                  onClick={handleClick}
                  className={`flex-1 py-1.5 text-[10px] font-medium rounded-lg cursor-pointer transition-all ${
                    level === opt.level
                      ? pendingFull && opt.level === 'full'
                        ? 'bg-[#f59e0b] text-white shadow-sm' // amber for pending
                        : 'bg-[#3b82f6] text-white shadow-sm'
                      : i <= levelIdx
                        ? 'bg-[#3b82f6]/20 text-[#3b82f6]'
                        : 'bg-[var(--color-bg-main)] text-[var(--color-text-secondary)] border border-[var(--color-border)]'
                  }`}
                >
                  {ko ? opt.labelKo : opt.labelEn}
                </button>
              )
            })}
          </div>

          {/* Selected level description */}
          <div className={`bg-[var(--color-bg-main)] rounded-lg p-2.5 border ${pendingFull ? 'border-[#f59e0b]/50' : 'border-[var(--color-border)]'}`}>
            <div className="flex items-center gap-2">
              <span className="text-[12px] font-medium">{ko ? selectedOption.labelKo : selectedOption.labelEn}</span>
              {pendingFull && (
                <span className="text-[9px] bg-[#f59e0b]/20 text-[#f59e0b] px-1.5 py-0.5 rounded font-medium">
                  {ko ? '설정 중' : 'Pending'}
                </span>
              )}
              {savedLevel === 'full' && !pendingFull && (
                <span className="text-[9px] bg-[var(--color-success)]/20 text-[var(--color-success)] px-1.5 py-0.5 rounded font-medium">
                  {ko ? '활성' : 'Active'}
                </span>
              )}
            </div>
            <div className="text-[10px] text-[var(--color-text-secondary)] mt-0.5">
              {ko ? selectedOption.descKo : selectedOption.descEn}
            </div>
            {level !== 'full' && (
              <div className="mt-1.5">
                <span className="text-[9px] text-[var(--color-text-secondary)]">
                  {ko
                    ? 'Tokamak AI 또는 자체 AI 모두 사용 가능합니다.'
                    : 'Works with both Tokamak AI and your own AI.'}
                </span>
              </div>
            )}
          </div>
        </div>

        {/* Full Delegation Requirements (AI + Wallet in one flow) */}
        {(pendingFull || savedLevel === 'full') && (
          <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
            <div className="pb-1">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">
                {ko ? '전체 위임 필수 설정' : 'Full Delegation Requirements'}
              </span>
            </div>

            {/* Step 1: AI */}
            <div className="mt-1">
              <div className="flex items-center gap-2 mb-1.5">
                <div className={`w-5 h-5 rounded-full flex items-center justify-center text-[10px] font-bold flex-shrink-0 ${
                  canFullDelegate ? 'bg-[var(--color-success)] text-white' : 'bg-[#3b82f6] text-white'
                }`}>
                  {canFullDelegate ? '✓' : '1'}
                </div>
                <span className="text-[11px] font-medium">{ko ? '자체 AI 연결' : 'Connect Own AI'}</span>
                {canFullDelegate && (
                  <span className="text-[9px] text-[var(--color-success)] ml-auto">{aiMode}</span>
                )}
              </div>
              {!canFullDelegate ? (
                <div className="ml-7 space-y-1.5">
                  <div className="text-[9px] text-[var(--color-text-secondary)] mb-1.5">
                    {ko ? '자산을 다루므로 공유 AI(Tokamak AI)는 사용할 수 없습니다.' : 'Shared AI cannot be used for asset operations.'}
                  </div>
                  <select value={aiProvider} onChange={e => { setAiProvider(e.target.value); setAiResult(null) }}
                    className="w-full bg-[var(--color-bg-main)] rounded-lg px-2.5 py-1.5 text-[11px] outline-none border border-[var(--color-border)] cursor-pointer">
                    {AI_PROVIDERS.map(p => <option key={p.value} value={p.value}>{p.label}</option>)}
                  </select>
                  <input type="password" value={aiApiKey} onChange={e => { setAiApiKey(e.target.value); setAiResult(null) }}
                    placeholder={ko ? 'API 키 입력' : 'Enter API key'}
                    className="w-full bg-[var(--color-bg-main)] rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none border border-[var(--color-border)]" />
                  <select value={aiModel} onChange={e => setAiModel(e.target.value)}
                    className="w-full bg-[var(--color-bg-main)] rounded-lg px-2.5 py-1.5 text-[11px] outline-none border border-[var(--color-border)] cursor-pointer">
                    {(AI_MODELS[aiProvider] || []).map(m => <option key={m.value} value={m.value}>{m.label}</option>)}
                  </select>
                  <button onClick={handleAiSave} disabled={aiSaving || !aiApiKey.trim()}
                    className="w-full bg-[#3b82f6] text-white text-[11px] font-medium py-1.5 rounded-lg hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-40">
                    {aiSaving ? (ko ? '연결 확인 중...' : 'Testing...') : (ko ? 'AI 연결' : 'Connect AI')}
                  </button>
                  {aiResult && (
                    <p className={`text-[10px] ${aiResult.ok ? 'text-[var(--color-success)]' : 'text-[var(--color-error)]'}`}>{aiResult.msg}</p>
                  )}
                </div>
              ) : (
                <div className="ml-7 text-[9px] text-[var(--color-text-secondary)]">
                  {ko ? '자체 AI가 연결되어 있습니다.' : 'Own AI is connected.'}
                </div>
              )}
            </div>

            {/* Divider */}
            <div className="border-t border-[var(--color-border)] my-2.5" />

            {/* Step 2: Wallet */}
            <div>
              <div className="flex items-center gap-2 mb-1.5">
                <div className={`w-5 h-5 rounded-full flex items-center justify-center text-[10px] font-bold flex-shrink-0 ${
                  walletAddress ? 'bg-[var(--color-success)] text-white' : canFullDelegate ? 'bg-[#3b82f6] text-white' : 'bg-[var(--color-border)] text-[var(--color-text-secondary)]'
                }`}>
                  {walletAddress ? '✓' : '2'}
                </div>
                <span className="text-[11px] font-medium">{ko ? '운영 지갑 등록' : 'Register Wallet'}</span>
                {walletAddress && (
                  <code className="text-[9px] font-mono text-[var(--color-success)] ml-auto">{walletAddress}</code>
                )}
              </div>

              {/* Security Notice */}
              <div className="ml-7 p-2 rounded-lg bg-[var(--color-success)]/10 border border-[var(--color-success)]/30 mb-1.5">
                <div className="flex items-center gap-1.5">
                  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="var(--color-success)" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>
                  </svg>
                  <span className="text-[9px] text-[var(--color-success)] font-medium">
                    {ko ? '하드웨어 보안 저장소(Keychain/TPM)에 암호화 저장' : 'Encrypted in hardware secure storage (Keychain/TPM)'}
                  </span>
                </div>
              </div>

              {!walletAddress ? (
                <div className="ml-7 space-y-1.5">
                  <input type="password" value={walletInput} onChange={e => setWalletInput(e.target.value)}
                    placeholder={ko ? '0x... 개인키를 입력하세요' : '0x... Enter private key'}
                    disabled={!canFullDelegate}
                    className="w-full bg-[var(--color-bg-main)] rounded-lg px-2.5 py-2 text-[11px] font-mono outline-none border border-[var(--color-border)] disabled:opacity-40" />
                  <button
                    onClick={() => { if (walletInput.trim()) { setWalletAddress('0x3d1e...885b'); setWalletInput('') } }}
                    disabled={!walletInput.trim() || !canFullDelegate}
                    className="w-full bg-[#3b82f6] text-white text-[11px] font-medium py-1.5 rounded-lg hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-40">
                    {ko ? '보안 저장소에 등록' : 'Save to Secure Storage'}
                  </button>
                  {!canFullDelegate && (
                    <p className="text-[9px] text-[var(--color-text-secondary)]">{ko ? '먼저 AI를 연결하세요.' : 'Connect AI first.'}</p>
                  )}
                </div>
              ) : (
                <div className="ml-7">
                  <div className="flex items-center justify-between bg-[var(--color-bg-main)] rounded-lg px-2.5 py-2 border border-[var(--color-border)]">
                    <div className="flex items-center gap-2">
                      <span className="w-2 h-2 rounded-full bg-[var(--color-success)]" />
                      <code className="text-[11px] font-mono">{walletAddress}</code>
                    </div>
                    <button onClick={() => { setWalletAddress(''); setWalletInput('') }}
                      className="text-[10px] text-[var(--color-error)] hover:underline cursor-pointer">
                      {ko ? '삭제' : 'Remove'}
                    </button>
                  </div>
                </div>
              )}
            </div>

            {/* Balance & Limits (shown when both are set) */}
            {canFullDelegate && walletAddress && (<>
              <div className="border-t border-[var(--color-border)] my-2.5" />
              <div className="grid grid-cols-3 gap-2">
                <div className="bg-[var(--color-bg-main)] rounded-lg p-2 border border-[var(--color-border)]">
                  <div className="text-[9px] text-[var(--color-text-secondary)]">{ko ? '잔액' : 'Balance'}</div>
                  <div className="text-[13px] font-semibold font-mono mt-0.5">2.45</div>
                  <div className="text-[9px] text-[var(--color-text-secondary)]">ETH</div>
                </div>
                <div className="bg-[var(--color-bg-main)] rounded-lg p-2 border border-[var(--color-border)]">
                  <div className="text-[9px] text-[var(--color-text-secondary)]">{ko ? '오늘 사용' : 'Today'}</div>
                  <div className="text-[13px] font-semibold font-mono mt-0.5">0.08</div>
                  <div className="text-[9px] text-[var(--color-text-secondary)]">ETH</div>
                </div>
                <div className="bg-[var(--color-bg-main)] rounded-lg p-2 border border-[var(--color-border)]">
                  <div className="text-[9px] text-[var(--color-text-secondary)]">{ko ? '이번 달' : 'Month'}</div>
                  <div className="text-[13px] font-semibold font-mono mt-0.5">0.32</div>
                  <div className="text-[9px] text-[var(--color-text-secondary)]">ETH</div>
                </div>
              </div>
              <div className="mt-2 space-y-2">
                <div className="flex items-center justify-between">
                  <div>
                    <div className="text-[11px] font-medium">{ko ? '일일 한도' : 'Daily Limit'}</div>
                  </div>
                  <div className="flex items-center gap-1">
                    <input type="text" value={spendingLimit} onChange={e => setSpendingLimit(e.target.value)}
                      className="w-14 bg-[var(--color-bg-main)] rounded-lg px-2 py-1 text-[11px] font-mono outline-none border border-[var(--color-border)] text-right" />
                    <span className="text-[10px] text-[var(--color-text-secondary)]">ETH</span>
                  </div>
                </div>
                <div className="flex items-center justify-between">
                  <div>
                    <div className="text-[11px] font-medium">{ko ? '건당 한도' : 'Per-Tx Limit'}</div>
                  </div>
                  <div className="flex items-center gap-1">
                    <input type="text" defaultValue="0.1"
                      className="w-14 bg-[var(--color-bg-main)] rounded-lg px-2 py-1 text-[11px] font-mono outline-none border border-[var(--color-border)] text-right" />
                    <span className="text-[10px] text-[var(--color-text-secondary)]">ETH</span>
                  </div>
                </div>
              </div>
            </>)}

            {/* Save / Cancel buttons */}
            <div className="mt-3 flex gap-2">
              {pendingFull && (
                <button
                  onClick={() => setPendingFull(false)}
                  className="flex-1 bg-[var(--color-bg-main)] text-[var(--color-text-secondary)] text-[11px] font-medium py-2 rounded-lg hover:opacity-80 transition-opacity cursor-pointer border border-[var(--color-border)]"
                >
                  {ko ? '취소' : 'Cancel'}
                </button>
              )}
              <button
                onClick={() => { setSavedLevel('full'); setPendingFull(false) }}
                disabled={!canFullDelegate || !walletAddress}
                className={`${pendingFull ? 'flex-1' : 'w-full'} bg-[var(--color-success)] text-white text-[11px] font-medium py-2 rounded-lg hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-30`}
              >
                {!canFullDelegate
                  ? (ko ? 'AI 설정을 먼저 완료하세요' : 'Complete AI setup first')
                  : !walletAddress
                    ? (ko ? '지갑을 먼저 등록하세요' : 'Register wallet first')
                    : (ko ? '전체 위임 저장' : 'Save Full Delegation')}
              </button>
            </div>
          </div>
        )}

        {/* Permissions */}
        <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
          <div className="pb-1">
            <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">
              {ko ? 'AI 권한' : 'AI Permissions'}
            </span>
          </div>
          <div className="mt-1 space-y-2">
            {PERMISSIONS.map(cat => {
              const active = isPermissionActive(cat.minLevel)
              const pending = isPermissionPending(cat.minLevel)
              return (
                <div key={cat.id}>
                  <div className="flex items-center gap-1.5 mb-1">
                    <span className={`text-[11px] font-medium ${active ? '' : pending ? 'text-[#f59e0b]' : 'text-[var(--color-text-secondary)] opacity-50'}`}>
                      {ko ? cat.labelKo : cat.labelEn}
                    </span>
                    {pending && (
                      <span className="text-[9px] text-[#f59e0b] bg-[#f59e0b]/10 px-1.5 py-0.5 rounded font-medium">
                        {ko ? '저장 시 활성화' : 'Active on save'}
                      </span>
                    )}
                    {!active && !pending && (
                      <span className="text-[9px] text-[var(--color-text-secondary)] bg-[var(--color-bg-main)] px-1.5 py-0.5 rounded border border-[var(--color-border)]">
                        {ko ? `${DELEGATION_LEVELS.find(d => d.level === cat.minLevel)?.labelKo} 이상` : `Requires ${DELEGATION_LEVELS.find(d => d.level === cat.minLevel)?.labelEn}`}
                      </span>
                    )}
                  </div>
                  <div className="space-y-0.5">
                    {cat.items.map(item => (
                      <div key={item.id} className={`flex items-center gap-2 px-2 py-1.5 rounded-lg ${active ? 'bg-[var(--color-bg-main)]' : pending ? 'bg-[#f59e0b]/5' : 'opacity-40'}`}>
                        <div className={`w-3.5 h-3.5 rounded border flex items-center justify-center flex-shrink-0 ${
                          active ? 'border-[#3b82f6] bg-[#3b82f6]' : pending ? 'border-[#f59e0b] bg-[#f59e0b]/20' : 'border-[var(--color-border)]'
                        }`}>
                          {active && (
                            <svg width="8" height="8" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
                              <polyline points="20 6 9 17 4 12"/>
                            </svg>
                          )}
                          {pending && (
                            <svg width="8" height="8" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
                              <circle cx="12" cy="12" r="3"/>
                            </svg>
                          )}
                        </div>
                        <span className={`text-[11px] ${pending ? 'text-[var(--color-text-secondary)]' : ''}`}>{ko ? item.labelKo : item.labelEn}</span>
                      </div>
                    ))}
                  </div>
                </div>
              )
            })}
          </div>
        </div>

        {/* Recent AI Actions */}
        <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
          <div className="pb-1">
            <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">
              {ko ? 'AI 활동 기록' : 'AI Activity Log'}
            </span>
          </div>
          <div className="mt-1 space-y-1">
            {actions.map((a, i) => (
              <div key={i} className="flex items-start gap-2 py-1.5 px-2 rounded-lg bg-[var(--color-bg-main)]">
                <span className="w-1.5 h-1.5 rounded-full mt-1.5 flex-shrink-0" style={{ backgroundColor: actionTypeColor(a.type) }} />
                <div className="flex-1 min-w-0">
                  <div className="text-[11px]">{a.action}</div>
                  <div className="text-[9px] text-[var(--color-text-secondary)]">{a.time}</div>
                </div>
              </div>
            ))}
          </div>
        </div>

      </div>
    </div>
  )
}
